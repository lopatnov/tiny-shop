//! Репозиторий товара продавца поверх `db::ContextDb` (T1a-4).
//!
//! ## Транзакционный outbox (отличие от `TaxonomyRepo`)
//! Каждая операция записи кладёт доменное изменение и `db::outbox::enqueue` в ОДНУ транзакцию
//! (`design-1a.md` §1.2–1.3): открываем `self.db.writer.begin()`, пишем доменные строки и
//! событие через `&mut *tx`, затем коммитим. **Никогда** не передавать `&self.db.writer` внутрь
//! открытой транзакции — `writer` имеет `max_connections(1)`, второй запрос к пулу зависнет до
//! `busy_timeout` (см. предупреждение в `crates/db/src/lib.rs`).
//!
//! ## i18n (ADR O5/T1a-4)
//! Канон `title`/`description`/`label` — поля сущности (язык `uk`). Перевод, если есть, лежит
//! в локальной `translations` и резолвится `LEFT JOIN ... COALESCE(override, канон)` — единая
//! ветка для любого [`Lang`], без `match lang { Uk => ..., _ => ... }` (см. прецедент
//! `crates/catalog/src/repository.rs`).
//!
//! ## Структура
//! Файл уже близок к ~1000 строкам (conventions.md «размер файла»), поэтому операции с
//! digital-конфигурацией/вариантами/медиа/атрибутами/переводами вынесены в [`crate::extras`]
//! как `impl ProductRepo` в отдельном файле — единый тип, разделённая ответственность.

use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use db::{ContextDb, outbox};
use shared::{NewEvent, Page, Pagination, now_ms};

use crate::product::{Lang, Product, ProductStatus};

/// Имя агрегата в outbox (`aggregate`/`aggregate_id` событий продукта).
pub(crate) const AGGREGATE: &str = "product";

/// Значения `translations.entity_type` (CHECK в миграции `0002_product.sql`).
pub(crate) mod entity_types {
    pub const PRODUCT: &str = "product";
    pub const DIGITAL_VARIANT: &str = "digital_variant";
}

/// Значения `translations.field` (CHECK в миграции `0002_product.sql`).
pub(crate) mod fields {
    pub const TITLE: &str = "title";
    pub const DESCRIPTION: &str = "description";
    pub const LABEL: &str = "label";
}

