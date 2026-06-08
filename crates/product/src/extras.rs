//! Операции `ProductRepo` над дочерними сущностями товара (T1a-4): цифровая конфигурация,
//! варианты, медиа, значения атрибутов, переводы.
//!
//! Вынесены из `repository.rs` ради размера файла (conventions.md «не более ~1000 строк») —
//! единый тип `ProductRepo`, разделённая по ответственности реализация. Те же правила, что и
//! в основном файле: запись — `writer.begin()` + `outbox::enqueue(&mut *tx, ...)` в одной
//! транзакции (эмитят `ProductUpdated` — изменение дочерней сущности есть изменение товара),
//! чтение — через `reader` с `LEFT JOIN translations ... COALESCE` для i18n-резолва.

use sqlx::Row;
use sqlx::sqlite::SqliteRow;

use db::outbox;
use shared::{NewEvent, now_ms};

use crate::product::{
    DataType, DeliveryKind, DigitalConfig, DigitalVariant, Lang, LicenseKind, MediaKind,
    ProductAttributeValue, ProductMedia,
};
use crate::repository::{AGGREGATE, ProductError, ProductRepo, entity_types, fields};

impl ProductRepo {
    // -----------------------------------------------------------------
    // Цифровая конфигурация (1:1 с товаром)
    // -----------------------------------------------------------------

    /// Создать или заменить цифровую конфигурацию товара. Эмитит `ProductUpdated`.
    pub async fn upsert_digital_config(&self, c: &DigitalConfig) -> Result<(), ProductError> {
        let mut tx = self.db.writer.begin().await?;
        sqlx::query(
            "INSERT INTO digital_config (product_id, delivery_kind, license_kind, notes) \
             VALUES (?, ?, ?, ?) \
             ON CONFLICT (product_id) DO UPDATE SET \
               delivery_kind = excluded.delivery_kind, \
               license_kind = excluded.license_kind, \
               notes = excluded.notes",
        )
        .bind(&c.product_id)
        .bind(c.delivery_kind.as_str())
        .bind(c.license_kind.map(LicenseKind::as_str))
        .bind(&c.notes)
        .execute(&mut *tx)
        .await?;

        enqueue_product_updated(&mut tx, &c.product_id, "digital_config").await?;
        tx.commit().await?;
        Ok(())
    }

    /// Прочитать цифровую конфигурацию товара (если задана).
    pub async fn get_digital_config(
        &self,
        product_id: &str,
    ) -> Result<Option<DigitalConfig>, ProductError> {
        let row = sqlx::query(
            "SELECT product_id, delivery_kind, license_kind, notes \
             FROM digital_config WHERE product_id = ?",
        )
        .bind(product_id)
        .fetch_optional(&self.db.reader)
        .await?;
        row.map(digital_config_from_row).transpose()
    }

    // -----------------------------------------------------------------
    // Варианты цифрового товара
    // -----------------------------------------------------------------

