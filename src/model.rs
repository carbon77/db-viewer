use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    Table,
    View,
    Index,
    Trigger,
}

impl ObjectKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Table => "Tables",
            Self::View => "Views",
            Self::Index => "Indexes",
            Self::Trigger => "Triggers",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SchemaObject {
    pub kind: ObjectKind,
    pub name: String,
    pub table_name: String,
    pub sql: String,
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
