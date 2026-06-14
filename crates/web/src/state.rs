//! Общее состояние приложения для `axum::extract::State`.

use catalog::{SqliteCatalogSearch, TaxonomyRepo};
use orders::CartRepo;

/// Состояние, доступное всем обработчикам через `State<AppState>`.
#[derive(Clone)]
pub struct AppState {
    pub search: SqliteCatalogSearch,
    pub taxonomy: TaxonomyRepo,
    pub carts: CartRepo,
    /// Базовый URL сайта (для абсолютных ссылок в JSON-LD/sitemap), без хвостового `/`.
    pub base_url: String,
}
