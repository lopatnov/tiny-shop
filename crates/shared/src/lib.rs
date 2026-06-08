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
