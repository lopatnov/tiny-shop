//! `catalog` — контекст каталога: таксономия, фильтры и порт поиска/фильтрации.
//!
//! В T1a-7 здесь — контракт `CatalogSearch` и его типы (design-1a.md §2.3, §4).
//! 1a-адаптер `SqliteCatalogSearch` (FTS5 + SQL по проекции) — задача T1a-5.
//! Таксономия/EAV-схема и проекция — T1a-3/T1a-5.

use std::future::Future;

use shared::Pagination;

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("search backend error: {0}")]
    Backend(String),
}

/// Условие фильтра (типы — из CLAUDE.md «Типы фильтров каталога»).
#[derive(Debug, Clone, PartialEq)]
pub enum FilterCond {
    /// Любой из выбранных (OR).
    CheckboxOr {
        attribute_id: String,
        values: Vec<String>,
    },
    /// Все выбранные присутствуют (AND).
    EnumAnd {
        attribute_id: String,
        values: Vec<String>,
    },
    /// Точное число.
    ///
    /// NOTE: `f64` соответствует `val_num REAL` из EAV (design-1a.md §2). Точное сравнение
    /// чисел с плавающей запятой ненадёжно — стратегия (сравнение с допуском / масштабированные
    /// целые / `Decimal`) фиксируется в catalog-core (T1a-3/T1a-5) вместе с `architect`. Для
    /// точных совпадений предпочтительны enum/checkbox-атрибуты, а не `Number`.
    Number { attribute_id: String, value: f64 },
    /// Диапазон по числовому атрибуту.
    RangeGeneric {
        attribute_id: String,
        min: Option<f64>,
        max: Option<f64>,
    },
    /// Диапазон цены (минорные единицы).
    RangePrice {
        min_minor: Option<i64>,
        max_minor: Option<i64>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sort {
    #[default]
    Relevance,
    PriceAsc,
    PriceDesc,
    Newest,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchQuery {
    pub text: Option<String>,
    pub category_id: Option<String>,
    pub filters: Vec<FilterCond>,
    pub sort: Sort,
    pub page: Pagination,
}

/// Карточка товара для листинга/поиска (из проекции каталога).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductCard {
    pub product_id: String,
    pub title: String,
    pub slug: String,
    pub price_minor: i64,
    pub currency: String,
    pub thumb: Option<String>,
}

/// Фасет: атрибут + (значение, число товаров). Точные счётчики — позже (Tantivy), см. §2.3.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Facet {
    pub attribute_id: String,
    pub options: Vec<(String, u64)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub items: Vec<ProductCard>,
    pub total: u64,
    pub facets: Vec<Facet>,
}

/// Денормализованный документ товара для индекса поиска (наполняется проекцией по событиям).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductDoc {
    pub product_id: String,
    pub category_id: String,
    pub title: String,
    pub description: String,
    pub price_minor: i64,
    pub currency: String,
    pub slug: String,
    /// Конкатенация индексируемых атрибутов для FTS.
    pub attrs_text: String,
}

/// Порт поиска/фильтрации каталога. Нативный async-fn-in-trait.
/// Адаптер сегодня — SQLite/FTS5; при росте — Tantivy за тем же трейтом.
pub trait CatalogSearch {
    fn search(
        &self,
        query: &SearchQuery,
    ) -> impl Future<Output = Result<SearchResult, SearchError>> + Send;

    /// Вставка/обновление документа в индексе (вызывается из проекции).
    fn upsert(&self, doc: &ProductDoc) -> impl Future<Output = Result<(), SearchError>> + Send;

    fn remove(&self, product_id: &str) -> impl Future<Output = Result<(), SearchError>> + Send;
}
