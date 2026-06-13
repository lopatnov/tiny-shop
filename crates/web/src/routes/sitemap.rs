//! `GET /sitemap.xml` — sitemap для індексації (design-1a.md §6).
//!
//! Містить `/`, усі категорії (повне дерево, BFS від коренів) і всі опубліковані товари
//! (сторінками через [`CatalogSearch::search`]).

use std::collections::VecDeque;

use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use catalog::{CatalogSearch, Lang, SearchQuery, Sort};
use shared::Pagination;

use crate::AppState;
use crate::error::WebError;
use crate::jsonld::absolute_url;

/// Обработчик `GET /sitemap.xml`.
pub async fn show(State(state): State<AppState>) -> Response {
    match render(&state).await {
        Ok(xml) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/xml")],
            xml,
        )
            .into_response(),
        Err(err) => err.into_response(),
    }
}

async fn render(state: &AppState) -> Result<String, WebError> {
    let mut locs = vec![absolute_url(&state.base_url, "/")];

    locs.extend(category_paths(state).await?);
    locs.extend(product_paths(state).await?);

    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for loc in &locs {
        xml.push_str("  <url><loc>");
        xml.push_str(&escape_xml(loc));
        xml.push_str("</loc></url>\n");
    }
    xml.push_str("</urlset>\n");

    Ok(xml)
}

/// Обхід усього дерева категорій (BFS від коренів) → абсолютні `/c/{slug}` URL.
async fn category_paths(state: &AppState) -> Result<Vec<String>, WebError> {
    let mut paths = Vec::new();
    let mut queue: VecDeque<Option<String>> = VecDeque::new();
    queue.push_back(None);

    while let Some(parent_id) = queue.pop_front() {
        let children = state
            .taxonomy
            .list_categories_by_parent(parent_id.as_deref(), Lang::Uk)
            .await
            .map_err(|e| WebError::Internal(e.to_string()))?;

        for category in children {
            paths.push(absolute_url(
                &state.base_url,
                &format!("/c/{}", category.slug),
            ));
            queue.push_back(Some(category.id));
        }
    }

    Ok(paths)
}

/// Усі опубліковані товари (сторінками через [`CatalogSearch::search`]) → абсолютні
/// `/p/{slug}` URL.
async fn product_paths(state: &AppState) -> Result<Vec<String>, WebError> {
    let mut paths = Vec::new();
    let limit = Pagination::MAX_LIMIT;
    let mut offset = 0u32;

    loop {
        let result = state
            .search
            .search(&SearchQuery {
                text: None,
                category_id: None,
                filters: vec![],
                sort: Sort::default(),
                page: Pagination { offset, limit },
            })
            .await
            .map_err(|e| WebError::Internal(e.to_string()))?;

        if result.items.is_empty() {
            break;
        }

        for item in &result.items {
            paths.push(absolute_url(&state.base_url, &format!("/p/{}", item.slug)));
        }

        offset = offset.saturating_add(limit);
        if offset as u64 >= result.total {
            break;
        }
    }

    Ok(paths)
}

/// Екранувати спецсимволи XML у тексті елемента `<loc>`.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
