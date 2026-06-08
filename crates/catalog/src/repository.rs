//! Репозиторий таксономии каталога поверх `db::ContextDb` (T1a-3).
//!
//! ## Решение по форме репозитория
//! Обобщённый `db::repository::Repository<T, Id>` здесь не используем: у таксономии четыре
//! связанные сущности (категория/атрибут/опция/фильтр) с разной семантикой выборок (дерево по
//! `parent_id`/`path`, списки по `category_id`/`attribute_id`, уникальность пар), и натягивание
//! единого `get/list/save/delete` на всё это раздуло бы код без пользы — выбран прагматичный
//! набор операций под нужды T1a-3 (создание + точечное и древовидное чтение + переводы).
//! Запись — через `writer` (с транзакциями там, где это даёт атомарность), чтение — через
//! `reader`. ID — строки (ULID), генерируются вызывающей стороной (как `outbox.aggregate_id`).
//!
//! ## i18n (ADR O5)
//! Канон названия — поле `name`/`value` (язык `uk`). Перевод на другой язык, если он есть,
//! лежит в `translations` и резолвится через `COALESCE(перевод, канон)`.
//!
//! Все пути чтения всегда делают `LEFT JOIN translations` — единая ветка для любого [`Lang`],
//! без `match lang { Uk => ..., _ => ... }` на каждый запрос. Для [`Lang::Uk`] перевода в таблице
//! нет (канон уже на этом языке), `JOIN` просто не находит строку и `COALESCE` возвращает канон —
//! результат идентичен «короткому пути», но без удвоения SQL в каждом методе (см. также §«Простота»
//! в `.claude/rules/index.md`). Таксономия — низкочастотные admin-управляемые таблицы (категории/
//! атрибуты — десятки-сотни строк), поэтому цена лишнего `LEFT JOIN` пренебрежимо мала; если
//! профилирование в будущем покажет иное — переоценить вместе с `architect`.

use sqlx::Row;

use db::ContextDb;

use crate::taxonomy::{Attribute, AttributeOption, Category, DataType, Filter, FilterType, Lang};

/// Значения `translations.entity_type` (CHECK в миграции `0002_taxonomy.sql`).
mod entity_types {
    pub const CATEGORY: &str = "category";
    pub const ATTRIBUTE: &str = "attribute";
    pub const ATTRIBUTE_OPTION: &str = "attribute_option";
}

/// Значения `translations.field` (CHECK в миграции `0002_taxonomy.sql`).
mod fields {
    pub const NAME: &str = "name";
    pub const VALUE: &str = "value";
}

