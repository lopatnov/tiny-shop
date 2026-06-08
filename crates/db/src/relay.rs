//! In-process relay (см. `design-1a.md` §1.3).
//!
//! Фоновая задача опрашивает outbox каждого источника и доставляет события подписчикам
//! ВНУТРИ процесса (без брокера — мы lightweight). At-least-once, упорядоченно по `id`.
//! Идемпотентность — на стороне потребителя (см. [`crate::inbox`]).

use std::time::Duration;

use sqlx::SqlitePool;

use crate::{DbError, outbox};
use shared::DomainEvent;

/// Ошибка доставки события потребителю. Возврат ошибки = «не помечать published, повторить позже».
#[derive(Debug, thiserror::Error)]
#[error("dispatch error: {0}")]
pub struct DispatchError(pub String);

/// Потребитель событий. Нативный async-fn-in-trait (ADR O2 — без `async-trait`).
pub trait Dispatcher {
    fn dispatch(
        &self,
        source: &str,
        event: &DomainEvent,
    ) -> impl std::future::Future<Output = Result<(), DispatchError>> + Send;
}

/// Источник для relay: имя контекста + пул для чтения outbox и пометки published.
pub struct RelaySource {
    pub name: String,
    pub pool: SqlitePool,
}

/// Один проход relay по всем источникам. Возвращает число доставленных событий.
/// При ошибке доставки события прекращает обработку этого источника (сохраняет порядок,
/// не теряет последующие — они уйдут на следующем проходе).
pub async fn relay_tick<D: Dispatcher>(
    sources: &[RelaySource],
    dispatcher: &D,
    batch: u32,
) -> Result<usize, DbError> {
    let mut total = 0;
    for src in sources {
        let events = outbox::fetch_unpublished(&src.pool, batch).await?;
        let mut delivered = Vec::new();
        for ev in &events {
            match dispatcher.dispatch(&src.name, ev).await {
                Ok(()) => delivered.push(ev.id),
                Err(e) => {
                    tracing::warn!(source = %src.name, id = ev.id, error = %e,
                        "relay dispatch failed; will retry next tick");
                    break;
                }
            }
        }
        if !delivered.is_empty() {
            outbox::mark_published(&src.pool, &delivered).await?;
            total += delivered.len();
        }
    }
    Ok(total)
}

/// Бесконечный цикл relay (для фоновой tokio-задачи). Ошибки прохода логируются, не фатальны.
pub async fn run_relay<D: Dispatcher>(sources: Vec<RelaySource>, dispatcher: D, poll: Duration) {
    loop {
        if let Err(e) = relay_tick(&sources, &dispatcher, 100).await {
            tracing::error!(error = %e, "relay tick failed");
        }
        tokio::time::sleep(poll).await;
    }
}

/// Дефолтный потребитель-заглушка: только логирует. Реальные проекции — T1a-5.
pub struct LoggingDispatcher;

impl Dispatcher for LoggingDispatcher {
    async fn dispatch(&self, source: &str, event: &DomainEvent) -> Result<(), DispatchError> {
        tracing::debug!(source, event_type = %event.event_type, id = event.id, "relay dispatch");
        Ok(())
    }
}
