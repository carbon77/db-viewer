use super::{DbError, Result, qualify_identifier, quote_identifier};
use crate::model::{
    CellValue, ColumnInfo, DatabaseTarget, FilterOperator, FilterSpec, ForeignKeyInfo,
    ObjectDetails, ObjectKind, RowSet, SchemaObject, SortDirection, SortSpec,
};
use fallible_iterator::FallibleIterator;
use native_tls::TlsConnector;
use postgres::types::{IsNull, ToSql, Type};
use postgres::{Client, Config, NoTls, Row};
use postgres_native_tls::MakeTlsConnector;
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

fn config(target: &DatabaseTarget) -> Result<(Config, crate::model::PgSslMode)> {
    let DatabaseTarget::PostgreSQL {
        host,
        port,
        database,
        user,
        password,
        ssl_mode,
    } = target
    else {
        unreachable!()
    };
    let mut cfg = Config::new();
    cfg.host(host)
        .port(*port)
        .dbname(database)
        .user(user)
        .connect_timeout(Duration::from_secs(10));
    if !password.is_empty() {
        cfg.password(password);
    }
    cfg.ssl_mode(match ssl_mode {
        crate::model::PgSslMode::Disable => postgres::config::SslMode::Disable,
        crate::model::PgSslMode::Prefer => postgres::config::SslMode::Prefer,
        crate::model::PgSslMode::Require => postgres::config::SslMode::Require,
    });
    Ok((cfg, *ssl_mode))
}

fn connect(target: &DatabaseTarget) -> Result<Client> {
    let (cfg, mode) = config(target)?;
    if mode == crate::model::PgSslMode::Disable {
        Ok(cfg.connect(NoTls)?)
    } else {
        Ok(cfg.connect(MakeTlsConnector::new(TlsConnector::builder().build()?))?)
    }
}

pub fn test_connection(target: &DatabaseTarget) -> Result<()> {
    connect(target).map(|_| ())
}

fn read_only(client: &mut Client) -> Result<()> {
    client.batch_execute("BEGIN READ ONLY")?;
    Ok(())
}

struct CancelGuard(Arc<AtomicBool>);
impl Drop for CancelGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}
fn cancellation(client: &Client, target: &DatabaseTarget, cancel: Arc<AtomicBool>) -> CancelGuard {
    let done = Arc::new(AtomicBool::new(false));
    let worker_done = done.clone();
    let token = client.cancel_token();
    let target = target.clone();
    std::thread::spawn(move || {
        while !worker_done.load(Ordering::Relaxed) && !cancel.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(25));
        }
        if cancel.load(Ordering::Relaxed)
            && !worker_done.load(Ordering::Relaxed)
            && let Ok((_, mode)) = config(&target)
        {
            if mode == crate::model::PgSslMode::Disable {
                let _ = token.cancel_query(NoTls);
            } else if let Ok(tls) = TlsConnector::builder().build() {
                let _ = token.cancel_query(MakeTlsConnector::new(tls));
            }
        }
    });
    CancelGuard(done)
}
fn text(row: &Row, index: usize) -> CellValue {
    row.get::<_, Option<String>>(index)
        .map(CellValue::Text)
        .unwrap_or(CellValue::Null)
}

#[derive(Debug)]
struct TextParameter(String);
impl ToSql for TextParameter {
    fn to_sql(
        &self,
        _ty: &Type,
        out: &mut bytes::BytesMut,
    ) -> std::result::Result<IsNull, Box<dyn std::error::Error + Send + Sync>> {
        out.extend_from_slice(self.0.as_bytes());
        Ok(IsNull::No)
    }
    fn accepts(_ty: &Type) -> bool {
        true
    }
    fn encode_format(&self, _ty: &Type) -> postgres::types::Format {
        postgres::types::Format::Text
    }
    postgres::types::to_sql_checked!();
}