/// Ошибки репозитория таксономии.
#[derive(Debug, thiserror::Error)]
pub enum TaxonomyError {
    #[error("db: {0}")]
    Db(#[from] sqlx::Error),
    /// Колонка БД содержит значение вне ожидаемого набора (CHECK должен это предотвращать,
    /// но парсер обязан вернуть ошибку, а не запаниковать).
    #[error("invalid {field} in row: {value}")]
    InvalidEnum { field: &'static str, value: String },
}

/// Репозиторий чтения/записи таксономии каталога (категории, атрибуты, опции, фильтры, переводы).
#[derive(Clone)]
pub struct TaxonomyRepo {
    db: ContextDb,
}

impl TaxonomyRepo {
    pub fn new(db: ContextDb) -> Self {
        Self { db }
    }

    // -----------------------------------------------------------------
    // Категории
    // -----------------------------------------------------------------

    /// Создать категорию. Канон `name` хранится напрямую (язык по умолчанию — `uk`).
    pub async fn create_category(&self, c: &Category) -> Result<(), TaxonomyError> {
        sqlx::query(
            "INSERT INTO categories (id, parent_id, name, slug, path, position) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&c.id)
        .bind(&c.parent_id)
        .bind(&c.name)
        .bind(&c.slug)
        .bind(&c.path)
        .bind(c.position)
        .execute(&self.db.writer)
        .await?;
        Ok(())
    }

    /// Прочитать категорию по id, с резолвом названия на запрошенный язык.
    pub async fn get_category(
        &self,
        id: &str,
        lang: Lang,
    ) -> Result<Option<Category>, TaxonomyError> {
        let row = sqlx::query(
            "SELECT c.id, c.parent_id, COALESCE(t.value, c.name) AS name, c.slug, c.path, c.position \
             FROM categories c \
             LEFT JOIN translations t \
               ON t.entity_type = ? AND t.entity_id = c.id \
              AND t.lang = ? AND t.field = ? \
             WHERE c.id = ?",
        )
        .bind(entity_types::CATEGORY)
        .bind(lang.as_str())
        .bind(fields::NAME)
        .bind(id)
        .fetch_optional(&self.db.reader)
        .await?;
        row.map(category_from_row).transpose()
    }

    /// Прочитать категорию по materialized path (уникален глобально).
    pub async fn get_category_by_path(
        &self,
        path: &str,
        lang: Lang,
    ) -> Result<Option<Category>, TaxonomyError> {
        let row = sqlx::query(
            "SELECT c.id, c.parent_id, COALESCE(t.value, c.name) AS name, c.slug, c.path, c.position \
             FROM categories c \
             LEFT JOIN translations t \
               ON t.entity_type = ? AND t.entity_id = c.id \
              AND t.lang = ? AND t.field = ? \
             WHERE c.path = ?",
        )
        .bind(entity_types::CATEGORY)
        .bind(lang.as_str())
        .bind(fields::NAME)
        .bind(path)
        .fetch_optional(&self.db.reader)
        .await?;
        row.map(category_from_row).transpose()
    }

    /// Дочерние категории узла (для построения дерева). `parent_id = NULL` → корни.
    pub async fn list_categories_by_parent(
        &self,
        parent_id: Option<&str>,
        lang: Lang,
    ) -> Result<Vec<Category>, TaxonomyError> {
        let rows = match parent_id {
            Some(pid) => {
                sqlx::query(
                    "SELECT c.id, c.parent_id, COALESCE(t.value, c.name) AS name, c.slug, c.path, c.position \
                     FROM categories c \
                     LEFT JOIN translations t \
                       ON t.entity_type = ? AND t.entity_id = c.id \
                      AND t.lang = ? AND t.field = ? \
                     WHERE c.parent_id = ? ORDER BY c.position ASC, c.id ASC",
                )
                .bind(entity_types::CATEGORY)
                .bind(lang.as_str())
                .bind(fields::NAME)
                .bind(pid)
                .fetch_all(&self.db.reader)
                .await?
            }
            None => {
                sqlx::query(
                    "SELECT c.id, c.parent_id, COALESCE(t.value, c.name) AS name, c.slug, c.path, c.position \
                     FROM categories c \
                     LEFT JOIN translations t \
                       ON t.entity_type = ? AND t.entity_id = c.id \
                      AND t.lang = ? AND t.field = ? \
                     WHERE c.parent_id IS NULL ORDER BY c.position ASC, c.id ASC",
                )
                .bind(entity_types::CATEGORY)
                .bind(lang.as_str())
                .bind(fields::NAME)
                .fetch_all(&self.db.reader)
                .await?
            }
        };
        rows.into_iter().map(category_from_row).collect()
    }

    // -----------------------------------------------------------------
    // Атрибуты
    // -----------------------------------------------------------------

    /// Создать атрибут категории.
    pub async fn create_attribute(&self, a: &Attribute) -> Result<(), TaxonomyError> {
        sqlx::query(
            "INSERT INTO attributes (id, category_id, name, data_type, unit, is_required, position) \
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&a.id)
        .bind(&a.category_id)
        .bind(&a.name)
        .bind(a.data_type.as_str())
        .bind(&a.unit)
        .bind(a.is_required as i64)
        .bind(a.position)
        .execute(&self.db.writer)
        .await?;
        Ok(())
    }

    /// Атрибуты категории по порядку отображения.
    pub async fn list_attributes_by_category(
        &self,
        category_id: &str,
        lang: Lang,
    ) -> Result<Vec<Attribute>, TaxonomyError> {
        let rows = sqlx::query(
            "SELECT a.id, a.category_id, COALESCE(t.value, a.name) AS name, a.data_type, a.unit, \
                    a.is_required, a.position \
             FROM attributes a \
             LEFT JOIN translations t \
               ON t.entity_type = ? AND t.entity_id = a.id \
              AND t.lang = ? AND t.field = ? \
             WHERE a.category_id = ? ORDER BY a.position ASC, a.id ASC",
        )
        .bind(entity_types::ATTRIBUTE)
        .bind(lang.as_str())
        .bind(fields::NAME)
        .bind(category_id)
        .fetch_all(&self.db.reader)
        .await?;
        rows.into_iter().map(attribute_from_row).collect()
    }

    // -----------------------------------------------------------------
    // Опции атрибутов (enum)
    // -----------------------------------------------------------------

    /// Создать допустимое значение enum-атрибута.
    pub async fn create_attribute_option(&self, o: &AttributeOption) -> Result<(), TaxonomyError> {
        sqlx::query(
            "INSERT INTO attribute_options (id, attribute_id, value, position) VALUES (?, ?, ?, ?)",
        )
        .bind(&o.id)
        .bind(&o.attribute_id)
        .bind(&o.value)
        .bind(o.position)
        .execute(&self.db.writer)
        .await?;
        Ok(())
    }

    /// Опции атрибута по порядку отображения.
    pub async fn list_attribute_options(
        &self,
        attribute_id: &str,
        lang: Lang,
    ) -> Result<Vec<AttributeOption>, TaxonomyError> {
        let rows = sqlx::query(
            "SELECT o.id, o.attribute_id, COALESCE(t.value, o.value) AS value, o.position \
             FROM attribute_options o \
             LEFT JOIN translations t \
               ON t.entity_type = ? AND t.entity_id = o.id \
              AND t.lang = ? AND t.field = ? \
             WHERE o.attribute_id = ? ORDER BY o.position ASC, o.id ASC",
        )
        .bind(entity_types::ATTRIBUTE_OPTION)
        .bind(lang.as_str())
        .bind(fields::VALUE)
        .bind(attribute_id)
        .fetch_all(&self.db.reader)
        .await?;
        rows.into_iter().map(attribute_option_from_row).collect()
    }

    // -----------------------------------------------------------------
    // Фильтры
    // -----------------------------------------------------------------

    /// Привязать атрибут к категории как фильтр. `UNIQUE(category_id, attribute_id)` —
    /// нарушение возвращается как `TaxonomyError::Db` (caller распознаёт по `sqlx::Error`).
    pub async fn create_filter(&self, f: &Filter) -> Result<(), TaxonomyError> {
        sqlx::query(
            "INSERT INTO filters (id, category_id, attribute_id, filter_type, position) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&f.id)
        .bind(&f.category_id)
        .bind(&f.attribute_id)
        .bind(f.filter_type.as_str())
        .bind(f.position)
        .execute(&self.db.writer)
        .await?;
        Ok(())
    }

    /// Фильтры категории по порядку отображения.
    pub async fn list_filters_by_category(
        &self,
        category_id: &str,
    ) -> Result<Vec<Filter>, TaxonomyError> {
        let rows = sqlx::query(
            "SELECT id, category_id, attribute_id, filter_type, position \
             FROM filters WHERE category_id = ? ORDER BY position ASC, id ASC",
        )
        .bind(category_id)
        .fetch_all(&self.db.reader)
        .await?;
        rows.into_iter().map(filter_from_row).collect()
    }

    // -----------------------------------------------------------------
    // Переводы (i18n, ADR O5)
    // -----------------------------------------------------------------

    /// Записать (или заменить) перевод поля сущности на язык `lang`.
    /// `lang = Uk` не имеет смысла как override (канон уже на `uk`), но не запрещается на
    /// уровне репозитория — это вопрос вызывающей стороны/валидации выше.
    pub async fn set_translation(
        &self,
        entity_type: &str,
        entity_id: &str,
        lang: Lang,
        field: &str,
        value: &str,
    ) -> Result<(), TaxonomyError> {
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
        .execute(&self.db.writer)
        .await?;
        Ok(())
    }

    /// Резолв перевода поля сущности: `COALESCE(override, канон)`. `canonical` — значение
    /// канонического поля (`name`/`value`) сущности на языке по умолчанию (`uk`).
    pub async fn resolve_translation(
        &self,
        entity_type: &str,
        entity_id: &str,
        lang: Lang,
        field: &str,
        canonical: &str,
    ) -> Result<String, TaxonomyError> {
        let row = sqlx::query(
            "SELECT value FROM translations \
             WHERE entity_type = ? AND entity_id = ? AND lang = ? AND field = ?",
        )
        .bind(entity_type)
        .bind(entity_id)
        .bind(lang.as_str())
        .bind(field)
        .fetch_optional(&self.db.reader)
        .await?;
        Ok(row
            .map(|r| r.get::<String, _>("value"))
            .unwrap_or_else(|| canonical.to_string()))
    }
}

// -----------------------------------------------------------------
// Маппинг строк БД → доменные типы
// -----------------------------------------------------------------

fn category_from_row(row: sqlx::sqlite::SqliteRow) -> Result<Category, TaxonomyError> {
    Ok(Category {
        id: row.get("id"),
        parent_id: row.get("parent_id"),
        name: row.get("name"),
        slug: row.get("slug"),
        path: row.get("path"),
        position: row.get("position"),
    })
}

fn attribute_from_row(row: sqlx::sqlite::SqliteRow) -> Result<Attribute, TaxonomyError> {
    let data_type_raw: String = row.get("data_type");
    let data_type = DataType::parse(&data_type_raw).ok_or_else(|| TaxonomyError::InvalidEnum {
        field: "data_type",
        value: data_type_raw.clone(),
    })?;
    let is_required: i64 = row.get("is_required");
    Ok(Attribute {
        id: row.get("id"),
        category_id: row.get("category_id"),
        name: row.get("name"),
        data_type,
        unit: row.get("unit"),
        is_required: is_required != 0,
        position: row.get("position"),
    })
}

fn attribute_option_from_row(
    row: sqlx::sqlite::SqliteRow,
) -> Result<AttributeOption, TaxonomyError> {
    Ok(AttributeOption {
        id: row.get("id"),
        attribute_id: row.get("attribute_id"),
        value: row.get("value"),
        position: row.get("position"),
    })
}

fn filter_from_row(row: sqlx::sqlite::SqliteRow) -> Result<Filter, TaxonomyError> {
    let filter_type_raw: String = row.get("filter_type");
    let filter_type =
        FilterType::parse(&filter_type_raw).ok_or_else(|| TaxonomyError::InvalidEnum {
            field: "filter_type",
            value: filter_type_raw.clone(),
        })?;
    Ok(Filter {
        id: row.get("id"),
        category_id: row.get("category_id"),
        attribute_id: row.get("attribute_id"),
        filter_type,
        position: row.get("position"),
    })
}
