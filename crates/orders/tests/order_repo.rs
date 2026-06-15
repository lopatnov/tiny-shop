//! Интеграционные тесты T1a-8: создание заказа, добавление позиций,
//! снимок конфигурации, пагинация, корректность total_minor.

use std::sync::atomic::{AtomicUsize, Ordering};

use db::{ContextDb, migrate_orders, open};
use orders::{NewOrder, NewOrderContact, NewOrderItem, OrderError, OrderRepo, OrderStatus};
use shared::Pagination;

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
    let path = std::env::temp_dir().join(format!("tinyshop-orders-{nanos}-{n}.db"));
    let _ = std::fs::remove_file(&path);
    let db = open(tag, &path).await.expect("open");
    migrate_orders(&db.writer).await.expect("migrate");
    TempDb { path, db }
}

fn new_order(id: &str, buyer_id: &str) -> NewOrder {
    NewOrder {
        id: id.to_string(),
        buyer_id: buyer_id.to_string(),
        currency: "UAH".to_string(),
    }
}

fn new_item(id: &str, order_id: &str, title: &str, price: i64) -> NewOrderItem {
    NewOrderItem {
        id: id.to_string(),
        order_id: order_id.to_string(),
        product_id: "prod-1".to_string(),
        seller_id: "seller-1".to_string(),
        variant_id: None,
        title: title.to_string(),
        unit_price_minor: price,
        currency: "UAH".to_string(),
        config_snapshot: None,
    }
}

// -----------------------------------------------------------------
// Tests
// -----------------------------------------------------------------

#[tokio::test]
async fn create_and_get_order() {
    let t = temp_db("create-order").await;
    let repo = OrderRepo::new(t.db.clone());

    repo.create_order(&new_order("ord-1", "buyer-1"))
        .await
        .expect("create");

    let order = repo.get_order("ord-1").await.expect("get").expect("found");
    assert_eq!(order.id, "ord-1");
    assert_eq!(order.buyer_id, "buyer-1");
    assert_eq!(order.status, OrderStatus::Created);
    assert_eq!(order.total_minor, 0);
    assert_eq!(order.currency, "UAH");
}

#[tokio::test]
async fn get_order_not_found() {
    let t = temp_db("not-found").await;
    let repo = OrderRepo::new(t.db.clone());
    assert!(repo.get_order("missing").await.expect("get").is_none());
}

#[tokio::test]
async fn add_item_updates_total() {
    let t = temp_db("total").await;
    let repo = OrderRepo::new(t.db.clone());

    repo.create_order(&new_order("ord-2", "buyer-2"))
        .await
        .expect("create");
    repo.add_item(&new_item("item-1", "ord-2", "Книга PDF", 10000))
        .await
        .expect("add 1");
    repo.add_item(&new_item("item-2", "ord-2", "Книга EPUB", 8000))
        .await
        .expect("add 2");

    let order = repo.get_order("ord-2").await.expect("get").expect("found");
    assert_eq!(order.total_minor, 18000);
}

#[tokio::test]
async fn get_order_with_items() {
    let t = temp_db("with-items").await;
    let repo = OrderRepo::new(t.db.clone());

    repo.create_order(&new_order("ord-3", "buyer-3"))
        .await
        .expect("create");
    repo.add_item(&new_item("item-a", "ord-3", "Відео-курс", 25000))
        .await
        .expect("add");

    let order = repo
        .get_order_with_items("ord-3")
        .await
        .expect("get")
        .expect("found");
    assert_eq!(order.items.len(), 1);
    assert_eq!(order.items[0].title, "Відео-курс");
    assert_eq!(order.items[0].unit_price_minor, 25000);
}