    /// Добавить вариант. Эмитит `ProductUpdated`.
    pub async fn add_variant(&self, v: &DigitalVariant) -> Result<(), ProductError> {
        let mut tx = self.db.writer.begin().await?;
        sqlx::query(
            "INSERT INTO digital_variant (id, product_id, label, format, price_delta_minor, position) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&v.id)
        .bind(&v.product_id)
        .bind(&v.label)
        .bind(&v.format)
        .bind(v.price_delta_minor)
        .bind(v.position)
        .execute(&mut *tx)
        .await?;

        enqueue_product_updated(&mut tx, &v.product_id, "variant_added").await?;
        tx.commit().await?;
        Ok(())
    }

    /// Обновить вариант (label/format/цена-дельта/позиция). Эмитит `ProductUpdated`.
    pub async fn update_variant(&self, v: &DigitalVariant) -> Result<(), ProductError> {
        let mut tx = self.db.writer.begin().await?;
        sqlx::query(
            "UPDATE digital_variant \
             SET label = ?, format = ?, price_delta_minor = ?, position = ? \
             WHERE id = ?",
        )
        .bind(&v.label)
        .bind(&v.format)
        .bind(v.price_delta_minor)
        .bind(v.position)
        .bind(&v.id)
        .execute(&mut *tx)
        .await?;

        enqueue_product_updated(&mut tx, &v.product_id, "variant_updated").await?;
        tx.commit().await?;
        Ok(())
    }

    /// Удалить вариант. Также чистит его переводы (`translations` — полиморфная, без FK).
    /// Эмитит `ProductUpdated`.
    pub async fn remove_variant(
        &self,
        product_id: &str,
        variant_id: &str,
    ) -> Result<(), ProductError> {
        let mut tx = self.db.writer.begin().await?;
        sqlx::query("DELETE FROM translations WHERE entity_type = ? AND entity_id = ?")
            .bind(entity_types::DIGITAL_VARIANT)
            .bind(variant_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM digital_variant WHERE id = ? AND product_id = ?")
            .bind(variant_id)
            .bind(product_id)
            .execute(&mut *tx)
            .await?;

        enqueue_product_updated(&mut tx, product_id, "variant_removed").await?;
        tx.commit().await?;
        Ok(())
    }

    /// Варианты товара по порядку отображения, с резолвом `label` на запрошенный язык.
    pub async fn list_variants(
        &self,
        product_id: &str,
        lang: Lang,
    ) -> Result<Vec<DigitalVariant>, ProductError> {
        let rows = sqlx::query(
            "SELECT v.id, v.product_id, COALESCE(t.value, v.label) AS label, v.format, \
                    v.price_delta_minor, v.position \
             FROM digital_variant v \
             LEFT JOIN translations t \
               ON t.entity_type = ? AND t.entity_id = v.id AND t.lang = ? AND t.field = ? \
             WHERE v.product_id = ? ORDER BY v.position ASC, v.id ASC",
        )
        .bind(entity_types::DIGITAL_VARIANT)
        .bind(lang.as_str())
        .bind(fields::LABEL)
        .bind(product_id)
        .fetch_all(&self.db.reader)
        .await?;
        rows.into_iter().map(digital_variant_from_row).collect()
    }

    // -----------------------------------------------------------------
    // Медиа
    // -----------------------------------------------------------------

    /// Добавить медиа-вложение. Эмитит `ProductUpdated`.
    pub async fn add_media(&self, m: &ProductMedia) -> Result<(), ProductError> {
        let mut tx = self.db.writer.begin().await?;
        sqlx::query(
            "INSERT INTO product_media (id, product_id, kind, url, position) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&m.id)
        .bind(&m.product_id)
        .bind(m.kind.as_str())
        .bind(&m.url)
        .bind(m.position)
        .execute(&mut *tx)
        .await?;

        enqueue_product_updated(&mut tx, &m.product_id, "media_added").await?;
        tx.commit().await?;
        Ok(())
    }

    /// Удалить медиа-вложение. Эмитит `ProductUpdated`.
    pub async fn remove_media(&self, product_id: &str, media_id: &str) -> Result<(), ProductError> {
        let mut tx = self.db.writer.begin().await?;
        sqlx::query("DELETE FROM product_media WHERE id = ? AND product_id = ?")
            .bind(media_id)
            .bind(product_id)
            .execute(&mut *tx)
            .await?;

        enqueue_product_updated(&mut tx, product_id, "media_removed").await?;
        tx.commit().await?;
        Ok(())
    }

    /// Медиа товара по порядку отображения.
    pub async fn list_media(&self, product_id: &str) -> Result<Vec<ProductMedia>, ProductError> {
        let rows = sqlx::query(
            "SELECT id, product_id, kind, url, position FROM product_media \
             WHERE product_id = ? ORDER BY position ASC, id ASC",
        )
        .bind(product_id)
        .fetch_all(&self.db.reader)
        .await?;
        rows.into_iter().map(product_media_from_row).collect()
    }

    // -----------------------------------------------------------------
    // Значения атрибутов (типизированный EAV)
    // -----------------------------------------------------------------

    /// Установить (или заменить) значение атрибута товара. Эмитит `ProductUpdated`.
    pub async fn set_attribute_value(&self, v: &ProductAttributeValue) -> Result<(), ProductError> {
        let mut tx = self.db.writer.begin().await?;
        sqlx::query(
            "INSERT INTO product_attribute_values \
                (product_id, attribute_id, data_type, val_text, val_num) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT (product_id, attribute_id) DO UPDATE SET \
               data_type = excluded.data_type, \
               val_text = excluded.val_text, \
               val_num = excluded.val_num",
        )
        .bind(&v.product_id)
        .bind(&v.attribute_id)
        .bind(v.data_type.as_str())
        .bind(&v.val_text)
        .bind(v.val_num)
        .execute(&mut *tx)
        .await?;

        enqueue_product_updated(&mut tx, &v.product_id, "attribute_value_set").await?;
        tx.commit().await?;
        Ok(())
    }

    /// Снять значение атрибута товара. Эмитит `ProductUpdated`.
    pub async fn clear_attribute_value(
        &self,
        product_id: &str,
        attribute_id: &str,
    ) -> Result<(), ProductError> {
        let mut tx = self.db.writer.begin().await?;
        sqlx::query(
            "DELETE FROM product_attribute_values WHERE product_id = ? AND attribute_id = ?",
        )
        .bind(product_id)
        .bind(attribute_id)
        .execute(&mut *tx)
        .await?;

        enqueue_product_updated(&mut tx, product_id, "attribute_value_cleared").await?;
        tx.commit().await?;
        Ok(())
    }

    /// Значения атрибутов товара (без определённого порядка — порядок задаёт `catalog::Attribute`).
    pub async fn list_attribute_values(
        &self,
        product_id: &str,
    ) -> Result<Vec<ProductAttributeValue>, ProductError> {
        let rows = sqlx::query(
            "SELECT product_id, attribute_id, data_type, val_text, val_num \
             FROM product_attribute_values WHERE product_id = ? ORDER BY attribute_id ASC",
        )
        .bind(product_id)
        .fetch_all(&self.db.reader)
        .await?;
        rows.into_iter().map(attribute_value_from_row).collect()
    }

    // -----------------------------------------------------------------
    // Переводы (i18n, ADR O5/T1a-4)
    // -----------------------------------------------------------------

    /// Записать (или заменить) перевод поля сущности товара (`product`/`digital_variant`)
    /// на язык `lang`. Эмитит `ProductUpdated` для затронутого товара.
    ///
    /// `lang = Uk` не имеет смысла как override (канон уже на `uk`), но не запрещается на
    /// уровне репозитория — см. аналогичное замечание в `catalog::TaxonomyRepo::set_translation`.
    pub async fn set_translation(
        &self,
        product_id: &str,
        entity_type: &str,
        entity_id: &str,
        lang: Lang,
        field: &str,
        value: &str,
    ) -> Result<(), ProductError> {
        let mut tx = self.db.writer.begin().await?;
        sqlx::query(
            "INSERT INTO translations (entity_type, entity_id, lang, field, value) \
             VALUES (?, ?, ?, ?, ?) \
             ON CONFLICT (entity_type, entity_id, lang, field) DO UPDATE SET value = excluded.value",
        )
        .bind(entity_type)
        .bind(entity_id)
        .bind(lang.as_str())
        .bind(field)
        .bind(value)
        .execute(&mut *tx)
        .await?;

        enqueue_product_updated(&mut tx, product_id, "translation_set").await?;
        tx.commit().await?;
        Ok(())
    }
}

/// Записать `ProductUpdated` с указанием причины (`reason`) — лёгкий снимок для consumers,
/// которым достаточно знать "товар изменился, перечитай агрегат" (design-1a.md §1.2: payload —
/// разумный снимок, не полный агрегат).
async fn enqueue_product_updated(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    product_id: &str,
    reason: &'static str,
) -> Result<(), ProductError> {
    outbox::enqueue(
        &mut **tx,
        &NewEvent::new(
            AGGREGATE,
            product_id,
            "ProductUpdated",
            serde_json::json!({ "id": product_id, "reason": reason, "updated_at": now_ms() }),
        ),
    )
    .await?;
    Ok(())
}

// -----------------------------------------------------------------
// Маппинг строк БД → доменные типы
// -----------------------------------------------------------------

fn digital_config_from_row(row: SqliteRow) -> Result<DigitalConfig, ProductError> {
    let delivery_raw: String = row.get("delivery_kind");
    let delivery_kind =
        DeliveryKind::parse(&delivery_raw).ok_or_else(|| ProductError::InvalidEnum {
            field: "delivery_kind",
            value: delivery_raw.clone(),
        })?;
    let license_kind = row
        .get::<Option<String>, _>("license_kind")
        .map(|raw| {
            LicenseKind::parse(&raw).ok_or_else(|| ProductError::InvalidEnum {
                field: "license_kind",
                value: raw,
            })
        })
        .transpose()?;
    Ok(DigitalConfig {
        product_id: row.get("product_id"),
        delivery_kind,
        license_kind,
        notes: row.get("notes"),
    })
}

fn digital_variant_from_row(row: SqliteRow) -> Result<DigitalVariant, ProductError> {
    Ok(DigitalVariant {
        id: row.get("id"),
        product_id: row.get("product_id"),
        label: row.get("label"),
        format: row.get("format"),
        price_delta_minor: row.get("price_delta_minor"),
        position: row.get("position"),
    })
}

fn product_media_from_row(row: SqliteRow) -> Result<ProductMedia, ProductError> {
    let kind_raw: String = row.get("kind");
    let kind = MediaKind::parse(&kind_raw).ok_or_else(|| ProductError::InvalidEnum {
        field: "kind",
        value: kind_raw.clone(),
    })?;
    Ok(ProductMedia {
        id: row.get("id"),
        product_id: row.get("product_id"),
        kind,
        url: row.get("url"),
        position: row.get("position"),
    })
}

fn attribute_value_from_row(row: SqliteRow) -> Result<ProductAttributeValue, ProductError> {
    let data_type_raw: String = row.get("data_type");
    let data_type = DataType::parse(&data_type_raw).ok_or_else(|| ProductError::InvalidEnum {
        field: "data_type",
        value: data_type_raw.clone(),
    })?;
    Ok(ProductAttributeValue {
        product_id: row.get("product_id"),
        attribute_id: row.get("attribute_id"),
        data_type,
        val_text: row.get("val_text"),
        val_num: row.get("val_num"),
    })
}
