//! `GET /p/{slug}` — страница товара (design-1a.md §6: `Product`+`Offer`+`BreadcrumbList`).

use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Response};
use catalog::Lang;
use maud::html;

use crate::AppState;
use crate::error::WebError;
use crate::jsonld::{self, BreadcrumbList, ListItem, Offer, Product, absolute_url, jsonld_script};
use crate::view::breadcrumb::breadcrumb_nav;
use crate::view::layout::page_shell;

/// Обработчик `GET /p/{slug}`.
pub async fn show(State(state): State<AppState>, Path(slug): Path<String>) -> Response {
    match render(&state, &slug).await {
        Ok(html) => Html(html).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn render(state: &AppState, slug: &str) -> Result<String, WebError> {
    let card = state
        .search
        .get_card_by_slug(slug)
        .await
        .map_err(|e| WebError::Internal(e.to_string()))?
        .ok_or(WebError::NotFound)?;

    // Хлебные крошки: "Головна" → (категорія, якщо є) → назва товару (поточна сторінка).
    let mut crumbs: Vec<(String, Option<String>)> =
        vec![("Головна".to_string(), Some("/".to_string()))];
    if let Some(category_id) = &card.category_id {
        let category = state
            .taxonomy
            .get_category(category_id, Lang::Uk)
            .await
            .map_err(|e| WebError::Internal(e.to_string()))?;
        if let Some(category) = category {
            crumbs.push((category.name, Some(format!("/c/{}", category.slug))));
        }
    }
    crumbs.push((card.title.clone(), None));

    let price = jsonld::format_price_minor(card.price_minor);

    let product_ld = Product {
        context: "https://schema.org",
        type_: "Product",
        name: &card.title,
        description: &card.description,
        image: card
            .thumb
            .as_deref()
            .map(|thumb| absolute_url(&state.base_url, thumb)),
        offers: Offer {
            type_: "Offer",
            price: price.clone(),
            price_currency: &card.currency,
            // Карточка уже отфильтрована по status='published' — товар доступний для замовлення.
            availability: "https://schema.org/InStock",
        },
    };

    let breadcrumb_ld = BreadcrumbList {
        context: "https://schema.org",
        type_: "BreadcrumbList",
        item_list_element: crumbs
            .iter()
            .enumerate()
            .map(|(i, (name, url))| {
                let path = url.clone().unwrap_or_else(|| format!("/p/{slug}"));
                ListItem {
                    type_: "ListItem",
                    position: (i + 1) as u32,
                    name: name.clone(),
                    item: absolute_url(&state.base_url, &path),
                }
            })
            .collect(),
    };

    let head_extra = html! {
        (jsonld_script(&product_ld))
        (jsonld_script(&breadcrumb_ld))
    };

    let main = html! {
        (breadcrumb_nav(&crumbs))
        h1 { (card.title) }
        @if let Some(thumb) = &card.thumb {
            img src=(thumb) alt=(card.title);
        }
        p {
            span { "Ціна: " }
            (price) " " (card.currency)
        }
        p { (card.description) }
    };

    Ok(page_shell(&card.title, head_extra, main).into_string())
}