#[tokio::test]
async fn config_snapshot_roundtrip() {
    let t = temp_db("snapshot").await;
    let repo = OrderRepo::new(t.db.clone());

    let snapshot = serde_json::json!({
        "delivery": "download",
        "format": "PDF",
        "license": "single"
    });
    repo.create_order(&new_order("ord-4", "buyer-4"))
        .await
        .expect("create");
    let item = NewOrderItem {
        id: "item-snap".to_string(),
        order_id: "ord-4".to_string(),
        product_id: "prod-snap".to_string(),
        seller_id: "seller-snap".to_string(),
        variant_id: Some("var-pdf".to_string()),
        title: "e-book".to_string(),
        unit_price_minor: 5000,
        currency: "UAH".to_string(),
        config_snapshot: Some(snapshot.clone()),
    };
    repo.add_item(&item).await.expect("add");

    let order = repo
        .get_order_with_items("ord-4")
        .await
        .expect("get")
        .expect("found");
    let stored = order.items[0].config_snapshot.as_ref().expect("snapshot");
    assert_eq!(*stored, snapshot);
    assert_eq!(order.items[0].variant_id.as_deref(), Some("var-pdf"));
}

#[tokio::test]
async fn list_by_buyer_pagination() {
    let t = temp_db("list-pg").await;
    let repo = OrderRepo::new(t.db.clone());

    for i in 0..5u32 {
        repo.create_order(&NewOrder {
            id: format!("ord-pg-{i}"),
            buyer_id: "buyer-pg".to_string(),
            currency: "UAH".to_string(),
        })
        .await
        .expect("create");
    }
    // Different buyer — must not appear in results.
    repo.create_order(&new_order("ord-other", "other-buyer"))
        .await
        .expect("create other");

    let page1 = repo
        .list_by_buyer(
            "buyer-pg",
            &Pagination {
                limit: 3,
                offset: 0,
            },
        )
        .await
        .expect("page1");
    assert_eq!(page1.total, 5);
    assert_eq!(page1.items.len(), 3);
    assert_eq!(page1.page.offset, 0);

    let page2 = repo
        .list_by_buyer(
            "buyer-pg",
            &Pagination {
                limit: 3,
                offset: 3,
            },
        )
        .await
        .expect("page2");
    assert_eq!(page2.total, 5);
    assert_eq!(page2.items.len(), 2);
    assert_eq!(page2.page.offset, 3);
}

#[tokio::test]
async fn order_emits_to_outbox() {
    let t = temp_db("outbox").await;
    let repo = OrderRepo::new(t.db.clone());

    repo.create_order(&new_order("ord-outbox", "buyer-o"))
        .await
        .expect("create");

    let events = db::outbox::fetch_unpublished(&t.db.reader, 10)
        .await
        .expect("fetch");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "OrderCreated");
    assert_eq!(events[0].aggregate_id, "ord-outbox");
}

// -----------------------------------------------------------------
// checkout (T1b-2)
// -----------------------------------------------------------------

/// Позиция корзины с `qty > 1` разворачивается в несколько `NewOrderItem` (по одной на
/// единицу товара) — см. `web::routes::checkout::render_submit`. Хелпер генерирует `qty`
/// штук с уникальными id и одинаковым снимком.
fn item_units(
    product_id: &str,
    title: &str,
    unit_price_minor: i64,
    qty: usize,
) -> Vec<NewOrderItem> {
    (0..qty)
        .map(|i| NewOrderItem {
            id: format!("{product_id}-unit-{i}"),
            order_id: String::new(),
            product_id: product_id.to_string(),
            seller_id: "seller-checkout".to_string(),
            variant_id: None,
            title: title.to_string(),
            unit_price_minor,
            currency: "UAH".to_string(),
            config_snapshot: None,
        })
        .collect()
}

#[tokio::test]
async fn checkout_happy_path_creates_order_with_items_total_contact_and_outbox() {
    let t = temp_db("checkout-happy").await;
    let repo = OrderRepo::new(t.db.clone());

    // Two distinct products, one with qty=2 — total must be Σ unit_price_minor * qty.
    let mut items = item_units("prod-a", "Електронна книга", 10_000, 2);
    items.extend(item_units("prod-b", "Відео-курс", 25_000, 1));

    let contact = NewOrderContact {
        email: "buyer@example.com".to_string(),
        name: Some("Іван".to_string()),
    };

    let order_id = repo
        .checkout("guest:abc", "UAH", &items, Some(&contact))
        .await
        .expect("checkout");

    let order = repo
        .get_order_with_items(&order_id)
        .await
        .expect("get")
        .expect("found");
    assert_eq!(order.buyer_id, "guest:abc");
    assert_eq!(order.status, OrderStatus::Created);
    assert_eq!(order.currency, "UAH");
    // (10_000 * 2) + (25_000 * 1) = 45_000
    assert_eq!(order.total_minor, 45_000);
    assert_eq!(order.items.len(), 3);

    let events = db::outbox::fetch_unpublished(&t.db.reader, 10)
        .await
        .expect("fetch");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "OrderCreated");
    assert_eq!(events[0].aggregate_id, order_id);

    let contact_row: (String, Option<String>) =
        sqlx::query_as("SELECT email, name FROM order_contact WHERE order_id = ?")
            .bind(&order_id)
            .fetch_one(&t.db.reader)
            .await
            .expect("contact row");
    assert_eq!(contact_row.0, "buyer@example.com");
    assert_eq!(contact_row.1.as_deref(), Some("Іван"));
}

