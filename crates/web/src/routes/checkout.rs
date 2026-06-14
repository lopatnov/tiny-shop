//! Checkout (guest) — T1b-2: `GET /checkout`, `POST /checkout`, `GET /checkout/done/{order_id}`.
//!
//! Заказы оформляются без аккаунта: контактные данные (email/имя) собираются формой на
//! `/checkout`, заказ создаётся с синтетическим `buyer_id` вида `guest:<uuid>` (см.
//! [`guest_buyer_id`]). Логин-checkout — отдельный follow-up (см. `.claude/backlog/tasks.md`,
//! chunk 2.5).
//!
//! Цена/`seller_id` позиций берутся свежим снимком из `product_projection`
//! (`SqliteCatalogSearch::get_card_by_id`) на момент оформления, а не из снимка корзины —
//! заказ фиксирует актуальную цену, даже если товар был переопубликован с другой ценой между
//! добавлением в корзину и checkout.

use axum::extract::{Form, Path, State};
use axum::http::HeaderMap;
use axum::http::header::SET_COOKIE;
use axum::response::{Html, IntoResponse, Redirect, Response};
use maud::{Markup, html};
use orders::{CartItem, NewOrderContact, NewOrderItem, Order};
use serde::Deserialize;

use crate::AppState;
use crate::cart_cookie::{expire_cart_cookie, extract_cart_token};
use crate::error::WebError;
use crate::jsonld::format_price_minor;
use crate::view::layout::page_shell;

/// Верхний предел длины email/имени в форме контакта — защита от чрезмерного PII-ввода
/// (security-engineer, T1b-2).
const MAX_EMAIL_LEN: usize = 254; // RFC 5321 максимум длины адреса.
const MAX_NAME_LEN: usize = 200;

// -----------------------------------------------------------------
// GET /checkout
// -----------------------------------------------------------------

/// `GET /checkout` — сводка заказа из корзины + форма контакта. Пустая корзина / нет
/// cart-cookie → редирект 303 на `/cart` (заказ не создаём).
pub async fn show(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match render_show(&state, &headers).await {
        Ok(response) => response,
        Err(err) => err.into_response(),
    }
}

async fn render_show(state: &AppState, headers: &HeaderMap) -> Result<Response, WebError> {
    let items = cart_items(state, headers).await?;
    if items.is_empty() {
        return Ok(Redirect::to("/cart").into_response());
    }

    let main = html! {
        h1 { "Оформлення замовлення" }
        (summary_table(&items))
        (contact_form(None))
        p { a href="/cart" { "Назад до кошика" } }
    };
    Ok(Html(page_shell("Оформлення замовлення", html! {}, main).into_string()).into_response())
}

/// Сводная таблица позиций корзины и суммы (как `cart::cart_table`, но без форм
/// зміни/видалення — checkout лише показує підсумок).
fn summary_table(items: &[CartItem]) -> Markup {
    let total_minor: i64 = items
        .iter()
        .map(|i| i.unit_price_minor.saturating_mul(i.qty))
        .fold(0i64, |acc, x| acc.saturating_add(x));
    let currency = items.first().map(|i| i.currency.as_str()).unwrap_or("UAH");

    html! {
        table {
            caption { "Товари в замовленні" }
            thead {
                tr {
                    th scope="col" { "Товар" }
                    th scope="col" { "Кількість" }
                    th scope="col" { "Ціна за од." }
                    th scope="col" { "Сума" }
                }
            }
            tbody {
                @for item in items {
                    tr {
                        td { (item.title) }
                        td { (item.qty) }
                        td { (format_price_minor(item.unit_price_minor)) " " (item.currency) }
                        td { (format_price_minor(item.unit_price_minor.saturating_mul(item.qty))) " " (item.currency) }
                    }
                }
            }
            tfoot {
                tr {
                    th scope="row" colspan="3" { "Разом" }
                    td { (format_price_minor(total_minor)) " " (currency) }
                }
            }
        }
    }
}

/// Форма контактных данных гостя. `error` — сообщение валидации для повторного показа после
/// `POST /checkout` с некорректными данными (сохраняет введённые значения).
fn contact_form(prefill: Option<&ContactForm>) -> Markup {
    let email = prefill.map(|f| f.email.as_str()).unwrap_or("");
    let name = prefill.and_then(|f| f.name.as_deref()).unwrap_or("");

    html! {
        form method="post" action="/checkout" {
            div {
                label for="email" { "Електронна пошта" }
                input type="email" id="email" name="email" value=(email)
                    maxlength=(MAX_EMAIL_LEN) required;
            }
            div {
                label for="name" { "Ім'я (необов'язково)" }
                input type="text" id="name" name="name" value=(name) maxlength=(MAX_NAME_LEN);
            }
            button type="submit" { "Оформити замовлення" }
        }
    }
}

