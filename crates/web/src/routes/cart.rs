//! Корзина (T1b-1): `GET /cart`, `POST /cart/add`, `POST /cart/update`, `POST /cart/remove`.
//!
//! Корзина анонимна — адресуется по cart-токену в cookie `cart` (см. [`crate::cart_cookie`]).
//! Снимок названия/цены товара в позициях корзины берётся из catalog-проекции
//! (`SqliteCatalogSearch::get_card_by_slug`) на момент добавления — best-effort для отображения;
//! источник истины подтверждается на checkout (Phase 1b chunk 2).

use axum::extract::{Form, State};
use axum::http::HeaderMap;
use axum::http::header::SET_COOKIE;
use axum::response::{Html, IntoResponse, Redirect, Response};
use maud::{Markup, html};
use orders::{Cart, CartItem, NewCartItem};
use serde::Deserialize;

use crate::AppState;
use crate::cart_cookie::{extract_cart_token, set_cart_cookie};
use crate::error::WebError;
use crate::jsonld;
use crate::view::layout::page_shell;

/// `GET /cart` — показать корзину. Без cookie / неизвестная корзина / пустая корзина —
/// дружелюбное сообщение "кошик порожній" (как пустой результат в `category.rs`). Ничего не
/// создаёт и не выставляет `Set-Cookie`.
pub async fn show(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match render(&state, &headers).await {
        Ok(html) => Html(html).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn render(state: &AppState, headers: &HeaderMap) -> Result<String, WebError> {
    let items = match find_cart(state, headers).await? {
        Some(cart) => state.carts.list_items(&cart.token_hash).await?,
        None => Vec::new(),
    };

    let main = if items.is_empty() {
        html! {
            h1 { "Кошик" }
            p { "Кошик порожній." }
            p { a href="/" { "До каталогу" } }
        }
    } else {
        html! {
            (cart_table(&items))
            p { a href="/checkout" class="btn btn-primary" { "Оформити замовлення" } }
        }
    };

    Ok(page_shell("Кошик", html! {}, main).into_string())
}

/// Таблица позиций корзины с формами зміни кількості/видалення та підсумком.
fn cart_table(items: &[CartItem]) -> Markup {
    // saturating_* — защита от переполнения i64 при экстремальных qty/price (MAJOR, review).
    let total_minor: i64 = items
        .iter()
        .map(|i| i.unit_price_minor.saturating_mul(i.qty))
        .fold(0i64, |acc, x| acc.saturating_add(x));
    // Усі позиції корзини T1b-1 — в одній валюті (мультивалютна корзина поза обсягом chunk'а).
    let currency = items.first().map(|i| i.currency.as_str()).unwrap_or("UAH");

    html! {
        h1 { "Кошик" }
        table class="table table-striped align-middle" {
            caption { "Товари в кошику" }
            thead {
                tr {
                    th scope="col" { "Товар" }
                    th scope="col" { "Кількість" }
                    th scope="col" { "Ціна за од." }
                    th scope="col" { "Сума" }
                    th scope="col" { "Дії" }
                }
            }
            tbody {
                @for item in items {
                    tr {
                        td { (item.title) }
                        td {
                            form method="post" action="/cart/update" class="input-group input-group-sm" {
                                input
                                    type="number"
                                    name="qty"
                                    value=(item.qty)
                                    min="0"
                                    max=(orders::MAX_QTY)
                                    aria-label={ "Кількість для " (item.title) }
                                    class="form-control";
                                input type="hidden" name="item_id" value=(item.id);
                                button type="submit" class="btn btn-outline-secondary btn-sm" { "Оновити" }
                            }
                        }
                        td { (jsonld::format_price_minor(item.unit_price_minor)) " " (item.currency) }
                        td { (jsonld::format_price_minor(item.unit_price_minor.saturating_mul(item.qty))) " " (item.currency) }
                        td {
                            form method="post" action="/cart/remove" {
                                input type="hidden" name="item_id" value=(item.id);
                                button type="submit" class="btn btn-outline-danger btn-sm" { "Видалити" }
                            }
                        }
                    }
                }
            }
            tfoot class="table-group-divider" {
                tr {
                    th scope="row" colspan="3" class="fw-bold" { "Разом" }
                    td colspan="2" class="fw-bold" { (jsonld::format_price_minor(total_minor)) " " (currency) }
                }
            }
        }
    }
}

// -----------------------------------------------------------------
// POST /cart/add
// -----------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AddForm {
    pub slug: String,
    #[serde(default = "default_qty")]
    pub qty: i64,
}

fn default_qty() -> i64 {
    1
}

/// `POST /cart/add` — добавить товар по slug в корзину (получить-или-создать), редирект на
/// `/cart`. Несуществующий slug → редирект 303 на `/p/{slug}` без изменений (товар не найден —
/// не паниковать).
pub async fn add(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<AddForm>,
) -> Response {
    match render_add(&state, &headers, &form).await {
        Ok(response) => response,
        Err(err) => err.into_response(),
    }
}

async fn render_add(
    state: &AppState,
    headers: &HeaderMap,
    form: &AddForm,
) -> Result<Response, WebError> {
    let card = state
        .search
        .get_card_by_slug(&form.slug)
        .await
        .map_err(|e| WebError::Internal(e.to_string()))?;
    let Some(card) = card else {
        return Ok(Redirect::to(&format!("/p/{}", form.slug)).into_response());
    };

    let (cart, new_token) = get_or_create_cart(state, headers).await?;

    // Корзина T1b-1 — однієї валюти (див. cart_table); змішування USD/UAH тощо зламало б
    // підсумок (сумування minor units різних валют). Перевіряємо ДО побудови `NewCartItem`,
    // яка забирає `card.currency` за значенням.
    let existing_items = state.carts.list_items(&cart.token_hash).await?;
    if let Some(existing) = existing_items.first()
        && existing.currency != card.currency
    {
        return Err(WebError::BadRequest(
            "Товар у іншій валюті не можна додати до цього кошика".to_string(),
        ));
    }

    let item = NewCartItem {
        product_id: card.id,
        variant_id: None,
        qty: form.qty,
        title: card.title,
        unit_price_minor: card.price_minor,
        currency: card.currency,
    };
    state.carts.add_item(&cart.token_hash, &item).await?;

    Ok(redirect_to_cart(state, new_token))
}

// -----------------------------------------------------------------
// POST /cart/update
// -----------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct UpdateForm {
    pub item_id: i64,
    pub qty: i64,
}

