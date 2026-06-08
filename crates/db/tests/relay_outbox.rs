//! Интеграционные тесты T1a-1: outbox round-trip, идемпотентность inbox, relay tick.
//! Точки тестирования из design-1a.md §7.3 (R2/R3 — согласованность и идемпотентность).

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use db::relay::{DispatchError, Dispatcher, RelaySource, relay_tick};
use db::{ContextDb, inbox, migrate_catalog, open, outbox};
use serde_json::json;
use shared::{DomainEvent, NewEvent};

/// Уникальный временный файл БД на тест; чистим за собой (включая WAL/SHM).
struct TempDb {
    path: std::path::PathBuf,
    db: ContextDb,
}

impl Drop for TempDb {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let p = format!("{}{}", self.path.display(), suffix);
            let _ = std::fs::remove_file(p);
        }
    }
}

async fn temp_db(tag: &str) -> TempDb {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("tinyshop-{tag}-{nanos}-{n}.db"));
    let _ = std::fs::remove_file(&path);
    // catalog-набор содержит и outbox, и inbox_processed — удобно для всех тестов.
    let db = open(tag, &path).await.expect("open");
    migrate_catalog(&db.writer).await.expect("migrate");
    TempDb { path, db }
}

#[tokio::test]
async fn outbox_roundtrip() {
    let t = temp_db("outbox").await;
    let id1 = outbox::enqueue(
        &t.db.writer,
        &NewEvent::new("product", "p1", "ProductPublished", json!({ "x": 1 })),
    )
    .await
    .unwrap();
    let _id2 = outbox::enqueue(
        &t.db.writer,
        &NewEvent::new("product", "p2", "ProductPublished", json!({})),
    )
    .await
    .unwrap();

    let pending = outbox::fetch_unpublished(&t.db.reader, 10).await.unwrap();
    assert_eq!(pending.len(), 2, "оба события не разосланы");
    assert_eq!(pending[0].id, id1, "порядок по возрастанию id");
    assert_eq!(
        pending[0].payload,
        json!({ "x": 1 }),
        "payload сохранён как JSON"
    );

    outbox::mark_published(&t.db.writer, &[pending[0].id, pending[1].id])
        .await
        .unwrap();
    let pending2 = outbox::fetch_unpublished(&t.db.reader, 10).await.unwrap();
    assert!(pending2.is_empty(), "после mark_published очередь пуста");
}

#[tokio::test]
async fn inbox_is_idempotent() {
    let t = temp_db("inbox").await;
    let first = inbox::mark_processed(&t.db.writer, "product", 42)
        .await
        .unwrap();
    let second = inbox::mark_processed(&t.db.writer, "product", 42)
        .await
        .unwrap();
    assert!(first, "первая обработка — true");
    assert!(!second, "повтор того же события — false (идемпотентно)");
}

struct CountingDispatcher(Arc<AtomicUsize>);

impl Dispatcher for CountingDispatcher {
    async fn dispatch(&self, _source: &str, _event: &DomainEvent) -> Result<(), DispatchError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn relay_dispatches_then_marks_published() {
    let t = temp_db("relay").await;
    outbox::enqueue(
        &t.db.writer,
        &NewEvent::new("product", "p1", "E", json!({})),
    )
    .await
    .unwrap();
    outbox::enqueue(
        &t.db.writer,
        &NewEvent::new("product", "p2", "E", json!({})),
    )
    .await
    .unwrap();

    let counter = Arc::new(AtomicUsize::new(0));
    let sources = vec![RelaySource {
        name: "product".into(),
        pool: t.db.writer.clone(),
    }];

    let n = relay_tick(&sources, &CountingDispatcher(counter.clone()), 100)
        .await
        .unwrap();
    assert_eq!(n, 2, "доставлено два события");
    assert_eq!(counter.load(Ordering::SeqCst), 2);

    // Повторный проход: всё уже published, доставлять нечего.
    let n2 = relay_tick(&sources, &CountingDispatcher(counter.clone()), 100)
        .await
        .unwrap();
    assert_eq!(n2, 0, "второй проход — пусто (idempotent at-least-once)");
}