// -----------------------------------------------------------------
// POST /checkout
// -----------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ContactForm {
    pub email: String,
    #[serde(default)]
    pub name: Option<String>,
}

/// `POST /checkout` — оформить заказ. Пустая корзина / нет cart-cookie → редирект 303 на
/// `/cart`. Некорректный email/имя → 400 с повторным показом формы (введённые значения
/// сохраняются).
pub async fn submit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<ContactForm>,
) -> Response {
    match render_submit(&state, &headers, &form).await {
        Ok(response) => response,
        Err(err) => err.into_response(),
    }
}

async fn render_submit(
    state: &AppState,
    headers: &HeaderMap,
    form: &ContactForm,
) -> Result<Response, WebError> {
    let Some(cart) = find_cart(state, headers).await? else {
        return Ok(Redirect::to("/cart").into_response());
    };
    let cart_items = state.carts.list_items(&cart.token_hash).await?;
    if cart_items.is_empty() {
        return Ok(Redirect::to("/cart").into_response());
    }

    let contact = match validate_contact(form) {
        Ok(contact) => contact,
        Err(message) => return Ok(invalid_contact_response(&cart_items, form, &message)),
    };

    // Свежий снимок цены/seller_id из проекции каталога — позиция корзины может хранить
    // устаревшую цену (товар переопубликован с момента добавления).
    let total_qty: usize = cart_items.iter().map(|item| item.qty.max(0) as usize).sum();
    let mut order_items = Vec::with_capacity(total_qty);
    for cart_item in &cart_items {
        let Some(card) = state
            .search
            .get_card_by_id(&cart_item.product_id)
            .await
            .map_err(|e| WebError::Internal(e.to_string()))?
        else {
            // Товар знято з публікації між додаванням у кошик і checkout — підсумок на сторінці
            // показував цю позицію, тож тихо пропускати її було б розбіжністю між тим, що
            // побачив користувач, і тим, що реально замовлено. Повертаємо помилку — користувач
            // повертається в кошик і прибирає недоступну позицію свідомо.
            return Err(WebError::BadRequest(format!(
                "Товар \"{}\" більше не доступний для замовлення. Будь ласка, поверніться до кошика.",
                cart_item.title
            )));
        };
        // Одна позиція корзини з qty=N розгортається в N рядків order_items (T1b-2): кожен
        // рядок — незмінний снімок ОДНІЄЇ одиниці товару (схема `order_items` без стовпця qty,
        // T1a-8), а total_minor рахується як SUM(unit_price_minor) — той самий запит, що
        // `add_item`.
        for _ in 0..cart_item.qty {
            order_items.push(NewOrderItem {
                id: uuid::Uuid::new_v4().to_string(),
                order_id: String::new(), // заповнюється всередині OrderRepo::checkout
                product_id: card.id.clone(),
                seller_id: card.seller_id.clone(),
                variant_id: cart_item.variant_id.clone(),
                title: card.title.clone(),
                unit_price_minor: card.price_minor,
                currency: card.currency.clone(),
                config_snapshot: None,
            });
        }
    }
    if order_items.is_empty() {
        return Err(WebError::BadRequest(
            "усі товари кошика більше не доступні".to_string(),
        ));
    }

    let currency = order_items[0].currency.clone();
    if order_items.iter().any(|item| item.currency != currency) {
        // Товари в одній корзині оновили статус і тепер у різних валютах (рідкий випадок —
        // зміна валюти товару продавцем між додаванням у кошик і checkout). total_minor у
        // одній валюті був би некоректним, тож відмовляємо явно замість тихого змішування.
        return Err(WebError::BadRequest(
            "товари в кошику мають різні валюти, оформлення неможливе".to_string(),
        ));
    }
    let buyer_id = guest_buyer_id();
    let order_id = state
        .orders
        .checkout(&buyer_id, &currency, &order_items, Some(&contact))
        .await?;

    state.carts.clear(&cart.token_hash).await?;

    let secure = state.base_url.starts_with("https://");
    Ok((
        [(SET_COOKIE, expire_cart_cookie(secure))],
        Redirect::to(&format!("/checkout/done/{order_id}")),
    )
        .into_response())
}

/// Синтетический `buyer_id` для гостевых заказов: `guest:<uuid>`. Удовлетворяет
/// `orders.buyer_id NOT NULL`, не выдаёт себя за identity-аккаунт; логин-checkout (follow-up)
/// подставит реальный account id вместо `guest:...`.
fn guest_buyer_id() -> String {
    format!("guest:{}", uuid::Uuid::new_v4())
}

