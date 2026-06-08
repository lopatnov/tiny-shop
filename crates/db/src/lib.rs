//! `db` — обвязка SQLite для модульного монолита (см. `.claude/docs/design-1a.md` §1).
//!
//! Принципы 1a:
//! - каждый bounded context = свой файл SQLite + свой пул (без JOIN между файлами);
//! - PRAGMA: WAL, synchronous=NORMAL, foreign_keys=ON, busy_timeout=5s;
//! - writer-пул = 1 соединение (SQLite — один писатель → нет `SQLITE_BUSY`), reader-пул — несколько;
//! - кросс-контекстная связь — через transactional outbox + in-process relay (модули ниже).

use std::path::Path;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

pub mod inbox;
pub mod outbox;
pub mod relay;

pub use shared::{DomainEvent, NewEvent};

/// Ошибки слоя БД.
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("sqlx: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("migrate: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Пара пулов одного файла-контекста: один писатель + несколько читателей.
#[derive(Clone)]
pub struct ContextDb {
    pub name: String,
    pub writer: SqlitePool,
    pub reader: SqlitePool,
}

/// Открыть файл-контекст: создаёт файл при отсутствии, ставит PRAGMA, поднимает пулы.
pub async fn open(name: impl Into<String>, path: impl AsRef<Path>) -> Result<ContextDb, DbError> {
    let base = SqliteConnectOptions::new()
        .filename(path.as_ref())
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));

    let writer = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(base.clone())
        .await?;
    let reader = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(base)
        .await?;

    Ok(ContextDb {
        name: name.into(),
        writer,
        reader,
    })
}

// Прогон миграций на каждый файл-контекст. Наборы миграций раздельные (per-file),
// встраиваются в бинарь на этапе компиляции.
pub async fn migrate_identity(pool: &SqlitePool) -> Result<(), DbError> {
    sqlx::migrate!("../../migrations/identity")
        .run(pool)
        .await?;
    Ok(())
}
pub async fn migrate_catalog(pool: &SqlitePool) -> Result<(), DbError> {
    sqlx::migrate!("../../migrations/catalog").run(pool).await?;
    Ok(())
}
pub async fn migrate_product(pool: &SqlitePool) -> Result<(), DbError> {
    sqlx::migrate!("../../migrations/product").run(pool).await?;
    Ok(())
}
pub async fn migrate_orders(pool: &SqlitePool) -> Result<(), DbError> {
    sqlx::migrate!("../../migrations/orders").run(pool).await?;
    Ok(())
}
