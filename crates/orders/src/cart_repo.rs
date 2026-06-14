//! Репозиторий корзины поверх `db::ContextDb` (T1b-1).
//!
//! ## Без outbox
//! Корзина — приватное состояние покупателя до checkout; `add_item`/`update_qty`/`remove_item`/
//! `clear` НЕ эмитят события (в отличие от `OrderRepo::create_order`).
//!
//! ## Cart-токен
//! `create_cart` генерирует случайный 48-символьный alnum-токен, хранит только его BLAKE3-хэш
//! (`carts.token_hash`) и возвращает raw-токен один раз — тот же паттерн, что
//! `identity::SessionRepo::create` (`crates/identity/src/session_repo.rs`). Токен/хэш-хелперы
//! (`generate_token`/`hash_token` ниже) продублированы локально, чтобы не связывать контексты
//! `orders`/`identity` ради пяти строк.

use sqlx::Row;

use db::ContextDb;
use shared::now_ms;

use crate::cart::{Cart, CartItem, CartToken, NewCartItem};

/// Верхний предел количества одной позиции — защита от переполнения total при отображении
/// (security-engineer, T1b-1).
pub const MAX_QTY: i64 = 999;

/// Ошибки репозитория корзины.
#[derive(Debug, thiserror::Error)]
pub enum CartError {
    #[error("db: {0}")]
    Db(#[from] sqlx::Error),
    #[error("quantity {0} out of range (0..={MAX_QTY})")]
    InvalidQty(i64),
}

/// Репозиторий корзины — создание, поиск по токену, управление позициями.
#[derive(Clone)]
pub struct CartRepo {
    db: ContextDb,
}

impl CartRepo {
    pub fn new(db: ContextDb) -> Self {
        Self { db }
    }

    /// Создать новую пустую корзину. Возвращает raw cart-токен (хранится только его хэш) и
    /// саму корзину — избегает повторного `find_by_token` сразу после создания.
    pub async fn create_cart(&self) -> Result<(CartToken, Cart), CartError> {
        let raw = generate_token();
        let token_hash = hash_token(&raw);
        let ts = now_ms();
        sqlx::query("INSERT INTO carts (token_hash, created_at, updated_at) VALUES (?, ?, ?)")
            .bind(&token_hash)
            .bind(ts)
            .bind(ts)
            .execute(&self.db.writer)
            .await?;
        let cart = Cart {
            token_hash,
            created_at: ts,
            updated_at: ts,
        };
        Ok((CartToken(raw), cart))
    }

