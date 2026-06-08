//! `shared` — типы-ядро, общие для всех контекстов.
//!
//! В 1a здесь живёт конверт доменного события (для transactional outbox / inbox)
//! и мелкие утилиты. Расширяется по мере фаз; держим минимальным (Простота).

use serde::{Deserialize, Serialize};

/// Конверт доменного события — как оно лежит в `outbox` и доставляется relay'ем.
///
/// `id` и `created_at` назначаются при записи в outbox; до записи используется [`NewEvent`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainEvent {
    pub id: i64,
    /// Тип агрегата-источника: `"product"`, `"category"`, `"account"`, …
    pub aggregate: String,
    pub aggregate_id: String,
    /// Имя события: `"ProductPublished"`, `"CategoryRenamed"`, …
    pub event_type: String,
    /// Полезная нагрузка (JSON).
    pub payload: serde_json::Value,
    /// Unix-время создания, миллисекунды.
    pub created_at: i64,
}

/// Новое событие до записи в outbox (id/created_at назначит `OutboxStore`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewEvent {
    pub aggregate: String,
    pub aggregate_id: String,
    pub event_type: String,
    pub payload: serde_json::Value,
}

impl NewEvent {
    pub fn new(
        aggregate: impl Into<String>,
        aggregate_id: impl Into<String>,
        event_type: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            aggregate: aggregate.into(),
            aggregate_id: aggregate_id.into(),
            event_type: event_type.into(),
            payload,
        }
    }
}

/// Текущее unix-время в миллисекундах.
pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Пагинация и страница результатов (общие для listing/поиска/Repository).
// ---------------------------------------------------------------------------

/// Параметры пагинации (вход запроса).
///
/// `limit` ограничен сверху [`Pagination::MAX_LIMIT`] — используй [`Pagination::clamped`] для
/// значений из недоверенного ввода (запрос не должен мочь затребовать слишком большую страницу).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pagination {
    pub offset: u32,
    pub limit: u32,
}

impl Pagination {
    /// Верхняя граница размера страницы (защита от чрезмерных выборок).
    pub const MAX_LIMIT: u32 = 100;
    /// Размер страницы по умолчанию.
    pub const DEFAULT_LIMIT: u32 = 24;

    /// Создать, зажав `limit` в диапазон `1..=MAX_LIMIT` (для недоверенного ввода).
    pub fn clamped(offset: u32, limit: u32) -> Self {
        Self {
            offset,
            limit: limit.clamp(1, Self::MAX_LIMIT),
        }
    }
}

impl Default for Pagination {
    fn default() -> Self {
        Self {
            offset: 0,
            limit: Self::DEFAULT_LIMIT,
        }
    }
}

/// Страница результатов произвольного типа.
#[derive(Debug, Clone, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub page: Pagination,
}

/// Тип значения атрибута (типизированный EAV, design-1a.md §2/§3).
///
/// Общий для контекстов `catalog` и `product` — это техническое value-type-перечисление
/// (не доменная сущность контекста), поэтому его совместное использование не нарушает
/// изоляцию bounded contexts. Жил отдельными копиями в обоих крейтах — Sonar справедливо
/// отметил буквальное дублирование кода; вынесен сюда (Простота, DRY).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataType {
    String,
    Number,
    Enum,
    Bool,
}

impl DataType {
    pub fn as_str(self) -> &'static str {
        match self {
            DataType::String => "string",
            DataType::Number => "number",
            DataType::Enum => "enum",
            DataType::Bool => "bool",
        }
    }

    /// Разбор значения колонки `data_type`. `None` — неизвестное значение (БД хранит
    /// каноничные строки под CHECK-ограничением, но парсер не должен паниковать).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "string" => Some(DataType::String),
            "number" => Some(DataType::Number),
            "enum" => Some(DataType::Enum),
            "bool" => Some(DataType::Bool),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Порт Scanner — опциональный антивирус для загрузок (design-1a.md §4).
// По умолчанию NoopScanner (всегда Clean). Реальные адаптеры — 1c+.
// ---------------------------------------------------------------------------

/// Ссылка на проверяемый ассет (файл/объект хранилища).
#[derive(Debug, Clone)]
pub struct AssetRef {
    pub path: String,
}

/// Вердикт проверки.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Clean,
    Infected(String),
    Skipped,
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("scanner backend error: {0}")]
    Backend(String),
}

/// Порт антивируса. Нативный async-fn-in-trait (без `async-trait`).
pub trait Scanner {
    fn scan(
        &self,
        asset: &AssetRef,
    ) -> impl std::future::Future<Output = Result<Verdict, ScanError>> + Send;
}

/// Заглушка по умолчанию: скан выключен → всегда `Clean`.
pub struct NoopScanner;

impl Scanner for NoopScanner {
    async fn scan(&self, _asset: &AssetRef) -> Result<Verdict, ScanError> {
        Ok(Verdict::Clean)
    }
}
