//! Маршруты SSR-приложения.

pub mod category;
pub mod product;

use axum::Router;
use axum::routing::get;

use crate::AppState;

/// Собрать роутер с маршрутами страниц (без `fallback` — добавляется в `crate::router`).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/p/{slug}", get(product::show))
        .route("/c/{slug}", get(category::show))
}
