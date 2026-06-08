//! Проекция каталога — консьюмер событий Product* (T1a-5, design-1a.md §1.4).
//!
//! `CatalogProjection` реализует `relay::Dispatcher` и применяет события из product context
//! к таблицам `product_projection`, `product_attr_index`, `product_fts` в catalog.db.
//! Согласованность eventual: проекция может отставать на интервал relay (< 1 с).
//!
//! Схема событий (product outbox):
//! - `ProductCreated`  — `{id, seller_id, title, slug, description, price_minor, currency, status, created_at, updated_at}`
//! - `ProductUpdated` — без `reason`: `{id, title, slug, description, price_minor, currency, updated_at}`
//!   — `reason:"attribute_value_set"`:   `{id, attribute_id, data_type, val_text, val_num, updated_at}`
//!   — `reason:"attribute_value_cleared"`: `{id, attribute_id, updated_at}`
//!   — прочие reason (media, variant, …): `{id, reason, updated_at}` — только `updated_at`
//! - `ProductPublished` / `ProductUnpublished` / `ProductArchived` — `{id, from, to, updated_at}`
//! - `ProductDeleted`  — `{id}`

use db::{ContextDb, relay};
use shared::DomainEvent;
use sqlx::Row;

pub struct CatalogProjection {
    db: ContextDb,
}

impl CatalogProjection {
    pub fn new(db: ContextDb) -> Self {
        Self { db }
    }
}

impl relay::Dispatcher for CatalogProjection {
    async fn dispatch(
        &self,
        _source: &str,
        event: &DomainEvent,
    ) -> Result<(), relay::DispatchError> {
        let mut tx = self
            .db
            .writer
            .begin()
            .await
            .map_err(|e| relay::DispatchError(e.to_string()))?;

        let result = match event.event_type.as_str() {
            "ProductCreated" => apply_created(&mut tx, &event.payload).await,
            "ProductUpdated" => apply_updated(&mut tx, &event.payload).await,
            "ProductPublished" | "ProductUnpublished" | "ProductArchived" => {
                apply_status_changed(&mut tx, &event.payload).await
            }
            "ProductDeleted" => apply_deleted(&mut tx, &event.payload).await,
            _ => Ok(()),
        };

        if let Err(e) = result {
            return Err(relay::DispatchError(e.to_string()));
        }

        tx.commit()
            .await
            .map_err(|e| relay::DispatchError(e.to_string()))?;
        Ok(())
    }
}

// -----------------------------------------------------------------
// Обработчики событий
// -----------------------------------------------------------------

