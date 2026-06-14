//! Ошибки веб-слоя и их маппинг в HTTP-ответы (семантические HTML-страницы).

use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use maud::html;

use crate::view::layout::page_shell;

/// Ошибки обработчиков `web`.
#[derive(Debug, thiserror::Error)]
pub enum WebError {
    #[error("not found")]
    NotFound,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("internal error: {0}")]
    Internal(String),
}

/// `orders::CartError` → `WebError`: `InvalidQty` — некорректный ввод пользователя (400),
/// прочие (`Db`) — внутренняя ошибка (500).
impl From<orders::CartError> for WebError {
    fn from(err: orders::CartError) -> Self {
        match err {
            orders::CartError::InvalidQty(qty) => {
                WebError::BadRequest(format!("invalid quantity: {qty}"))
            }
            orders::CartError::Db(e) => WebError::Internal(e.to_string()),
        }
    }
}

impl IntoResponse for WebError {
    fn into_response(self) -> Response {
        match self {
            WebError::NotFound => not_found_response(),
            WebError::BadRequest(msg) => {
                tracing::warn!(error = %msg, "bad request");
                let body = page_shell(
                    "Некоректний запит",
                    html! {},
                    html! {
                        h1 { "Некоректний запит" }
                        p { "Дані форми некоректні. Перевірте введені значення і спробуйте ще раз." }
                        p { a href="/cart" { "До кошика" } }
                    },
                );
                (StatusCode::BAD_REQUEST, Html(body.into_string())).into_response()
            }
            WebError::Internal(msg) => {
                tracing::error!(error = %msg, "internal server error");
                let body = page_shell(
                    "Помилка сервера",
                    html! {},
                    html! {
                        h1 { "Помилка сервера" }
                        p { "Сталася неочікувана помилка. Спробуйте пізніше." }
                    },
                );
                (StatusCode::INTERNAL_SERVER_ERROR, Html(body.into_string())).into_response()
            }
        }
    }
}

fn not_found_response() -> Response {
    let body = page_shell(
        "Сторінку не знайдено",
        html! {},
        html! {
            h1 { "Сторінку не знайдено" }
            p { "Запитана сторінка не існує або була видалена." }
            p { a href="/" { "На головну" } }
        },
    );
    (StatusCode::NOT_FOUND, Html(body.into_string())).into_response()
}

/// Axum-фолбэк для незарегистрированных маршрутов.
pub async fn not_found() -> Response {
    not_found_response()
}
