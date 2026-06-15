//! `GET /` — головна сторінка (design-1a.md §6: вхід у категорії каталогу).

use axum::extract::State;
use axum::response::{Html, IntoResponse, Response};
use catalog::Lang;
use maud::html;

use crate::AppState;
use crate::error::WebError;
use crate::view::layout::page_shell;

/// Обработчик `GET /`.
pub async fn show(State(state): State<AppState>) -> Response {
    match render(&state).await {
        Ok(html) => Html(html).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn render(state: &AppState) -> Result<String, WebError> {
    let categories = state
        .taxonomy
        .list_categories_by_parent(None, Lang::Uk)
        .await
        .map_err(|e| WebError::Internal(e.to_string()))?;

    let main = html! {
        h1 { "Vuriy" }
        @if categories.is_empty() {
            p { "Категорії з'являться тут найближчим часом." }
        } @else {
            nav aria-label="Категорії" {
                div class="row g-3" {
                    @for category in &categories {
                        div class="col-12 col-sm-6 col-md-4 col-lg-3" {
                            a href=(format!("/c/{}", category.slug)) class="card h-100 text-decoration-none" {
                                div class="card-body" {
                                    h2 class="card-title h6 mb-0" { (category.name) }
                                }
                            }
                        }
                    }
                }
            }
        }
    };

    Ok(page_shell("Vuriy", html! {}, main).into_string())
}