/// `POST /cart/update` — изменить количество позиции (`qty == 0` удаляет строку), редирект на
/// `/cart`. Нет cart-cookie / корзина не найдена → редирект без изменений (нечего обновлять).
pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<UpdateForm>,
) -> Response {
    match render_update(&state, &headers, &form).await {
        Ok(response) => response,
        Err(err) => err.into_response(),
    }
}

async fn render_update(
    state: &AppState,
    headers: &HeaderMap,
    form: &UpdateForm,
) -> Result<Response, WebError> {
    if let Some(cart) = find_cart(state, headers).await? {
        state
            .carts
            .update_qty(&cart.token_hash, form.item_id, form.qty)
            .await?;
    }
    Ok(redirect_to_cart(state, None))
}

// -----------------------------------------------------------------
// POST /cart/remove
// -----------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct RemoveForm {
    pub item_id: i64,
}

/// `POST /cart/remove` — удалить позицию, редирект на `/cart`. Нет cart-cookie / корзина не
/// найдена → редирект без изменений.
pub async fn remove(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<RemoveForm>,
) -> Response {
    match render_remove(&state, &headers, &form).await {
        Ok(response) => response,
        Err(err) => err.into_response(),
    }
}

async fn render_remove(
    state: &AppState,
    headers: &HeaderMap,
    form: &RemoveForm,
) -> Result<Response, WebError> {
    if let Some(cart) = find_cart(state, headers).await? {
        state
            .carts
            .remove_item(&cart.token_hash, form.item_id)
            .await?;
    }
    Ok(redirect_to_cart(state, None))
}

// -----------------------------------------------------------------
// Общие хелперы
// -----------------------------------------------------------------

/// Найти корзину по cart-cookie. `None`, если cookie отсутствует или корзина с таким токеном
/// не найдена (например, истекла/была очищена) — НЕ ошибка.
async fn find_cart(state: &AppState, headers: &HeaderMap) -> Result<Option<Cart>, WebError> {
    let Some(token) = extract_cart_token(headers) else {
        return Ok(None);
    };
    Ok(state.carts.find_by_token(&token).await?)
}

/// Получить существующую корзину по cookie или создать новую. Возвращает корзину и —
/// если она была только что создана — raw cart-токен (для `Set-Cookie`).
async fn get_or_create_cart(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(Cart, Option<String>), WebError> {
    if let Some(cart) = find_cart(state, headers).await? {
        return Ok((cart, None));
    }
    let (token, cart) = state.carts.create_cart().await?;
    Ok((cart, Some(token.0)))
}

/// Редирект 303 → `/cart`, опционально с `Set-Cookie` для новой корзины.
///
/// `Secure`-атрибут cookie выводится из `state.base_url` (`https://` → cookie с `Secure`).
fn redirect_to_cart(state: &AppState, new_token: Option<String>) -> Response {
    match new_token {
        Some(raw) => {
            let secure = state.base_url.starts_with("https://");
            (
                [(SET_COOKIE, set_cart_cookie(&raw, secure))],
                Redirect::to("/cart"),
            )
                .into_response()
        }
        None => Redirect::to("/cart").into_response(),
    }
}