async fn apply_created(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    p: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    let id = str_val(p, "id");
    sqlx::query(
        "INSERT OR IGNORE INTO product_projection \
         (id, seller_id, title, slug, description, price_minor, currency, status, created_at, updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(str_val(p, "seller_id"))
    .bind(str_val(p, "title"))
    .bind(str_val(p, "slug"))
    .bind(str_val(p, "description"))
    .bind(p["price_minor"].as_i64().unwrap_or(0))
    .bind(str_val(p, "currency"))
    .bind(str_val(p, "status"))
    .bind(p["created_at"].as_i64().unwrap_or(0))
    .bind(p["updated_at"].as_i64().unwrap_or(0))
    .execute(&mut **tx)
    .await?;

    upsert_fts(tx, &id).await
}

async fn apply_updated(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    p: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    let id = str_val(p, "id");
    let reason = p["reason"].as_str().unwrap_or("");

    match reason {
        "attribute_value_set" => {
            let attribute_id = str_val(p, "attribute_id");
            // category_id is looked up within the same catalog.db — not a cross-context read.
            let category_id: Option<String> =
                sqlx::query_scalar("SELECT category_id FROM attributes WHERE id = ?")
                    .bind(&attribute_id)
                    .fetch_optional(&mut **tx)
                    .await?;

            if let Some(cat_id) = category_id {
                let val_text: Option<String> = p["val_text"].as_str().map(str::to_string);
                let val_num: Option<f64> = p["val_num"].as_f64();
                sqlx::query(
                    "INSERT INTO product_attr_index (product_id, category_id, attribute_id, val_text, val_num) \
                     VALUES (?, ?, ?, ?, ?) \
                     ON CONFLICT (product_id, attribute_id) DO UPDATE SET \
                       category_id = excluded.category_id, \
                       val_text    = excluded.val_text, \
                       val_num     = excluded.val_num",
                )
                .bind(&id)
                .bind(&cat_id)
                .bind(&attribute_id)
                .bind(&val_text)
                .bind(val_num)
                .execute(&mut **tx)
                .await?;
            }

            upsert_fts(tx, &id).await?;
        }
        "attribute_value_cleared" => {
            let attribute_id = str_val(p, "attribute_id");
            sqlx::query("DELETE FROM product_attr_index WHERE product_id = ? AND attribute_id = ?")
                .bind(&id)
                .bind(&attribute_id)
                .execute(&mut **tx)
                .await?;

            upsert_fts(tx, &id).await?;
        }
        "" => {
            // Core update from ProductRepo::update_product()
            sqlx::query(
                "UPDATE product_projection \
                 SET title = ?, slug = ?, description = ?, price_minor = ?, currency = ?, updated_at = ? \
                 WHERE id = ?",
            )
            .bind(str_val(p, "title"))
            .bind(str_val(p, "slug"))
            .bind(str_val(p, "description"))
            .bind(p["price_minor"].as_i64().unwrap_or(0))
            .bind(str_val(p, "currency"))
            .bind(p["updated_at"].as_i64().unwrap_or(0))
            .bind(&id)
            .execute(&mut **tx)
            .await?;

            upsert_fts(tx, &id).await?;
        }
        _ => {
            // media_added, variant_added, translation_set, etc. — touch updated_at only.
            if let Some(ts) = p["updated_at"].as_i64() {
                sqlx::query("UPDATE product_projection SET updated_at = ? WHERE id = ?")
                    .bind(ts)
                    .bind(&id)
                    .execute(&mut **tx)
                    .await?;
            }
        }
    }
    Ok(())
}

async fn apply_status_changed(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    p: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    let id = str_val(p, "id");
    let to = str_val(p, "to");
    let updated_at = p["updated_at"].as_i64().unwrap_or(0);
    sqlx::query("UPDATE product_projection SET status = ?, updated_at = ? WHERE id = ?")
        .bind(&to)
        .bind(updated_at)
        .bind(&id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn apply_deleted(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    p: &serde_json::Value,
) -> Result<(), sqlx::Error> {
    let id = str_val(p, "id");

    sqlx::query("DELETE FROM product_attr_index WHERE product_id = ?")
        .bind(&id)
        .execute(&mut **tx)
        .await?;

    delete_fts(tx, &id).await?;

    sqlx::query("DELETE FROM product_projection WHERE id = ?")
        .bind(&id)
        .execute(&mut **tx)
        .await?;

    Ok(())
}

// -----------------------------------------------------------------
// Вспомогательные функции
// -----------------------------------------------------------------

/// Обновить FTS-запись товара: удалить старую (по product_id), вставить новую.
/// Читает title/description из product_projection и attrs из product_attr_index.
/// Если товар ещё не в проекции — нет-оп (FTS не нужен без строки в projection).
async fn upsert_fts(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    product_id: &str,
) -> Result<(), sqlx::Error> {
    let row = sqlx::query("SELECT title, description FROM product_projection WHERE id = ?")
        .bind(product_id)
        .fetch_optional(&mut **tx)
        .await?;

    let (title, description) = match row {
        Some(r) => (
            r.get::<String, _>("title"),
            r.get::<String, _>("description"),
        ),
        None => return Ok(()),
    };

    let attrs: String = sqlx::query_scalar(
        "SELECT COALESCE(GROUP_CONCAT(val_text, ' '), '') \
         FROM product_attr_index \
         WHERE product_id = ? AND val_text IS NOT NULL AND val_text != ''",
    )
    .bind(product_id)
    .fetch_one(&mut **tx)
    .await?;

    delete_fts(tx, product_id).await?;

    sqlx::query(
        "INSERT INTO product_fts(product_id, title, description, attrs) VALUES (?, ?, ?, ?)",
    )
    .bind(product_id)
    .bind(title)
    .bind(description)
    .bind(attrs)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

async fn delete_fts(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    product_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM product_fts WHERE product_id = ?")
        .bind(product_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn str_val(p: &serde_json::Value, key: &str) -> String {
    p[key].as_str().unwrap_or("").to_string()
}