pub fn load_schema(target: &DatabaseTarget) -> Result<Vec<SchemaObject>> {
    let mut client = connect(target)?;
    let sql = "SELECT n.nspname, c.relname, c.relkind::text, COALESCE(t.relname,''), CASE c.relkind WHEN 'v' THEN pg_get_viewdef(c.oid, true) WHEN 'm' THEN pg_get_viewdef(c.oid, true) WHEN 'i' THEN pg_get_indexdef(c.oid) ELSE '' END FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace LEFT JOIN pg_index ix ON ix.indexrelid=c.oid LEFT JOIN pg_class t ON t.oid=ix.indrelid WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_toast' AND c.relkind IN ('r','p','v','m','i') ORDER BY n.nspname,c.relname";
    let mut objects = Vec::new();
    for row in client.query(sql, &[])? {
        let kind = match row.get::<_, String>(2).as_str() {
            "r" => ObjectKind::Table,
            "p" => ObjectKind::PartitionedTable,
            "v" => ObjectKind::View,
            "m" => ObjectKind::MaterializedView,
            _ => ObjectKind::Index,
        };
        let schema: String = row.get(0);
        let name: String = row.get(1);
        let mut definition: String = row.get(4);
        if matches!(kind, ObjectKind::Table | ObjectKind::PartitionedTable) {
            definition="PostgreSQL does not retain a canonical CREATE TABLE statement. See Structure for columns and keys.".into();
        }
        objects.push(SchemaObject {
            kind,
            schema,
            name,
            table_name: row.get(3),
            sql: definition,
        });
    }
    for row in client.query("SELECT n.nspname,t.tgname,c.relname,pg_get_triggerdef(t.oid,true) FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE NOT t.tgisinternal AND n.nspname NOT IN ('pg_catalog','information_schema') AND n.nspname !~ '^pg_toast' ORDER BY n.nspname,t.tgname", &[])? {
        objects.push(SchemaObject { kind:ObjectKind::Trigger, schema:row.get(0), name:row.get(1), table_name:row.get(2), sql:row.get(3) });
    }
    Ok(objects)
}

pub fn load_details(target: &DatabaseTarget, object: &SchemaObject) -> Result<ObjectDetails> {
    let mut client = connect(target)?;
    let columns=client.query("SELECT (a.attnum-1)::bigint,a.attname,format_type(a.atttypid,a.atttypmod),a.attnotnull,pg_get_expr(d.adbin,d.adrelid),COALESCE((SELECT key.ordinality FROM unnest(i.indkey) WITH ORDINALITY AS key(attnum,ordinality) WHERE key.attnum=a.attnum),0)::bigint FROM pg_attribute a JOIN pg_class c ON c.oid=a.attrelid JOIN pg_namespace n ON n.oid=c.relnamespace LEFT JOIN pg_attrdef d ON d.adrelid=c.oid AND d.adnum=a.attnum LEFT JOIN pg_index i ON i.indrelid=c.oid AND i.indisprimary WHERE n.nspname=$1 AND c.relname=$2 AND a.attnum>0 AND NOT a.attisdropped ORDER BY a.attnum", &[&object.schema,&object.name])?.into_iter().map(|r| ColumnInfo { cid:r.get(0), name:r.get(1), declared_type:r.get(2), not_null:r.get(3), default_value:r.get(4), primary_key:r.get(5) }).collect();
    let foreign_keys=client.query("SELECT sa.attname,tn.nspname||'.'||tc.relname,ta.attname,CASE c.confupdtype WHEN 'c' THEN 'CASCADE' WHEN 'n' THEN 'SET NULL' WHEN 'd' THEN 'SET DEFAULT' WHEN 'r' THEN 'RESTRICT' ELSE 'NO ACTION' END,CASE c.confdeltype WHEN 'c' THEN 'CASCADE' WHEN 'n' THEN 'SET NULL' WHEN 'd' THEN 'SET DEFAULT' WHEN 'r' THEN 'RESTRICT' ELSE 'NO ACTION' END FROM pg_constraint c JOIN pg_class sc ON sc.oid=c.conrelid JOIN pg_namespace sn ON sn.oid=sc.relnamespace JOIN pg_class tc ON tc.oid=c.confrelid JOIN pg_namespace tn ON tn.oid=tc.relnamespace JOIN LATERAL unnest(c.conkey,c.confkey) k(s,t) ON true JOIN pg_attribute sa ON sa.attrelid=sc.oid AND sa.attnum=k.s JOIN pg_attribute ta ON ta.attrelid=tc.oid AND ta.attnum=k.t WHERE c.contype='f' AND sn.nspname=$1 AND sc.relname=$2", &[&object.schema,&object.name])?.into_iter().map(|r| ForeignKeyInfo { from:r.get(0), target_table:r.get(1), to:r.get(2), on_update:r.get(3), on_delete:r.get(4) }).collect();
    Ok(ObjectDetails {
        columns,
        foreign_keys,
    })
}

