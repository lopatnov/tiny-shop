//! `web` — SSR-фасад (Axum + maud), T1a-6.
//!
//! Семантический HTML без обязательной гидратации (design-1a.md §6): страницы каталога
//! рендерятся сервером, JSON-LD (`Product`/`Offer`/`BreadcrumbList`) встраивается в `<head>`.

pub mod error;
pub mod jsonld;
pub mod routes;
pub mod state;
pub mod view;

pub use state::AppState;

/// Собрать корневой роутер приложения.
pub fn router(state: AppState) -> axum::Router {
    routes::router()
        .with_state(state)
        .fallback(error::not_found)
}