/// Ошибки репозитория товара.
#[derive(Debug, thiserror::Error)]
pub enum ProductError {
    #[error("db: {0}")]
    Db(#[from] sqlx::Error),
    /// Колонка БД содержит значение вне ожидаемого набора (CHECK должен это предотвращать,
    /// но парсер обязан вернуть ошибку, а не запаниковать).
    #[error("invalid {field} in row: {value}")]
    InvalidEnum { field: &'static str, value: String },
}

impl From<db::DbError> for ProductError {
    fn from(e: db::DbError) -> Self {
        match e {
            db::DbError::Sqlx(e) => ProductError::Db(e),
            // outbox::enqueue также может вернуть ошибку сериализации — не теряем её молча,
            // но в контракте репозитория единственный вариант БД-уровня — `Db`; маппим как sqlx
            // через явное сообщение, чтобы не плодить варианты ради редкого пути.
            other => ProductError::Db(sqlx::Error::Protocol(other.to_string())),
        }
    }
}

/// Репозиторий чтения/записи товаров продавца (+ цифровая конфигурация, медиа, атрибуты, переводы).
#[derive(Clone)]
pub struct ProductRepo {
    pub(crate) db: ContextDb,
}

impl ProductRepo {
    pub fn new(db: ContextDb) -> Self {
        Self { db }
    }

    // -----------------------------------------------------------------
    // Запись: товар
    // -----------------------------------------------------------------

    /// Создать товар. Эмитит `ProductCreated` в той же транзакции (transactional outbox).
    pub async fn create_product(&self, p: &Product) -> Result<(), ProductError> {
        let mut tx = self.db.writer.begin().await?;
        sqlx::query(
            "INSERT INTO products \
                (id, seller_id, title, slug, description, price_minor, currency, status, \
                 created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&p.id)
        .bind(&p.seller_id)
        .bind(&p.title)
        .bind(&p.slug)
        .bind(&p.description)
        .bind(p.price_minor)
        .bind(&p.currency)
        .bind(p.status.as_str())
        .bind(p.created_at)
        .bind(p.updated_at)
        .execute(&mut *tx)
        .await?;

        outbox::enqueue(
            &mut *tx,
            &NewEvent::new(AGGREGATE, &p.id, "ProductCreated", product_snapshot(p)),
        )
        .await?;

        tx.commit().await?;
        Ok(())
    }

    /// Обновить редактируемые поля товара (title/slug/description/price/currency).
    /// Статус меняется отдельно через [`Self::set_status`] (другая семантика событий).
    /// Эмитит `ProductUpdated` в той же транзакции.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_product(
        &self,
        id: &str,
        title: &str,
        slug: &str,
        description: &str,
        price_minor: i64,
        currency: &str,
    ) -> Result<(), ProductError> {
        let mut tx = self.db.writer.begin().await?;
        let now = now_ms();
        sqlx::query(
            "UPDATE products \
             SET title = ?, slug = ?, description = ?, price_minor = ?, currency = ?, updated_at = ? \
             WHERE id = ?",
        )
        .bind(title)
        .bind(slug)
        .bind(description)
        .bind(price_minor)
        .bind(currency)
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        outbox::enqueue(
            &mut *tx,
            &NewEvent::new(
                AGGREGATE,
                id,
                "ProductUpdated",
                serde_json::json!({
                    "id": id,
                    "title": title,
                    "slug": slug,
                    "description": description,
                    "price_minor": price_minor,
                    "currency": currency,
                    "updated_at": now,
                }),
            ),
        )
        .await?;

        tx.commit().await?;
        Ok(())
    }

    /// Сменить статус товара. Имя события зависит от перехода (см. [`status_transition_event`]).
    pub async fn set_status(&self, id: &str, to: ProductStatus) -> Result<(), ProductError> {
        let mut tx = self.db.writer.begin().await?;

        let row = sqlx::query("SELECT status FROM products WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        let Some(row) = row else {
            // Товар не найден — ничего не меняем и не эмитим событие. Вызывающая сторона
            // (web-слой) транслирует это в 404; репозиторий не выдумывает доменную ошибку
            // сверх того, что есть в `ProductError`.
            return Ok(());
        };
        let from_raw: String = row.get("status");
        let from = ProductStatus::parse(&from_raw).ok_or_else(|| ProductError::InvalidEnum {
            field: "status",
            value: from_raw.clone(),
        })?;
        if from == to {
            return Ok(());
        }

        let now = now_ms();
        sqlx::query("UPDATE products SET status = ?, updated_at = ? WHERE id = ?")
            .bind(to.as_str())
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;

        if let Some(event_type) = status_transition_event(from, to) {
            outbox::enqueue(
                &mut *tx,
                &NewEvent::new(
                    AGGREGATE,
                    id,
                    event_type,
                    serde_json::json!({
                        "id": id,
                        "from": from.as_str(),
                        "to": to.as_str(),
                        "updated_at": now,
                    }),
                ),
            )
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    /// Удалить товар. `ON DELETE CASCADE` чистит `product_media`/`digital_config`/
    /// `digital_variant`/`product_attribute_values`, но НЕ `translations` (полиморфная таблица
    /// без FK на `products`) — переводы товара и его вариантов удаляются явно, иначе остаются
    /// сиротами и могут коллизировать с будущими id. Эмитит `ProductDeleted`.
    pub async fn delete_product(&self, id: &str) -> Result<(), ProductError> {
        let mut tx = self.db.writer.begin().await?;

        sqlx::query("DELETE FROM translations WHERE entity_type = ? AND entity_id = ?")
            .bind(entity_types::PRODUCT)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "DELETE FROM translations WHERE entity_type = ? AND entity_id IN \
             (SELECT id FROM digital_variant WHERE product_id = ?)",
        )
        .bind(entity_types::DIGITAL_VARIANT)
        .bind(id)
        .execute(&mut *tx)
        .await?;

        let res = sqlx::query("DELETE FROM products WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;

        if res.rows_affected() > 0 {
            outbox::enqueue(
                &mut *tx,
                &NewEvent::new(
                    AGGREGATE,
                    id,
                    "ProductDeleted",
                    serde_json::json!({ "id": id }),
                ),
            )
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Чтение: товар
    // -----------------------------------------------------------------

    /// Прочитать товар по id, с резолвом title/description на запрошенный язык.
    pub async fn get_product(&self, id: &str, lang: Lang) -> Result<Option<Product>, ProductError> {
        let row = sqlx::query(sqlx::AssertSqlSafe(product_select_with_translations(
            "p.id = ?", "p.id ASC",
        )))
        .bind(entity_types::PRODUCT)
        .bind(lang.as_str())
        .bind(fields::TITLE)
        .bind(entity_types::PRODUCT)
        .bind(lang.as_str())
        .bind(fields::DESCRIPTION)
        .bind(id)
        .fetch_optional(&self.db.reader)
        .await?;
        row.map(product_from_row).transpose()
    }

    /// Прочитать товар продавца по `slug` (`UNIQUE(seller_id, slug)`).
    pub async fn get_product_by_slug(
        &self,
        seller_id: &str,
        slug: &str,
        lang: Lang,
    ) -> Result<Option<Product>, ProductError> {
        let row = sqlx::query(sqlx::AssertSqlSafe(product_select_with_translations(
            "p.seller_id = ? AND p.slug = ?",
            "p.id ASC",
        )))
        .bind(entity_types::PRODUCT)
        .bind(lang.as_str())
        .bind(fields::TITLE)
        .bind(entity_types::PRODUCT)
        .bind(lang.as_str())
        .bind(fields::DESCRIPTION)
        .bind(seller_id)
        .bind(slug)
        .fetch_optional(&self.db.reader)
        .await?;
        row.map(product_from_row).transpose()
    }

    /// Список товаров продавца с пагинацией; опционально отфильтрованный по статусу.
    /// Сортировка — как в индексе `products_by_seller`: по `updated_at DESC`.
    pub async fn list_products_by_seller(
        &self,
        seller_id: &str,
        status: Option<ProductStatus>,
        page: Pagination,
        lang: Lang,
    ) -> Result<Page<Product>, ProductError> {
        let (where_clause, status_str) = match status {
            Some(s) => ("p.seller_id = ? AND p.status = ?", Some(s.as_str())),
            None => ("p.seller_id = ?", None),
        };

        let select_sql = format!(
            "{} LIMIT ? OFFSET ?",
            product_select_with_translations(where_clause, "p.updated_at DESC, p.id ASC")
        );

        let mut select_q = sqlx::query(sqlx::AssertSqlSafe(select_sql))
            .bind(entity_types::PRODUCT)
            .bind(lang.as_str())
            .bind(fields::TITLE)
            .bind(entity_types::PRODUCT)
            .bind(lang.as_str())
            .bind(fields::DESCRIPTION)
            .bind(seller_id);
        if let Some(s) = status_str {
            select_q = select_q.bind(s);
        }
        let rows = select_q
            .bind(page.limit as i64)
            .bind(page.offset as i64)
            .fetch_all(&self.db.reader)
            .await?;
        let items = rows
            .into_iter()
            .map(product_from_row)
            .collect::<Result<Vec<_>, _>>()?;

        let count_sql = format!("SELECT COUNT(*) AS total FROM products p WHERE {where_clause}");
        let mut count_q = sqlx::query(sqlx::AssertSqlSafe(count_sql)).bind(seller_id);
        if let Some(s) = status_str {
            count_q = count_q.bind(s);
        }
        let total: i64 = count_q.fetch_one(&self.db.reader).await?.get("total");

        Ok(Page {
            items,
            total: total as u64,
            page,
        })
    }
}

/// Маппинг перехода статуса в имя доменного события. `None` — переход не порождает событие
/// (например, `draft → draft` после неудачного запроса; на практике `set_status` вызывается
/// только при реальной смене, но функция остаётся тотальной и безопасной).
pub(crate) fn status_transition_event(
    from: ProductStatus,
    to: ProductStatus,
) -> Option<&'static str> {
    use ProductStatus::*;
    match (from, to) {
        (Draft, Published) => Some("ProductPublished"),
        (Published, Archived) => Some("ProductArchived"),
        (Published, Draft) => Some("ProductUnpublished"),
        _ => None,
    }
}

/// JSON-снимок товара для события (`ProductCreated`). Разумный набор полей — без хранения
/// производных/чувствительных данных в payload (design-1a.md §1.2 — outbox payload как контракт).
fn product_snapshot(p: &Product) -> serde_json::Value {
    serde_json::json!({
        "id": p.id,
        "seller_id": p.seller_id,
        "title": p.title,
        "slug": p.slug,
        "description": p.description,
        "price_minor": p.price_minor,
        "currency": p.currency,
        "status": p.status.as_str(),
        "created_at": p.created_at,
        "updated_at": p.updated_at,
    })
}

/// Построить `SELECT` товара с резолвом title/description через `LEFT JOIN translations`.
/// `where_clause`/`order_by` подставляются как часть запроса (доверенные строковые константы —
/// формируются репозиторием, не пользовательским вводом).
fn product_select_with_translations(where_clause: &str, order_by: &str) -> String {
    format!(
        "SELECT p.id, p.seller_id, \
                COALESCE(tt.value, p.title) AS title, \
                p.slug, \
                COALESCE(td.value, p.description) AS description, \
                p.price_minor, p.currency, p.status, p.created_at, p.updated_at \
         FROM products p \
         LEFT JOIN translations tt \
           ON tt.entity_type = ? AND tt.entity_id = p.id AND tt.lang = ? AND tt.field = ? \
         LEFT JOIN translations td \
           ON td.entity_type = ? AND td.entity_id = p.id AND td.lang = ? AND td.field = ? \
         WHERE {where_clause} ORDER BY {order_by}"
    )
}

/// Маппинг строки БД → [`Product`].
pub(crate) fn product_from_row(row: SqliteRow) -> Result<Product, ProductError> {
    let status_raw: String = row.get("status");
    let status = ProductStatus::parse(&status_raw).ok_or_else(|| ProductError::InvalidEnum {
        field: "status",
        value: status_raw.clone(),
    })?;
    Ok(Product {
        id: row.get("id"),
        seller_id: row.get("seller_id"),
        title: row.get("title"),
        slug: row.get("slug"),
        description: row.get("description"),
        price_minor: row.get("price_minor"),
        currency: row.get("currency"),
        status,
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}