pub(crate) fn where_clause(filters: &[FilterSpec]) -> (String, Vec<String>) {
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    for f in filters {
        let c = quote_identifier(&f.column);
        let n = values.len() + 1;
        let clause = match f.operator {
            FilterOperator::Equals => format!("{c} = ${n}"),
            FilterOperator::NotEquals => format!("{c} != ${n}"),
            FilterOperator::Less => format!("{c} < ${n}"),
            FilterOperator::LessOrEqual => format!("{c} <= ${n}"),
            FilterOperator::Greater => format!("{c} > ${n}"),
            FilterOperator::GreaterOrEqual => format!("{c} >= ${n}"),
            FilterOperator::Contains => format!("{c}::text LIKE '%' || ${n} || '%'"),
            FilterOperator::IsNull => format!("{c} IS NULL"),
            FilterOperator::IsNotNull => format!("{c} IS NOT NULL"),
        };
        clauses.push(clause);
        if f.operator.needs_value() {
            values.push(f.value.clone())
        }
    }
    (
        if clauses.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", clauses.join(" AND "))
        },
        values,
    )
}
fn select_columns(details: &ObjectDetails) -> String {
    details
        .columns
        .iter()
        .map(|c| {
            format!(
                "{}::text AS {}",
                quote_identifier(&c.name),
                quote_identifier(&c.name)
            )
        })
        .collect::<Vec<_>>()
        .join(",")
}
fn params(values: &[TextParameter]) -> Vec<&(dyn postgres::types::ToSql + Sync)> {
    values
        .iter()
        .map(|v| v as &(dyn postgres::types::ToSql + Sync))
        .collect()
}
fn rows(columns: Vec<String>, source: Vec<Row>, cap: Option<usize>) -> RowSet {
    let truncated = cap.is_some_and(|c| source.len() > c);
    let take = cap.unwrap_or(usize::MAX);
    RowSet {
        columns,
        rows: source
            .into_iter()
            .take(take)
            .map(|r| (0..r.len()).map(|i| text(&r, i)).collect())
            .collect(),
        truncated,
    }
}

pub fn load_page(
    target: &DatabaseTarget,
    object: &SchemaObject,
    filters: &[FilterSpec],
    sort: Option<&SortSpec>,
    offset: usize,
    limit: usize,
) -> Result<(RowSet, u64)> {
    let mut client = connect(target)?;
    read_only(&mut client)?;
    let details = load_details(target, object)?;
    let (w, v) = where_clause(filters);
    let typed: Vec<_> = v.into_iter().map(TextParameter).collect();
    let p = params(&typed);
    let table = qualify_identifier(&object.schema, &object.name);
    let total: i64 = client
        .query_one(&format!("SELECT COUNT(*) FROM {table}{w}"), &p)?
        .get(0);
    let order = sort
        .map(|s| {
            format!(
                " ORDER BY {} {}",
                quote_identifier(&s.column),
                if s.direction == SortDirection::Asc {
                    "ASC"
                } else {
                    "DESC"
                }
            )
        })
        .unwrap_or_default();
    let sql = format!(
        "SELECT {} FROM {table}{w}{order} LIMIT {limit} OFFSET {offset}",
        select_columns(&details)
    );
    let statement = client.prepare(&sql)?;
    let names = statement
        .columns()
        .iter()
        .map(|c| c.name().to_owned())
        .collect();
    let data = client.query(&statement, &p)?;
    Ok((rows(names, data, None), total as u64))
}

