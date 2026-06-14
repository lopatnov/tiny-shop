//! Хелперы cart-cookie (T1b-1).
//!
//! Корзина анонимна: клиент идентифицируется по opaque cart-токену, который сервер
//! хранит в cookie `cart`. Без `axum-extra` (Простота) — парсинг/сборка вручную через
//! `axum::http::header::{COOKIE, SET_COOKIE}`.

use axum::http::HeaderMap;
use axum::http::header::COOKIE;

/// Имя cart-cookie.
pub const CART_COOKIE_NAME: &str = "cart";

/// Срок жизни cart-cookie — 30 дней (в секундах), `Max-Age`.
const MAX_AGE_SECS: u64 = 60 * 60 * 24 * 30;

/// Извлечь raw cart-токен из заголовка `Cookie`, если в нём есть пара `cart=<value>`.
///
/// Заголовок `Cookie` может содержать несколько пар через `; ` — разбираем все и ищем
/// нужное имя. Пустое значение (`cart=`) считается отсутствующим токеном.
pub fn extract_cart_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|pair| {
        let pair = pair.trim();
        let (name, value) = pair.split_once('=')?;
        if name == CART_COOKIE_NAME && !value.is_empty() {
            Some(value.to_string())
        } else {
            None
        }
    })
}

/// Собрать значение заголовка `Set-Cookie` для нового/обновлённого cart-токена.
///
/// `HttpOnly` (недоступен из JS — защита от XSS-кражи токена), `SameSite=Lax` (CSRF-защита
/// для cross-site навигации, но допускает обычные GET-переходы по ссылкам), `Path=/`
/// (действует на весь сайт), `Max-Age` — 30 дней.
pub fn set_cart_cookie(raw_token: &str) -> String {
    format!(
        "{CART_COOKIE_NAME}={raw_token}; HttpOnly; SameSite=Lax; Path=/; Max-Age={MAX_AGE_SECS}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn extract_cart_token_from_single_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_static("cart=abc123"));
        assert_eq!(extract_cart_token(&headers), Some("abc123".to_string()));
    }

    #[test]
    fn extract_cart_token_among_multiple_cookies() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("theme=dark; cart=abc123; lang=uk"),
        );
        assert_eq!(extract_cart_token(&headers), Some("abc123".to_string()));
    }

    #[test]
    fn extract_cart_token_missing_returns_none() {
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_static("theme=dark"));
        assert_eq!(extract_cart_token(&headers), None);

        assert_eq!(extract_cart_token(&HeaderMap::new()), None);
    }

    #[test]
    fn extract_cart_token_empty_value_returns_none() {
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_static("cart="));
        assert_eq!(extract_cart_token(&headers), None);
    }

    #[test]
    fn set_cart_cookie_includes_security_attributes() {
        let value = set_cart_cookie("rawtoken");
        assert!(value.starts_with("cart=rawtoken;"));
        assert!(value.contains("HttpOnly"));
        assert!(value.contains("SameSite=Lax"));
        assert!(value.contains("Path=/"));
        assert!(value.contains("Max-Age=2592000"));
    }
}
