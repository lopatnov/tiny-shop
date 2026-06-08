//! Доменные типы таксономии каталога (T1a-3) — design-1a.md §2.1 + i18n (ADR O5).
//!
//! Схема — `migrations/catalog/0002_taxonomy.sql`. Канон названий хранится на `uk`
//! непосредственно в полях `name`/`value`; переводы на другие языки — в `translations`
//! (резолв см. [`crate::repository`]).

/// Поддерживаемые языки интерфейса (i18n названий каталога, ADR O5).
///
/// `uk` — язык канона (хранится прямо в `name`/`value`), `ru` — переопределение в `translations`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Uk,
    Ru,
}

impl Lang {
    pub fn as_str(self) -> &'static str {
        match self {
            Lang::Uk => "uk",
            Lang::Ru => "ru",
        }
    }
}

/// Категория каталога (узел дерева). `path` — materialized path вида `/electronics/phones`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub slug: String,
    pub path: String,
    pub position: i64,
}

/// Тип значения атрибута (типизированный EAV, design-1a.md §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    String,
    Number,
    Enum,
    Bool,
}

impl DataType {
    pub fn as_str(self) -> &'static str {
        match self {
            DataType::String => "string",
            DataType::Number => "number",
            DataType::Enum => "enum",
            DataType::Bool => "bool",
        }
    }

    /// Разбор значения колонки `data_type`. `None` — неизвестное значение (БД хранит
    /// каноничные строки под CHECK-ограничением, но парсер не должен паниковать).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "string" => Some(DataType::String),
            "number" => Some(DataType::Number),
            "enum" => Some(DataType::Enum),
            "bool" => Some(DataType::Bool),
            _ => None,
        }
    }
}

/// Атрибут категории.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribute {
    pub id: String,
    pub category_id: String,
    pub name: String,
    pub data_type: DataType,
    /// Технический юнит (`'GB'`, `'kg'`, ...) — НЕ переводится.
    pub unit: Option<String>,
    pub is_required: bool,
    pub position: i64,
}

/// Допустимое значение enum-атрибута.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeOption {
    pub id: String,
    pub attribute_id: String,
    pub value: String,
    pub position: i64,
}

/// Тип фильтра в UI (CHECK `filters.filter_type`, см. CLAUDE.md «Типы фильтров каталога»).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterType {
    CheckboxOr,
    EnumAnd,
    String,
    Number,
    RangePrice,
    RangeGeneric,
}

impl FilterType {
    pub fn as_str(self) -> &'static str {
        match self {
            FilterType::CheckboxOr => "checkbox_or",
            FilterType::EnumAnd => "enum_and",
            FilterType::String => "string",
            FilterType::Number => "number",
            FilterType::RangePrice => "range_price",
            FilterType::RangeGeneric => "range_generic",
        }
    }

    /// См. [`DataType::parse`] — та же логика терпимости к неожиданным строкам.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "checkbox_or" => Some(FilterType::CheckboxOr),
            "enum_and" => Some(FilterType::EnumAnd),
            "string" => Some(FilterType::String),
            "number" => Some(FilterType::Number),
            "range_price" => Some(FilterType::RangePrice),
            "range_generic" => Some(FilterType::RangeGeneric),
            _ => None,
        }
    }
}

/// Привязка атрибута к категории как фильтра.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    pub id: String,
    pub category_id: String,
    pub attribute_id: String,
    pub filter_type: FilterType,
    pub position: i64,
}
