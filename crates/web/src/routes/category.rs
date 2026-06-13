//! `GET /c/{slug}` — страница листинга категории (design-1a.md §6: `ItemList`+`BreadcrumbList`).
//!
//! Фильтры (`?attr_...=`) — вне объёма этого чанка (T1a-6 chunk 3).

use axum::extract::{Path, Query, State};
use axum::response::{Html, IntoResponse, Response};
use catalog::{CatalogSearch, Lang, SearchQuery, Sort};
use maud::html;
use serde::Deserialize;
use shared::Pagination;

use crate::AppState;
use crate::error::WebError;
use crate::jsonld::{
    self, ItemList, ItemListElement, ItemProduct, absolute_url, breadcrumb_list_ld, jsonld_script,
};
use crate::view::breadcrumb::breadcrumb_nav;
use crate::view::layout::page_shell;

#[derive(Debug, Deserialize)]
pub struct ListingQuery {
    page: Option<u32>,
}

/// Обработчик `GET /c/{slug}`.
pub async fn show(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(query): Query<ListingQuery>,
) -> Response {
    match render(&state, &slug, query.page.unwrap_or(1).max(1)).await {
        Ok(html) => Html(html).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn render(state: &AppState, slug: &str, page: u32) -> Result<String, WebError> {
    let category = state
        .taxonomy
        .get_category_by_slug(slug, Lang::Uk)
        .await
        .map_err(|e| WebError::Internal(e.to_string()))?
        .ok_or(WebError::NotFound)?;

    // Хлебные крошки: "Головна" → (батьківська категорія, якщо є) → назва категорії (поточна).
    let mut crumbs: Vec<(String, Option<String>)> =
        vec![("Головна".to_string(), Some("/".to_string()))];
    if let Some(parent_id) = &category.parent_id {
        let parent = state
            .taxonomy
            .get_category(parent_id, Lang::Uk)
            .await
            .map_err(|e| WebError::Internal(e.to_string()))?;
        if let Some(parent) = parent {
            crumbs.push((parent.name, Some(format!("/c/{}", parent.slug))));
        }
    }
    crumbs.push((category.name.clone(), None));

    let pagination = Pagination::clamped(
        (page - 1) * Pagination::DEFAULT_LIMIT,
        Pagination::DEFAULT_LIMIT,
    );

    let result = state
        .search
        .search(&SearchQuery {
            text: None,
            category_id: Some(category.id.clone()),
            filters: vec![],
            sort: Sort::default(),
            page: pagination,
        })
        .await
        .map_err(|e| WebError::Internal(e.to_string()))?;

    let breadcrumb_ld = breadcrumb_list_ld(&state.base_url, &crumbs, &format!("/c/{slug}"));

    let item_list_ld = ItemList {
        context: "https://schema.org",
        type_: "ItemList",
        item_list_element: result
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| ItemListElement {
                type_: "ListItem",
                position: (i + 1) as u32,
                item: ItemProduct {
                    type_: "Product",
                    name: item.title.clone(),
                    url: absolute_url(&state.base_url, &format!("/p/{}", item.slug)),
                    image: item
                        .thumb
                        .as_deref()
                        .map(|thumb| absolute_url(&state.base_url, thumb)),
                },
            })
            .collect(),
    };

    let head_extra = html! {
        (jsonld_script(&breadcrumb_ld))
        @if !result.items.is_empty() {
            (jsonld_script(&item_list_ld))
        }
    };

    let has_prev = pagination.offset > 0;
    let has_next = (pagination.offset as u64 + result.items.len() as u64) < result.total;

    let main = html! {
        (breadcrumb_nav(&crumbs))
        h1 { (category.name) }
        @if result.items.is_empty() {
            p { "У цій категорії ще немає товарів." }
        } @else {
            ul {
                @for item in &result.items {
                    li {
                        a href=(format!("/p/{}", item.slug)) {
                            @if let Some(thumb) = &item.thumb {
                                img src=(thumb) alt=(item.title);
                            }
                            span { (item.title) }
                        }
                        p {
                            (jsonld::format_price_minor(item.price_minor)) " " (item.currency)
                        }
                    }
                }
            }
            nav aria-label="Сторінки" {
                @if has_prev {
                    a href=(format!("/c/{slug}?page={}", page - 1)) { "Попередня" }
                }
                @if has_next {
                    a href=(format!("/c/{slug}?page={}", page + 1)) { "Наступна" }
                }
            }
        }
    };

    Ok(page_shell(&category.name, head_extra, main).into_string())
}