fn query_sql(client: &mut Client, sql: &str, cap: Option<usize>) -> Result<RowSet> {
    let clean = sql.trim().trim_end_matches(';');
    let prepared = client.prepare(clean)?;
    if prepared.columns().is_empty() {
        return Err(DbError::NotReadOnly);
    }
    let select = prepared
        .columns()
        .iter()
        .map(|c| {
            format!(
                "q.{}::text AS {}",
                quote_identifier(c.name()),
                quote_identifier(c.name())
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let wrapped = format!(
        "SELECT {select} FROM ({clean}) q{}",
        cap.map(|c| format!(" LIMIT {}", c + 1)).unwrap_or_default()
    );
    let stmt = client.prepare(&wrapped)?;
    let names = stmt.columns().iter().map(|c| c.name().to_owned()).collect();
    let data = client.query(&stmt, &[])?;
    Ok(rows(names, data, cap))
}
pub fn execute_query(
    target: &DatabaseTarget,
    sql: &str,
    cap: usize,
    cancel: Arc<AtomicBool>,
) -> Result<RowSet> {
    let mut c = connect(target)?;
    read_only(&mut c)?;
    let _cancel_guard = cancellation(&c, target, cancel.clone());
    if cancel.load(Ordering::Relaxed) {
        return Err(DbError::NotReadOnly);
    }
    query_sql(&mut c, sql, Some(cap))
}
fn stream_csv(
    client: &mut Client,
    statement: &postgres::Statement,
    parameters: &[&(dyn ToSql + Sync)],
    destination: &Path,
) -> Result<u64> {
    let mut out = BufWriter::new(File::create(destination)?);
    out.write_all(&[0xEF, 0xBB, 0xBF])?;
    let mut w = csv::Writer::from_writer(out);
    w.write_record(statement.columns().iter().map(|c| c.name()))?;
    let mut count = 0;
    let mut rows = client.query_raw(statement, parameters.iter().copied())?;
    while let Some(row) = rows.next()? {
        w.write_record((0..row.len()).map(|index| text(&row, index).csv()))?;
        count += 1;
    }
    w.flush()?;
    Ok(count)
}
pub fn export_table(
    target: &DatabaseTarget,
    object: &SchemaObject,
    filters: &[FilterSpec],
    sort: Option<&SortSpec>,
    destination: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<u64> {
    let mut c = connect(target)?;
    read_only(&mut c)?;
    let _cancel_guard = cancellation(&c, target, cancel);
    let d = load_details(target, object)?;
    let (w, v) = where_clause(filters);
    let typed: Vec<_> = v.into_iter().map(TextParameter).collect();
    let p = params(&typed);
    let order = sort
        .map(|s| {
            format!(
                " ORDER BY {} {}",
                quote_identifier(&s.column),
                if s.direction == SortDirection::Asc {
                    "ASC"
                } else {
                    "DESC"
                }
            )
        })
        .unwrap_or_default();
    let stmt = c.prepare(&format!(
        "SELECT {} FROM {}{w}{order}",
        select_columns(&d),
        qualify_identifier(&object.schema, &object.name)
    ))?;
    stream_csv(&mut c, &stmt, &p, destination)
}
pub fn export_query(
    target: &DatabaseTarget,
    sql: &str,
    destination: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<u64> {
    let mut c = connect(target)?;
    read_only(&mut c)?;
    let _cancel_guard = cancellation(&c, target, cancel);
    let clean = sql.trim().trim_end_matches(';');
    let prepared = c.prepare(clean)?;
    if prepared.columns().is_empty() {
        return Err(DbError::NotReadOnly);
    }
    let select = prepared
        .columns()
        .iter()
        .map(|column| {
            format!(
                "q.{}::text AS {}",
                quote_identifier(column.name()),
                quote_identifier(column.name())
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let statement = c.prepare(&format!("SELECT {select} FROM ({clean}) q"))?;
    stream_csv(&mut c, &statement, &[], destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numbered_parameters() {
        let (f, v) = where_clause(&[
            FilterSpec {
                column: "a".into(),
                operator: FilterOperator::Equals,
                value: "1".into(),
            },
            FilterSpec {
                column: "b".into(),
                operator: FilterOperator::Contains,
                value: "x".into(),
            },
        ]);
        assert!(f.contains("$1"));
        assert!(f.contains("$2"));
        assert_eq!(v.len(), 2)
    }

    #[test]
    fn postgresql_integration() {
        let Ok(password) = std::env::var("POSTGRES_TEST_PASSWORD") else {
            return;
        };
        let target = DatabaseTarget::PostgreSQL {
            host: std::env::var("POSTGRES_TEST_HOST").unwrap_or_else(|_| "localhost".into()),
            port: std::env::var("POSTGRES_TEST_PORT")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(5432),
            database: "postgres".into(),
            user: "postgres".into(),
            password,
            ssl_mode: crate::model::PgSslMode::Disable,
        };
        let mut admin = connect(&target).unwrap();
        admin.batch_execute("DROP SCHEMA IF EXISTS dbv_a CASCADE; DROP SCHEMA IF EXISTS dbv_b CASCADE; CREATE SCHEMA dbv_a; CREATE SCHEMA dbv_b; CREATE TABLE dbv_a.items(id integer PRIMARY KEY, name text NOT NULL); CREATE TABLE dbv_b.items(id integer PRIMARY KEY, name text); INSERT INTO dbv_a.items VALUES (1,'Привет'),(2,'world'); CREATE VIEW dbv_a.names AS SELECT name FROM dbv_a.items; CREATE MATERIALIZED VIEW dbv_a.item_count AS SELECT count(*) AS count FROM dbv_a.items; CREATE INDEX items_name_idx ON dbv_a.items(name); CREATE FUNCTION dbv_a.noop() RETURNS trigger LANGUAGE plpgsql AS 'BEGIN RETURN NEW; END'; CREATE TRIGGER items_trigger BEFORE INSERT ON dbv_a.items FOR EACH ROW EXECUTE FUNCTION dbv_a.noop();").unwrap();
        let schema = load_schema(&target).unwrap();
        let copies: Vec<_> = schema.iter().filter(|o| o.name == "items").collect();
        assert_eq!(copies.len(), 2);
        assert!(
            schema
                .iter()
                .any(|o| o.kind == ObjectKind::MaterializedView)
        );
        assert!(schema.iter().any(|o| o.kind == ObjectKind::Trigger));
        let object = schema
            .iter()
            .find(|o| o.schema == "dbv_a" && o.name == "items")
            .unwrap();
        let details = load_details(&target, object).unwrap();
        assert_eq!(details.columns.len(), 2);
        assert_eq!(details.columns[0].primary_key, 1);
        let filter = [FilterSpec {
            column: "id".into(),
            operator: FilterOperator::Equals,
            value: "1".into(),
        }];
        let (page, total) = load_page(&target, object, &filter, None, 0, 10).unwrap();
        assert_eq!(total, 1);
        assert_eq!(page.rows[0][1].display(), "Привет");
        let query = execute_query(
            &target,
            "SELECT name FROM dbv_a.items ORDER BY id",
            10,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(query.rows.len(), 2);
        assert!(
            execute_query(
                &target,
                "INSERT INTO dbv_a.items VALUES (3, 'write') RETURNING id",
                10,
                Arc::new(AtomicBool::new(false))
            )
            .is_err()
        );
        let file = tempfile::NamedTempFile::new().unwrap();
        let count = export_table(
            &target,
            object,
            &[],
            None,
            file.path(),
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        assert_eq!(count, 2);
        assert!(
            std::fs::read_to_string(file.path())
                .unwrap()
                .contains("Привет")
        );
        admin
            .batch_execute("DROP SCHEMA dbv_a CASCADE; DROP SCHEMA dbv_b CASCADE;")
            .unwrap();
    }
}
