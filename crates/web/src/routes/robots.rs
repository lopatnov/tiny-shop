//! `GET /robots.txt` — дозвіл на індексацію + посилання на sitemap (design-1a.md §6).

use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};

use crate::AppState;
use crate::jsonld::absolute_url;

/// Обработчик `GET /robots.txt`.
pub async fn show(State(state): State<AppState>) -> Response {
    let sitemap = absolute_url(&state.base_url, "/sitemap.xml");
    let body = format!("User-agent: *\nAllow: /\n\nSitemap: {sitemap}\n");
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body).into_response()
}
