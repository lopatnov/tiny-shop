//! Маршруты SSR-приложения.

pub mod cart;
pub mod category;
pub mod checkout;
pub mod home;
pub mod product;
pub mod robots;
pub mod sitemap;

use axum::Router;
use axum::routing::{get, post};
use tower_http::services::ServeDir;

use crate::AppState;

/// Собрать роутер с маршрутами страниц (без `fallback` — добавляется в `crate::router`).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(home::show))
        .route("/p/{slug}", get(product::show))
        .route("/c/{slug}", get(category::show))
        .route("/cart", get(cart::show))
        .route("/cart/add", post(cart::add))
        .route("/cart/update", post(cart::update))
        .route("/cart/remove", post(cart::remove))
        .route("/checkout", get(checkout::show).post(checkout::submit))
        .route("/checkout/done/{order_id}", get(checkout::done))
        .route("/sitemap.xml", get(sitemap::show))
        .route("/robots.txt", get(robots::show))
        .nest_service(
            "/assets/brand",
            ServeDir::new(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/brand")),
        )
}
