//! Маршруты SSR-приложения.

pub mod category;
pub mod home;
pub mod product;
pub mod robots;
pub mod sitemap;

use axum::Router;
use axum::routing::get;
use tower_http::services::ServeDir;

use crate::AppState;

/// Собрать роутер с маршрутами страниц (без `fallback` — добавляется в `crate::router`).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(home::show))
        .route("/p/{slug}", get(product::show))
        .route("/c/{slug}", get(category::show))
        .route("/sitemap.xml", get(sitemap::show))
        .route("/robots.txt", get(robots::show))
        .nest_service(
            "/assets/brand",
            ServeDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/brand")),
        )
}