    /// Найти корзину по raw cart-токену (хэширует и ищет по `token_hash`). `None`, если
    /// корзины с таким токеном нет.
    pub async fn find_by_token(&self, raw_token: &str) -> Result<Option<Cart>, CartError> {
        let token_hash = hash_token(raw_token);
        let row = sqlx::query(
            "SELECT token_hash, created_at, updated_at FROM carts WHERE token_hash = ?",
        )
        .bind(&token_hash)
        .fetch_optional(&self.db.reader)
        .await?;
        Ok(row.map(|r| Cart {
            token_hash: r.get("token_hash"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        }))
    }

    /// Добавить позицию в корзину. Повторное добавление того же (product_id, variant_id)
    /// инкрементирует `qty` существующей строки вместо дублирования и обновляет снимок
    /// (`title`/`unit_price_minor`/`currency`/`added_at`) до актуального каталожного — иначе
    /// строка осталась бы со снимком первого добавления. Обновляет `carts.updated_at`.
    ///
    /// Итоговый `qty` ограничен сверху `MAX_QTY` (`MIN(...)` в `ON CONFLICT`) — защита от
    /// переполнения total при многократном повторном добавлении, а не отдельная ошибка.
    pub async fn add_item(&self, cart_id: &str, item: &NewCartItem) -> Result<(), CartError> {
        if item.qty <= 0 || item.qty > MAX_QTY {
            return Err(CartError::InvalidQty(item.qty));
        }
        let ts = now_ms();
        let mut tx = self.db.writer.begin().await?;
        sqlx::query(
            "INSERT INTO cart_items \
             (cart_id, product_id, variant_id, qty, title, unit_price_minor, currency, added_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
             ON CONFLICT (cart_id, product_id, COALESCE(variant_id, '')) \
             DO UPDATE SET qty = MIN(qty + excluded.qty, ?), \
                           title = excluded.title, \
                           unit_price_minor = excluded.unit_price_minor, \
                           currency = excluded.currency, \
                           added_at = excluded.added_at",
        )
        .bind(cart_id)
        .bind(&item.product_id)
        .bind(&item.variant_id)
        .bind(item.qty)
        .bind(&item.title)
        .bind(item.unit_price_minor)
        .bind(&item.currency)
        .bind(ts)
        .bind(MAX_QTY)
        .execute(&mut *tx)
        .await?;
        touch_cart(&mut tx, cart_id, ts).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Изменить количество позиции, принадлежащей `cart_id`. `qty == 0` удаляет строку.
    /// Не затрагивает позиции других корзин (IDOR-защита: фильтр по `cart_id`).
    ///
    /// `carts.updated_at` обновляется только если строка действительно затронута — попытка
    /// изменить позицию другой корзины (0 affected rows) не должна "трогать" чужую корзину.
    pub async fn update_qty(&self, cart_id: &str, item_id: i64, qty: i64) -> Result<(), CartError> {
        if !(0..=MAX_QTY).contains(&qty) {
            return Err(CartError::InvalidQty(qty));
        }
        let ts = now_ms();
        let mut tx = self.db.writer.begin().await?;
        let affected = if qty == 0 {
            sqlx::query("DELETE FROM cart_items WHERE id = ? AND cart_id = ?")
                .bind(item_id)
                .bind(cart_id)
                .execute(&mut *tx)
                .await?
                .rows_affected()
        } else {
            sqlx::query("UPDATE cart_items SET qty = ? WHERE id = ? AND cart_id = ?")
                .bind(qty)
                .bind(item_id)
                .bind(cart_id)
                .execute(&mut *tx)
                .await?
                .rows_affected()
        };
        if affected > 0 {
            touch_cart(&mut tx, cart_id, ts).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Удалить позицию, принадлежащую `cart_id`. Не затрагивает позиции других корзин.
    ///
    /// `carts.updated_at` обновляется только если строка действительно удалена (см.
    /// `update_qty`).
    pub async fn remove_item(&self, cart_id: &str, item_id: i64) -> Result<(), CartError> {
        let ts = now_ms();
        let mut tx = self.db.writer.begin().await?;
        let affected = sqlx::query("DELETE FROM cart_items WHERE id = ? AND cart_id = ?")
            .bind(item_id)
            .bind(cart_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if affected > 0 {
            touch_cart(&mut tx, cart_id, ts).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Список позиций корзины в порядке добавления.
    pub async fn list_items(&self, cart_id: &str) -> Result<Vec<CartItem>, CartError> {
        let rows = sqlx::query(
            "SELECT id, cart_id, product_id, variant_id, qty, title, unit_price_minor, currency, added_at \
             FROM cart_items WHERE cart_id = ? ORDER BY id",
        )
        .bind(cart_id)
        .fetch_all(&self.db.reader)
        .await?;
        Ok(rows.iter().map(map_item_row).collect())
    }

    /// Удалить все позиции корзины (например, после успешного checkout).
    ///
    /// `carts.updated_at` обновляется только если позиции действительно были (см.
    /// `update_qty`) — `clear()` пустой корзины не "трогает" её.
    pub async fn clear(&self, cart_id: &str) -> Result<(), CartError> {
        let ts = now_ms();
        let mut tx = self.db.writer.begin().await?;
        let affected = sqlx::query("DELETE FROM cart_items WHERE cart_id = ?")
            .bind(cart_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
        if affected > 0 {
            touch_cart(&mut tx, cart_id, ts).await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

/// Обновить `carts.updated_at`. Вызывается внутри транзакции, изменяющей позиции.
async fn touch_cart(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    cart_id: &str,
    ts: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE carts SET updated_at = ? WHERE token_hash = ?")
        .bind(ts)
        .bind(cart_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn map_item_row(r: &sqlx::sqlite::SqliteRow) -> CartItem {
    CartItem {
        id: r.get("id"),
        cart_id: r.get("cart_id"),
        product_id: r.get("product_id"),
        variant_id: r.get("variant_id"),
        qty: r.get("qty"),
        title: r.get("title"),
        unit_price_minor: r.get("unit_price_minor"),
        currency: r.get("currency"),
        added_at: r.get("added_at"),
    }
}

/// Сгенерировать случайный 48-символьный alnum cart-токен (паттерн
/// `identity::session_repo::generate_token`).
fn generate_token() -> String {
    use rand::Rng;
    use rand::distributions::Alphanumeric;
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect()
}

/// BLAKE3-хэш raw-токена для хранения/поиска (паттерн `identity::session_repo::hash_token`).
fn hash_token(raw: &str) -> String {
    format!("{}", blake3::hash(raw.as_bytes()))
}
