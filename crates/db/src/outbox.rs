//! Transactional outbox (см. `design-1a.md` §1.2–1.3).
//!
//! `enqueue` пишется в ТУ ЖЕ транзакцию, что и доменная операция (атомарность в пределах файла).
//! Relay читает неразосланное по возрастанию `id` и помечает `published_at`.

use sqlx::{Row, Sqlite, SqlitePool};

use crate::DbError;
use shared::{DomainEvent, NewEvent, now_ms};

/// Записать событие в outbox. Принимает любой executor — в т.ч. `&mut *tx`,
/// чтобы лечь в одну транзакцию с доменной записью. Возвращает id события.
pub async fn enqueue<'e, E>(exec: E, ev: &NewEvent) -> Result<i64, DbError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let payload = serde_json::to_string(&ev.payload)?;
    let res = sqlx::query(
        "INSERT INTO outbox (aggregate, aggregate_id, event_type, payload, created_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&ev.aggregate)
    .bind(&ev.aggregate_id)
    .bind(&ev.event_type)
    .bind(payload)
    .bind(now_ms())
    .execute(exec)
    .await?;
    Ok(res.last_insert_rowid())
}

/// Прочитать неразосланные события (published_at IS NULL), упорядоченно по id.
pub async fn fetch_unpublished(pool: &SqlitePool, limit: u32) -> Result<Vec<DomainEvent>, DbError> {
    let rows = sqlx::query(
        "SELECT id, aggregate, aggregate_id, event_type, payload, created_at \
         FROM outbox WHERE published_at IS NULL ORDER BY id ASC LIMIT ?",
    )
    .bind(limit as i64)
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let payload_str: String = row.get("payload");
        out.push(DomainEvent {
            id: row.get("id"),
            aggregate: row.get("aggregate"),
            aggregate_id: row.get("aggregate_id"),
            event_type: row.get("event_type"),
            payload: serde_json::from_str(&payload_str)?,
            created_at: row.get("created_at"),
        });
    }
    Ok(out)
}

/// Пометить события разосланными.
pub async fn mark_published(pool: &SqlitePool, ids: &[i64]) -> Result<(), DbError> {
    if ids.is_empty() {
        return Ok(());
    }
    let now = now_ms();
    let mut tx = pool.begin().await?;
    for id in ids {
        sqlx::query("UPDATE outbox SET published_at = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}
