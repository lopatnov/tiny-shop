//! Доменные типы корзины (T1b-1).
//!
//! Корзина анонимна и адресуется по opaque cart-токену (см. `cart_repo`). Позиции хранят
//! денормализованный снимок названия/цены на момент добавления — для отображения; источник
//! истины подтверждается на checkout (Phase 1b chunk 2).

/// Корзина — идентифицируется хэшем cart-токена.
#[derive(Debug, Clone)]
pub struct Cart {
    pub token_hash: String,
    /// Unix-миллисекунды создания.
    pub created_at: i64,
    /// Unix-миллисекунды последнего изменения (новая позиция/изменение qty).
    pub updated_at: i64,
}

/// Позиция корзины — снимок товара/варианта на момент добавления.
#[derive(Debug, Clone)]
pub struct CartItem {
    pub id: i64,
    pub cart_id: String,
    pub product_id: String,
    /// Вариант/опция; `None` если товар без вариантов.
    pub variant_id: Option<String>,
    pub qty: i64,
    /// Снимок названия на момент добавления.
    pub title: String,
    /// Снимок цены в минорных единицах.
    pub unit_price_minor: i64,
    pub currency: String,
    /// Unix-миллисекунды добавления.
    pub added_at: i64,
}

/// Входные данные для добавления позиции в корзину.
#[derive(Debug, Clone)]
pub struct NewCartItem {
    pub product_id: String,
    pub variant_id: Option<String>,
    pub qty: i64,
    pub title: String,
    pub unit_price_minor: i64,
    pub currency: String,
}

/// Opaque raw cart-токен, возвращаемый клиенту (значение cookie). В БД хранится только
/// его BLAKE3-хэш — см. `cart_repo::generate_token`/`hash_token`.
#[derive(Debug, Clone)]
pub struct CartToken(pub String);

impl CartToken {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
