//! Идемпотентный inbox (см. `design-1a.md` §1.2–1.3).
//!
//! Потребитель в своей транзакции вызывает [`mark_processed`]; если событие уже обработано —
//! возвращается `false`, и проекцию применять не нужно. Делает повтор события безопасным.

use sqlx::Sqlite;

use crate::DbError;
use shared::now_ms;

/// Зафиксировать факт обработки события источника. Возвращает `true`, если это ПЕРВАЯ обработка
/// (запись вставлена), и `false`, если событие уже обрабатывалось (дубликат — пропустить).
///
/// Вызывать в той же транзакции, что и применение проекции, чтобы «обработано + применено»
/// были атомарны.
pub async fn mark_processed<'e, E>(exec: E, source: &str, event_id: i64) -> Result<bool, DbError>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let res = sqlx::query(
        "INSERT OR IGNORE INTO inbox_processed (source, event_id, processed_at) VALUES (?, ?, ?)",
    )
    .bind(source)
    .bind(event_id)
    .bind(now_ms())
    .execute(exec)
    .await?;
    Ok(res.rows_affected() > 0)
}
