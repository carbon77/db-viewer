use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PgSslMode {
    Disable,
    #[default]
    Prefer,
    Require,
}

impl PgSslMode {
    pub const ALL: [Self; 3] = [Self::Disable, Self::Prefer, Self::Require];
    pub fn label(self) -> &'static str {
        match self {
            Self::Disable => "Disable",
            Self::Prefer => "Prefer",
            Self::Require => "Require",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DatabaseTarget {
    SQLite {
        path: PathBuf,
    },
    PostgreSQL {
        host: String,
        port: u16,
        database: String,
        user: String,
        #[serde(skip, default)]
        password: String,
        ssl_mode: PgSslMode,
    },
}

impl DatabaseTarget {
    pub fn label(&self) -> String {
        match self {
            Self::SQLite { path } => path.display().to_string(),
            Self::PostgreSQL {
                host,
                port,
                database,
                user,
                ..
            } => format!("{user}@{host}:{port}/{database}"),
        }
    }
    pub fn without_password(&self) -> Self {
        let mut value = self.clone();
        if let Self::PostgreSQL { password, .. } = &mut value {
            password.clear();
        }
        value
    }
    pub fn same_connection(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::SQLite { path: a }, Self::SQLite { path: b }) => {
                normalized_path(a) == normalized_path(b)
            }
            (
                Self::PostgreSQL {
                    host: ah,
                    port: ap,
                    database: ad,
                    user: au,
                    ssl_mode: am,
                    ..
                },
                Self::PostgreSQL {
                    host: bh,
                    port: bp,
                    database: bd,
                    user: bu,
                    ssl_mode: bm,
                    ..
                },
            ) => ah.eq_ignore_ascii_case(bh) && ap == bp && ad == bd && au == bu && am == bm,
            _ => false,
        }
    }
    pub fn compact_label(&self) -> String {
        match self {
            Self::SQLite { path } => path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
                .into_owned(),
            Self::PostgreSQL { host, database, .. } => format!("{database}@{host}"),
        }
    }
}

fn normalized_path(path: &std::path::Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    std::fs::canonicalize(&absolute).unwrap_or(absolute)
}

#[cfg(test)]
mod target_tests {
    use super::*;
    #[test]
    fn postgresql_label_is_redacted() {
        let target = DatabaseTarget::PostgreSQL {
            host: "server".into(),
            port: 5432,
            database: "data".into(),
            user: "alice".into(),
            password: "top-secret".into(),
            ssl_mode: PgSslMode::Prefer,
        };
        assert_eq!(target.label(), "alice@server:5432/data");
        assert!(!target.label().contains("secret"));
    }
    #[test]
    fn postgresql_identity_ignores_password_and_host_case() {
        let a = DatabaseTarget::PostgreSQL {
            host: "SERVER".into(),
            port: 5432,
            database: "data".into(),
            user: "alice".into(),
            password: "one".into(),
            ssl_mode: PgSslMode::Prefer,
        };
        let mut b = a.clone();
        if let DatabaseTarget::PostgreSQL { host, password, .. } = &mut b {
            *host = "server".into();
            *password = "two".into();
        }
        assert!(a.same_connection(&b));
    }
    #[test]
    fn sqlite_identity_normalizes_relative_paths() {
        let a = DatabaseTarget::SQLite {
            path: PathBuf::from("sample.db"),
        };
        let b = DatabaseTarget::SQLite {
            path: std::env::current_dir().unwrap().join("sample.db"),
        };
        assert!(a.same_connection(&b));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Table,
    PartitionedTable,
    View,
    MaterializedView,
    Index,
    Trigger,
}

impl ObjectKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Table => "Tables",
            Self::PartitionedTable => "Partitioned tables",
            Self::View => "Views",
            Self::MaterializedView => "Materialized views",
            Self::Index => "Indexes",
            Self::Trigger => "Triggers",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SchemaObject {
    pub kind: ObjectKind,
    pub schema: String,
    pub name: String,
    pub table_name: String,
    pub sql: String,
}

impl SchemaObject {
    pub fn qualified_name(&self) -> String {
        if self.schema.is_empty() {
            self.name.clone()
        } else {
            format!("{}.{}", self.schema, self.name)
        }
    }
    pub fn is_data_source(&self) -> bool {
        matches!(
            self.kind,
            ObjectKind::Table
                | ObjectKind::PartitionedTable
                | ObjectKind::View
                | ObjectKind::MaterializedView
        )
    }
}

#[derive(Debug, Clone)]
pub struct ColumnInfo {
    pub cid: i64,
    pub name: String,
    pub declared_type: String,
    pub not_null: bool,
    pub default_value: Option<String>,
    pub primary_key: i64,
}

#[derive(Debug, Clone)]
pub struct ForeignKeyInfo {
    pub from: String,
    pub target_table: String,
    pub to: String,
    pub on_update: String,
    pub on_delete: String,
}

#[derive(Debug, Clone)]
pub struct ObjectDetails {
    pub columns: Vec<ColumnInfo>,
    pub foreign_keys: Vec<ForeignKeyInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CellValue {
    Null,
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl CellValue {
    pub fn display(&self) -> String {
        match self {
            Self::Null => "NULL".into(),
            Self::Integer(value) => value.to_string(),
            Self::Real(value) => value.to_string(),
            Self::Text(value) => value.clone(),
            Self::Blob(value) => format!("BLOB ({} bytes)", value.len()),
        }
    }

    pub fn csv(&self) -> String {
        match self {
            Self::Null => String::new(),
            Self::Blob(bytes) => bytes.iter().map(|b| format!("{b:02X}")).collect(),
            _ => self.display(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct RowSet {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<CellValue>>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterOperator {
    Equals,
    NotEquals,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Contains,
    IsNull,
    IsNotNull,
}

impl FilterOperator {
    pub const ALL: [Self; 9] = [
        Self::Equals,
        Self::NotEquals,
        Self::Less,
        Self::LessOrEqual,
        Self::Greater,
        Self::GreaterOrEqual,
        Self::Contains,
        Self::IsNull,
        Self::IsNotNull,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Equals => "=",
            Self::NotEquals => "!=",
            Self::Less => "<",
            Self::LessOrEqual => "<=",
            Self::Greater => ">",
            Self::GreaterOrEqual => ">=",
            Self::Contains => "contains",
            Self::IsNull => "is null",
            Self::IsNotNull => "is not null",
        }
    }

    pub fn needs_value(self) -> bool {
        !matches!(self, Self::IsNull | Self::IsNotNull)
    }
}

#[derive(Debug, Clone)]
pub struct FilterSpec {
    pub column: String,
    pub operator: FilterOperator,
    pub value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone)]
pub struct SortSpec {
    pub column: String,
    pub direction: SortDirection,
}
