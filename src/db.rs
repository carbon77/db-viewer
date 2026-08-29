use crate::model::{
    CellValue, ColumnInfo, FilterOperator, FilterSpec, ForeignKeyInfo, ObjectDetails, ObjectKind,
    RowSet, SchemaObject, SortDirection, SortSpec,
};
use rusqlite::{Connection, OpenFlags, params_from_iter, types::ValueRef};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("SQLite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("CSV error: {0}")]
    Csv(#[from] csv::Error),
    #[error("Only one read-only SQL statement can be executed")]
    NotReadOnly,
}

pub type Result<T> = std::result::Result<T, DbError>;

fn install_cancel_handler(conn: &Connection, cancel: Arc<AtomicBool>) {
    conn.progress_handler(1_000, Some(move || cancel.load(Ordering::Relaxed)));
}

fn connect(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    conn.pragma_update(None, "query_only", true)?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(conn)
}

pub fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

pub fn load_schema(path: &Path) -> Result<Vec<SchemaObject>> {
    let conn = connect(path)?;
    let mut statement = conn.prepare(
        "SELECT type, name, tbl_name, COALESCE(sql, '') FROM sqlite_schema \
         WHERE name NOT LIKE 'sqlite_%' AND type IN ('table','view','index','trigger') \
         ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'view' THEN 1 WHEN 'index' THEN 2 ELSE 3 END, name COLLATE NOCASE",
    )?;
    let rows = statement.query_map([], |row| {
        let kind = match row.get::<_, String>(0)?.as_str() {
            "table" => ObjectKind::Table,
            "view" => ObjectKind::View,
            "index" => ObjectKind::Index,
            _ => ObjectKind::Trigger,
        };
        Ok(SchemaObject {
            kind,
            name: row.get(1)?,
            table_name: row.get(2)?,
            sql: row.get(3)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

pub fn load_details(path: &Path, object_name: &str) -> Result<ObjectDetails> {
    let conn = connect(path)?;
    let quoted = quote_identifier(object_name);
    let mut statement = conn.prepare(&format!("PRAGMA table_info({quoted})"))?;
    let columns = statement
        .query_map([], |row| {
            Ok(ColumnInfo {
                cid: row.get(0)?,
                name: row.get(1)?,
                declared_type: row.get(2)?,
                not_null: row.get::<_, i64>(3)? != 0,
                default_value: row.get(4)?,
                primary_key: row.get(5)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut statement = conn.prepare(&format!("PRAGMA foreign_key_list({quoted})"))?;
    let foreign_keys = statement
        .query_map([], |row| {
            Ok(ForeignKeyInfo {
                target_table: row.get(2)?,
                from: row.get(3)?,
                to: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
                on_update: row.get(5)?,
                on_delete: row.get(6)?,
            })
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    Ok(ObjectDetails {
        columns,
        foreign_keys,
    })
}

fn where_clause(filters: &[FilterSpec]) -> (String, Vec<String>) {
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    for filter in filters {
        let column = quote_identifier(&filter.column);
        let clause = match filter.operator {
            FilterOperator::Equals => format!("{column} = ?"),
            FilterOperator::NotEquals => format!("{column} != ?"),
            FilterOperator::Less => format!("{column} < ?"),
            FilterOperator::LessOrEqual => format!("{column} <= ?"),
            FilterOperator::Greater => format!("{column} > ?"),
            FilterOperator::GreaterOrEqual => format!("{column} >= ?"),
            FilterOperator::Contains => format!("instr(CAST({column} AS TEXT), ?) > 0"),
            FilterOperator::IsNull => format!("{column} IS NULL"),
            FilterOperator::IsNotNull => format!("{column} IS NOT NULL"),
        };
        clauses.push(clause);
        if filter.operator.needs_value() {
            values.push(filter.value.clone());
        }
    }
    if clauses.is_empty() {
        (String::new(), values)
    } else {
        (format!(" WHERE {}", clauses.join(" AND ")), values)
    }
}

fn order_clause(sort: Option<&SortSpec>) -> String {
    sort.map(|sort| {
        format!(
            " ORDER BY {} {}",
            quote_identifier(&sort.column),
            if sort.direction == SortDirection::Asc {
                "ASC"
            } else {
                "DESC"
            },
        )
    })
    .unwrap_or_default()
}

fn read_rows(
    statement: &mut rusqlite::Statement<'_>,
    values: &[String],
    cap: Option<usize>,
) -> Result<RowSet> {
    let columns = statement
        .column_names()
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    let column_count = statement.column_count();
    let mut cursor = statement.query(params_from_iter(values))?;
    let mut result = Vec::new();
    let max = cap.map(|value| value + 1);
    while let Some(row) = cursor.next()? {
        if max.is_some_and(|max| result.len() >= max) {
            break;
        }
        let mut values = Vec::with_capacity(column_count);
        for index in 0..column_count {
            values.push(match row.get_ref(index)? {
                ValueRef::Null => CellValue::Null,
                ValueRef::Integer(value) => CellValue::Integer(value),
                ValueRef::Real(value) => CellValue::Real(value),
                ValueRef::Text(value) => {
                    CellValue::Text(String::from_utf8_lossy(value).into_owned())
                }
                ValueRef::Blob(value) => CellValue::Blob(value.to_vec()),
            });
        }
        result.push(values);
    }
    let truncated = cap.is_some_and(|cap| result.len() > cap);
    if let Some(cap) = cap {
        result.truncate(cap);
    }
    Ok(RowSet {
        columns,
        rows: result,
        truncated,
    })
}

pub fn load_page(
    path: &Path,
    object_name: &str,
    filters: &[FilterSpec],
    sort: Option<&SortSpec>,
    offset: usize,
    limit: usize,
) -> Result<(RowSet, u64)> {
    let conn = connect(path)?;
    let (where_sql, values) = where_clause(filters);
    let table = quote_identifier(object_name);
    let total: u64 = conn.query_row(
        &format!("SELECT COUNT(*) FROM {table}{where_sql}"),
        params_from_iter(&values),
        |row| row.get(0),
    )?;
    let sql = format!(
        "SELECT * FROM {table}{where_sql}{} LIMIT {limit} OFFSET {offset}",
        order_clause(sort),
    );
    let mut statement = conn.prepare(&sql)?;
    Ok((read_rows(&mut statement, &values, None)?, total))
}

pub fn execute_query(
    path: &Path,
    sql: &str,
    cap: usize,
    cancel: Arc<AtomicBool>,
) -> Result<RowSet> {
    let conn = connect(path)?;
    install_cancel_handler(&conn, cancel);
    let mut statement = conn.prepare(sql.trim())?;
    if !statement.readonly() || statement.column_count() == 0 {
        return Err(DbError::NotReadOnly);
    }
    read_rows(&mut statement, &[], Some(cap))
}

fn export_statement(
    statement: &mut rusqlite::Statement<'_>,
    values: &[String],
    destination: &Path,
) -> Result<u64> {
    let file = File::create(destination)?;
    let mut output = BufWriter::new(file);
    output.write_all(&[0xEF, 0xBB, 0xBF])?;
    let mut writer = csv::WriterBuilder::new().from_writer(output);
    writer.write_record(statement.column_names())?;
    let column_count = statement.column_count();
    let mut cursor = statement.query(params_from_iter(values))?;
    let mut count = 0;
    while let Some(row) = cursor.next()? {
        let mut record = Vec::with_capacity(column_count);
        for index in 0..column_count {
            let value = match row.get_ref(index)? {
                ValueRef::Null => CellValue::Null,
                ValueRef::Integer(value) => CellValue::Integer(value),
                ValueRef::Real(value) => CellValue::Real(value),
                ValueRef::Text(value) => {
                    CellValue::Text(String::from_utf8_lossy(value).into_owned())
                }
                ValueRef::Blob(value) => CellValue::Blob(value.to_vec()),
            };
            record.push(value.csv());
        }
        writer.write_record(record)?;
        count += 1;
    }
    writer.flush()?;
    Ok(count)
}

pub fn export_table(
    path: &Path,
    object_name: &str,
    filters: &[FilterSpec],
    sort: Option<&SortSpec>,
    destination: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<u64> {
    let conn = connect(path)?;
    install_cancel_handler(&conn, cancel);
    let (where_sql, values) = where_clause(filters);
    let sql = format!(
        "SELECT * FROM {}{where_sql}{}",
        quote_identifier(object_name),
        order_clause(sort)
    );
    let mut statement = conn.prepare(&sql)?;
    export_statement(&mut statement, &values, destination)
}

pub fn export_query(
    path: &Path,
    sql: &str,
    destination: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<u64> {
    let conn = connect(path)?;
    install_cancel_handler(&conn, cancel);
    let mut statement = conn.prepare(sql.trim())?;
    if !statement.readonly() || statement.column_count() == 0 {
        return Err(DbError::NotReadOnly);
    }
    export_statement(&mut statement, &[], destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_safely_quoted() {
        assert_eq!(quote_identifier("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn read_only_browsing_and_querying_work() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        let conn = Connection::open(temp.path()).unwrap();
        conn.execute_batch("CREATE TABLE items(id INTEGER PRIMARY KEY, name TEXT, payload BLOB); INSERT INTO items(name,payload) VALUES ('one', X'CAFE'), ('two', NULL); CREATE VIEW named AS SELECT name FROM items;").unwrap();
        drop(conn);

        let schema = load_schema(temp.path()).unwrap();
        assert!(
            schema
                .iter()
                .any(|object| object.name == "items" && object.kind == ObjectKind::Table)
        );
        let (page, total) = load_page(temp.path(), "items", &[], None, 0, 50).unwrap();
        assert_eq!(total, 2);
        assert_eq!(page.rows.len(), 2);
        assert_eq!(
            execute_query(
                temp.path(),
                "SELECT name FROM items",
                10,
                Arc::new(AtomicBool::new(false))
            )
            .unwrap()
            .rows
            .len(),
            2
        );
        assert!(
            execute_query(
                temp.path(),
                "DELETE FROM items",
                10,
                Arc::new(AtomicBool::new(false))
            )
            .is_err()
        );
    }
}
