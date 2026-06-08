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

/// Сколько раз пытаемся доставить событие, прежде чем dead-letter (защита от «ядовитой пилюли»).
pub const MAX_DISPATCH_ATTEMPTS: i64 = 10;

/// Один проход relay по всем источникам. Возвращает число доставленных событий.
///
/// Обработка ошибок доставки:
/// - **транзиентная** (attempts < [`MAX_DISPATCH_ATTEMPTS`]): фиксируем попытку и прекращаем
///   обработку этого источника на проходе — сохраняем порядок, повторим на следующем тике;
/// - **перманентная** («ядовитая пилюля», attempts ≥ лимита): логируем критично и **dead-letter**
///   (помечаем published с сохранённым `last_error`), чтобы не блокировать очередь навсегда
///   (head-of-line blocking).
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
                    let attempts = outbox::record_failure(&src.pool, ev.id, &e.to_string()).await?;
                    if attempts >= MAX_DISPATCH_ATTEMPTS {
                        tracing::error!(source = %src.name, id = ev.id, attempts, error = %e,
                            "event dead-lettered after max attempts; skipping to unblock relay");
                        outbox::mark_published(&src.pool, &[ev.id]).await?;
                        // dead-letter: продолжаем со следующим событием, очередь не блокируется
                    } else {
                        tracing::warn!(source = %src.name, id = ev.id, attempts, error = %e,
                            "relay dispatch failed; will retry next tick");
                        break; // транзиентная ошибка: сохраняем порядок, повтор на следующем тике
                    }
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

/// Бесконечный цикл relay (для фоновой tokio-задачи). Спим только в простое (нет событий),
/// при наличии работы — `yield_now`, чтобы быстро разобрать накопившуюся очередь. Ошибки
/// прохода логируются и не фатальны.
pub async fn run_relay<D: Dispatcher>(sources: Vec<RelaySource>, dispatcher: D, poll: Duration) {
    loop {
        match relay_tick(&sources, &dispatcher, 100).await {
            Ok(0) => tokio::time::sleep(poll).await,
            Ok(_) => tokio::task::yield_now().await,
            Err(e) => {
                tracing::error!(error = %e, "relay tick failed");
                tokio::time::sleep(poll).await;
            }
        }
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