#[tokio::test]
async fn checkout_without_contact_skips_order_contact() {
    let t = temp_db("checkout-no-contact").await;
    let repo = OrderRepo::new(t.db.clone());

    let items = item_units("prod-a", "Електронна книга", 10_000, 1);
    let order_id = repo
        .checkout("guest:nocontact", "UAH", &items, None)
        .await
        .expect("checkout");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM order_contact WHERE order_id = ?")
        .bind(&order_id)
        .fetch_one(&t.db.reader)
        .await
        .expect("count");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn checkout_empty_items_returns_error_and_creates_nothing() {
    let t = temp_db("checkout-empty").await;
    let repo = OrderRepo::new(t.db.clone());

    let err = repo
        .checkout("guest:empty", "UAH", &[], None)
        .await
        .expect_err("empty items must be rejected");
    assert!(matches!(err, OrderError::EmptyCheckout), "got {err:?}");

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders")
        .fetch_one(&t.db.reader)
        .await
        .expect("count");
    assert_eq!(count, 0, "no order row must be created for empty checkout");
}

#[tokio::test]
async fn checkout_is_atomic_rollback_on_duplicate_item_id() {
    let t = temp_db("checkout-atomic").await;
    let repo = OrderRepo::new(t.db.clone());

    // Two items sharing the same `id` violate the order_items PRIMARY KEY mid-transaction —
    // the whole checkout (order + first item + contact) must roll back.
    let mut items = item_units("prod-a", "Дублікат", 5_000, 1);
    let mut duplicate = item_units("prod-a", "Дублікат", 5_000, 1).remove(0);
    duplicate.id = items[0].id.clone();
    items.push(duplicate);

    let contact = NewOrderContact {
        email: "rollback@example.com".to_string(),
        name: None,
    };

    let err = repo
        .checkout("guest:atomic", "UAH", &items, Some(&contact))
        .await
        .expect_err("duplicate item id should fail");
    assert!(matches!(err, OrderError::Db(_)), "got {err:?}");

    let orders_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders")
        .fetch_one(&t.db.reader)
        .await
        .expect("count orders");
    assert_eq!(orders_count, 0, "order must not exist after rollback");

    let items_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM order_items")
        .fetch_one(&t.db.reader)
        .await
        .expect("count items");
    assert_eq!(items_count, 0, "no items must exist after rollback");

    let contact_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM order_contact")
        .fetch_one(&t.db.reader)
        .await
        .expect("count contact");
    assert_eq!(contact_count, 0, "no contact must exist after rollback");
}

#[tokio::test]
async fn checkout_rejects_item_with_currency_other_than_order_currency() {
    let t = temp_db("checkout-currency-mismatch").await;
    let repo = OrderRepo::new(t.db.clone());

    let mut items = item_units("prod-a", "Електронна книга", 10_000, 1);
    items[0].currency = "EUR".to_string();

    let err = repo
        .checkout("guest:currency", "UAH", &items, None)
        .await
        .expect_err("mixed-currency checkout must be rejected");
    assert!(
        matches!(err, OrderError::CurrencyMismatch { .. }),
        "got {err:?}"
    );

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM orders")
        .fetch_one(&t.db.reader)
        .await
        .expect("count");
    assert_eq!(
        count, 0,
        "no order row must be created on currency mismatch"
    );
}