/// Валидация контактных данных: email — непустой, ≤ [`MAX_EMAIL_LEN`], содержит `@` с
/// непустыми частями до/после, без управляющих символов/пробелов; имя (если задано) —
/// ≤ [`MAX_NAME_LEN`], без управляющих символов. Возвращает [`NewOrderContact`] либо
/// человекочитаемое сообщение ошибки (укр.) для повторного показа формы.
fn validate_contact(form: &ContactForm) -> Result<NewOrderContact, String> {
    let email = form.email.trim();
    if email.is_empty() || email.len() > MAX_EMAIL_LEN {
        return Err("Вкажіть коректну електронну пошту.".to_string());
    }
    if email.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("Електронна пошта містить недопустимі символи.".to_string());
    }
    let Some((local, domain)) = email.split_once('@') else {
        return Err("Вкажіть коректну електронну пошту.".to_string());
    };
    if local.is_empty() || domain.is_empty() || !domain.contains('.') {
        return Err("Вкажіть коректну електронну пошту.".to_string());
    }

    let name = form
        .name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(name) = name
        && (name.len() > MAX_NAME_LEN || name.chars().any(char::is_control))
    {
        return Err("Ім'я містить недопустимі символи або занадто довге.".to_string());
    }

    Ok(NewOrderContact {
        email: email.to_string(),
        name: name.map(str::to_string),
    })
}

/// 400-ответ с повторным показом формы checkout, сохранив введённые значения и показав
/// сообщение об ошибке валидации.
fn invalid_contact_response(items: &[CartItem], form: &ContactForm, message: &str) -> Response {
    let main = html! {
        h1 { "Оформлення замовлення" }
        (summary_table(items))
        p role="alert" { (message) }
        (contact_form(Some(form)))
        p { a href="/cart" { "Назад до кошика" } }
    };
    let body = page_shell("Оформлення замовлення", html! {}, main).into_string();
    (axum::http::StatusCode::BAD_REQUEST, Html(body)).into_response()
}

// -----------------------------------------------------------------
// GET /checkout/done/{order_id}
// -----------------------------------------------------------------

/// `GET /checkout/done/{order_id}` — страница подтверждения заказа.
///
/// `order_id` — uuid v4 (неугадываемый), заказ показывается по знанию id без проверки сессии
/// ("confirmation by link"). Это осознанный trade-off guest-MVP (см. ADR "Checkout (guest)" в
/// `roadmap.md`); `order_contact` (email/имя) не читается и не отображается на этой странице —
/// confirmation показывает только позиции/сумму заказа.
pub async fn done(State(state): State<AppState>, Path(order_id): Path<String>) -> Response {
    match render_done(&state, &order_id).await {
        Ok(html) => Html(html).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn render_done(state: &AppState, order_id: &str) -> Result<String, WebError> {
    let order = state
        .orders
        .get_order_with_items(order_id)
        .await?
        .ok_or(WebError::NotFound)?;

    let main = html! {
        h1 { "Замовлення прийнято" }
        p { "Номер замовлення: " strong { (order.id) } }
        (order_summary(&order))
        p {
            "Дякуємо за замовлення! Інформацію про видачу товару буде надіслано на вашу "
            "електронну пошту."
        }
        p { a href="/" { "До каталогу" } }
    };
    Ok(page_shell("Замовлення прийнято", html! {}, main).into_string())
}

fn order_summary(order: &Order) -> Markup {
    html! {
        table {
            caption { "Склад замовлення" }
            thead {
                tr {
                    th scope="col" { "Товар" }
                    th scope="col" { "Ціна" }
                }
            }
            tbody {
                @for item in &order.items {
                    tr {
                        td { (item.title) }
                        td { (format_price_minor(item.unit_price_minor)) " " (item.currency) }
                    }
                }
            }
            tfoot {
                tr {
                    th scope="row" { "Разом" }
                    td { (format_price_minor(order.total_minor)) " " (order.currency) }
                }
            }
        }
    }
}

// -----------------------------------------------------------------
// Общие хелперы
// -----------------------------------------------------------------

/// Найти корзину по cart-cookie и вернуть её позиции. Нет cookie / корзина не найдена →
/// пустой список (не ошибка) — вызывающая сторона редиректит на `/cart`.
async fn cart_items(state: &AppState, headers: &HeaderMap) -> Result<Vec<CartItem>, WebError> {
    match find_cart(state, headers).await? {
        Some(cart) => Ok(state.carts.list_items(&cart.token_hash).await?),
        None => Ok(Vec::new()),
    }
}

async fn find_cart(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<orders::Cart>, WebError> {
    let Some(token) = extract_cart_token(headers) else {
        return Ok(None);
    };
    Ok(state.carts.find_by_token(&token).await?)
}
