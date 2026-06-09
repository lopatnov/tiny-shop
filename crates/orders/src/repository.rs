//! Репозиторий заказов поверх `db::ContextDb` (T1a-8).
//!
//! ## Транзакционный outbox
//! `create_order` записывает заказ и событие `OrderCreated` в одну транзакцию `orders.db`.
//! `add_item` не эмитит отдельное событие (позиции — часть агрегата заказа).
//!
//! ## Checkout — Phase 1b
//! Переходы статусов (Paid/Fulfilled/Cancelled), пересчёт `total_minor` и вся логика
//! checkout реализуются в Phase 1b. Здесь — только создание и чтение.

use sqlx::Row;

use db::{ContextDb, outbox};
use shared::{NewEvent, Page, Pagination, now_ms};

use crate::order::{NewOrder, NewOrderItem, Order, OrderItem, OrderStatus};

const AGGREGATE: &str = "order";

/// Ошибки репозитория заказов.
#[derive(Debug, thiserror::Error)]
pub enum OrderError {
    #[error("db: {0}")]
    Db(#[from] sqlx::Error),
    #[error("invalid {field} in row: {value}")]
    InvalidEnum { field: &'static str, value: String },
    #[error("order not found: {0}")]
    NotFound(String),
    #[error("serialization: {0}")]
    Json(#[from] serde_json::Error),
}

impl From<db::DbError> for OrderError {
    fn from(e: db::DbError) -> Self {
        match e {
            db::DbError::Sqlx(e) => OrderError::Db(e),
            other => OrderError::Db(sqlx::Error::Protocol(other.to_string())),
        }
    }
}

/// Репозиторий заказов — создание, чтение, добавление позиций.
#[derive(Clone)]
pub struct OrderRepo {
    db: ContextDb,
}

impl OrderRepo {
    pub fn new(db: ContextDb) -> Self {
        Self { db }
    }

    // -----------------------------------------------------------------
    // Запись
    // -----------------------------------------------------------------

    /// Создать пустой заказ в статусе `created`. Эмитит `OrderCreated` через transactional outbox.
    pub async fn create_order(&self, o: &NewOrder) -> Result<(), OrderError> {
        let created_at = now_ms();
        let payload = serde_json::json!({
            "order_id": o.id,
            "buyer_id": o.buyer_id,
            "currency": o.currency,
        });
        let mut tx = self.db.writer.begin().await?;
        sqlx::query(
            "INSERT INTO orders (id, buyer_id, status, total_minor, currency, created_at) \
             VALUES (?, ?, 'created', 0, ?, ?)",
        )
        .bind(&o.id)
        .bind(&o.buyer_id)
        .bind(&o.currency)
        .bind(created_at)
        .execute(&mut *tx)
        .await?;
        outbox::enqueue(
            &mut *tx,
            &NewEvent {
                aggregate: AGGREGATE.to_string(),
                aggregate_id: o.id.clone(),
                event_type: "OrderCreated".to_string(),
                payload,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Добавить позицию к заказу. Recalculates total_minor.
    pub async fn add_item(&self, item: &NewOrderItem) -> Result<(), OrderError> {
        let snapshot_text = item
            .config_snapshot
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let mut tx = self.db.writer.begin().await?;
        sqlx::query(
            "INSERT INTO order_items \
             (id, order_id, product_id, seller_id, variant_id, title, \
              unit_price_minor, currency, config_snapshot) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&item.id)
        .bind(&item.order_id)
        .bind(&item.product_id)
        .bind(&item.seller_id)
        .bind(&item.variant_id)
        .bind(&item.title)
        .bind(item.unit_price_minor)
        .bind(&item.currency)
        .bind(&snapshot_text)
        .execute(&mut *tx)
        .await?;
        // Recalculate total from all items (safe: single writer, no concurrent modification).
        sqlx::query(
            "UPDATE orders SET total_minor = \
             (SELECT COALESCE(SUM(unit_price_minor), 0) FROM order_items WHERE order_id = ?) \
             WHERE id = ?",
        )
        .bind(&item.order_id)
        .bind(&item.order_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Чтение
    // -----------------------------------------------------------------

    /// Получить заказ по id (без позиций).
    pub async fn get_order(&self, id: &str) -> Result<Option<Order>, OrderError> {
        let row = sqlx::query(
            "SELECT id, buyer_id, status, total_minor, currency, created_at FROM orders WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.db.reader)
        .await?;
        row.map(|r| map_order_row(&r)).transpose()
    }

    /// Получить заказ с позициями.
    pub async fn get_order_with_items(&self, id: &str) -> Result<Option<Order>, OrderError> {
        let row = sqlx::query(
            "SELECT id, buyer_id, status, total_minor, currency, created_at FROM orders WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.db.reader)
        .await?;
        let Some(row) = row else { return Ok(None) };
        let mut order = map_order_row(&row)?;
        order.items = self.fetch_items(id).await?;
        Ok(Some(order))
    }

    /// Список заказов покупателя (без позиций), с пагинацией.
    pub async fn list_by_buyer(
        &self,
        buyer_id: &str,
        pg: &Pagination,
    ) -> Result<Page<Order>, OrderError> {
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders WHERE buyer_id = ?")
            .bind(buyer_id)
            .fetch_one(&self.db.reader)
            .await?;
        let rows = sqlx::query(
            "SELECT id, buyer_id, status, total_minor, currency, created_at \
             FROM orders WHERE buyer_id = ? \
             ORDER BY created_at DESC \
             LIMIT ? OFFSET ?",
        )
        .bind(buyer_id)
        .bind(pg.limit as i64)
        .bind(pg.offset as i64)
        .fetch_all(&self.db.reader)
        .await?;
        let items = rows
            .iter()
            .map(map_order_row)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Page {
            items,
            total: total as u64,
            page: *pg,
        })
    }

    async fn fetch_items(&self, order_id: &str) -> Result<Vec<OrderItem>, OrderError> {
        let rows = sqlx::query(
            "SELECT id, order_id, product_id, seller_id, variant_id, title, \
              unit_price_minor, currency, config_snapshot \
             FROM order_items WHERE order_id = ? ORDER BY rowid",
        )
        .bind(order_id)
        .fetch_all(&self.db.reader)
        .await?;
        rows.iter().map(map_item_row).collect()
    }
}

fn map_order_row(r: &sqlx::sqlite::SqliteRow) -> Result<Order, OrderError> {
    let status_str: String = r.try_get("status")?;
    let status = OrderStatus::parse(&status_str).ok_or_else(|| OrderError::InvalidEnum {
        field: "status",
        value: status_str,
    })?;
    Ok(Order {
        id: r.try_get("id")?,
        buyer_id: r.try_get("buyer_id")?,
        status,
        total_minor: r.try_get("total_minor")?,
        currency: r.try_get("currency")?,
        created_at: r.try_get("created_at")?,
        items: vec![],
    })
}

fn map_item_row(r: &sqlx::sqlite::SqliteRow) -> Result<OrderItem, OrderError> {
    let snapshot: Option<String> = r.try_get("config_snapshot")?;
    let config_snapshot = snapshot
        .map(|s| serde_json::from_str::<serde_json::Value>(&s))
        .transpose()?;
    Ok(OrderItem {
        id: r.try_get("id")?,
        order_id: r.try_get("order_id")?,
        product_id: r.try_get("product_id")?,
        seller_id: r.try_get("seller_id")?,
        variant_id: r.try_get("variant_id")?,
        title: r.try_get("title")?,
        unit_price_minor: r.try_get("unit_price_minor")?,
        currency: r.try_get("currency")?,
        config_snapshot,
    })
}
