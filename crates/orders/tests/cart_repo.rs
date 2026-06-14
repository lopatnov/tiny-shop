//! Интеграционные тесты T1b-1: создание корзины, roundtrip cart-токена, добавление/изменение/
//! удаление позиций, изоляция между корзинами.

use std::sync::atomic::{AtomicUsize, Ordering};

use db::{ContextDb, migrate_orders, open};
use orders::{CartError, CartRepo, MAX_QTY, NewCartItem};

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
    let path = std::env::temp_dir().join(format!("tinyshop-cart-{nanos}-{n}.db"));
    let _ = std::fs::remove_file(&path);
    let db = open(tag, &path).await.expect("open");
    migrate_orders(&db.writer).await.expect("migrate");
    TempDb { path, db }
}

fn new_item(product_id: &str, qty: i64) -> NewCartItem {
    NewCartItem {
        product_id: product_id.to_string(),
        variant_id: None,
        qty,
        title: "Електронна книга".to_string(),
        unit_price_minor: 12_900,
        currency: "UAH".to_string(),
    }
}

// -----------------------------------------------------------------
// create_cart / find_by_token
// -----------------------------------------------------------------

#[tokio::test]
async fn create_and_find_by_token_roundtrip() {
    let t = temp_db("create-find").await;
    let repo = CartRepo::new(t.db.clone());

    let (token, created) = repo.create_cart().await.expect("create");
    let found = repo
        .find_by_token(token.as_str())
        .await
        .expect("find")
        .expect("present");

    assert_eq!(created.token_hash, found.token_hash);
    assert_eq!(found.created_at, found.updated_at);
    // token_hash stored, not the raw token.
    assert_ne!(found.token_hash, token.as_str());
}

#[tokio::test]
async fn find_by_token_unknown_returns_none() {
    let t = temp_db("find-unknown").await;
    let repo = CartRepo::new(t.db.clone());

    let found = repo
        .find_by_token("does-not-exist-token")
        .await
        .expect("no db error");
    assert!(found.is_none());
}

// -----------------------------------------------------------------
// add_item
// -----------------------------------------------------------------

#[tokio::test]
async fn add_item_then_list() {
    let t = temp_db("add-list").await;
    let repo = CartRepo::new(t.db.clone());
    let (_token, cart) = repo.create_cart().await.expect("create");

    repo.add_item(&cart.token_hash, &new_item("prod-1", 2))
        .await
        .expect("add");

    let items = repo.list_items(&cart.token_hash).await.expect("list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].product_id, "prod-1");
    assert_eq!(items[0].qty, 2);
    assert_eq!(items[0].title, "Електронна книга");
    assert_eq!(items[0].unit_price_minor, 12_900);
    assert_eq!(items[0].currency, "UAH");
    assert!(items[0].variant_id.is_none());
}

#[tokio::test]
async fn add_item_repeated_increments_qty_not_duplicate_row() {
    let t = temp_db("add-repeat").await;
    let repo = CartRepo::new(t.db.clone());
    let (_token, cart) = repo.create_cart().await.expect("create");

    repo.add_item(&cart.token_hash, &new_item("prod-1", 1))
        .await
        .expect("add 1");
    repo.add_item(&cart.token_hash, &new_item("prod-1", 3))
        .await
        .expect("add 2");

    let items = repo.list_items(&cart.token_hash).await.expect("list");
    assert_eq!(items.len(), 1, "should not duplicate row for same product");
    assert_eq!(items[0].qty, 4);
}

#[tokio::test]
async fn add_item_repeated_clamps_qty_to_max_qty() {
    // ON CONFLICT ... DO UPDATE SET qty = MIN(qty + excluded.qty, ?) — combined qty must not
    // exceed MAX_QTY even if individual adds stay within the per-call limit.
    let t = temp_db("add-repeat-clamp").await;
    let repo = CartRepo::new(t.db.clone());
    let (_token, cart) = repo.create_cart().await.expect("create");

    repo.add_item(&cart.token_hash, &new_item("prod-1", MAX_QTY))
        .await
        .expect("add 1");
    repo.add_item(&cart.token_hash, &new_item("prod-1", MAX_QTY))
        .await
        .expect("add 2");

    let items = repo.list_items(&cart.token_hash).await.expect("list");
    assert_eq!(items.len(), 1, "should not duplicate row for same product");
    assert_eq!(
        items[0].qty, MAX_QTY,
        "combined qty must be clamped to MAX_QTY"
    );
}

#[tokio::test]
async fn add_item_repeated_without_variant_increments() {
    // Regression guard: SQLite treats NULL != NULL in UNIQUE; cart_items_unique uses
    // COALESCE(variant_id, '') so two adds of the same variant-less product collapse.
    let t = temp_db("add-null-variant").await;
    let repo = CartRepo::new(t.db.clone());
    let (_token, cart) = repo.create_cart().await.expect("create");

    let mut item = new_item("prod-novariant", 1);
    item.variant_id = None;
    repo.add_item(&cart.token_hash, &item).await.expect("add 1");
    repo.add_item(&cart.token_hash, &item).await.expect("add 2");

    let items = repo.list_items(&cart.token_hash).await.expect("list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].qty, 2);
}

#[tokio::test]
async fn add_item_different_variants_are_separate_rows() {
    let t = temp_db("add-variants").await;
    let repo = CartRepo::new(t.db.clone());
    let (_token, cart) = repo.create_cart().await.expect("create");

    let mut pdf = new_item("prod-book", 1);
    pdf.variant_id = Some("pdf".to_string());
    let mut epub = new_item("prod-book", 1);
    epub.variant_id = Some("epub".to_string());

    repo.add_item(&cart.token_hash, &pdf)
        .await
        .expect("add pdf");
    repo.add_item(&cart.token_hash, &epub)
        .await
        .expect("add epub");

    let items = repo.list_items(&cart.token_hash).await.expect("list");
    assert_eq!(items.len(), 2);
}

#[tokio::test]
async fn add_item_invalid_qty_rejected() {
    let t = temp_db("add-invalid-qty").await;
    let repo = CartRepo::new(t.db.clone());
    let (_token, cart) = repo.create_cart().await.expect("create");

    let err = repo
        .add_item(&cart.token_hash, &new_item("prod-1", 0))
        .await
        .expect_err("qty=0 should be rejected");
    assert!(matches!(err, CartError::InvalidQty(0)), "got {err:?}");

    let err = repo
        .add_item(&cart.token_hash, &new_item("prod-1", 1000))
        .await
        .expect_err("qty>MAX_QTY should be rejected");
    assert!(matches!(err, CartError::InvalidQty(1000)), "got {err:?}");
}

#[tokio::test]
async fn add_item_updates_cart_timestamp() {
    let t = temp_db("add-touches-cart").await;
    let repo = CartRepo::new(t.db.clone());
    let (token, cart) = repo.create_cart().await.expect("create");
    let created_at = cart.created_at;

    repo.add_item(&cart.token_hash, &new_item("prod-1", 1))
        .await
        .expect("add");

    let cart = repo
        .find_by_token(token.as_str())
        .await
        .expect("find")
        .unwrap();
    assert!(cart.updated_at >= created_at);
}

// -----------------------------------------------------------------
// update_qty
// -----------------------------------------------------------------

#[tokio::test]
async fn update_qty_changes_value() {
    let t = temp_db("update-qty").await;
    let repo = CartRepo::new(t.db.clone());
    let (_token, cart) = repo.create_cart().await.expect("create");
    repo.add_item(&cart.token_hash, &new_item("prod-1", 1))
        .await
        .expect("add");
    let item_id = repo.list_items(&cart.token_hash).await.expect("list")[0].id;

    repo.update_qty(&cart.token_hash, item_id, 5)
        .await
        .expect("update");

    let items = repo.list_items(&cart.token_hash).await.expect("list");
    assert_eq!(items[0].qty, 5);
}

#[tokio::test]
async fn update_qty_zero_removes_row() {
    let t = temp_db("update-qty-zero").await;
    let repo = CartRepo::new(t.db.clone());
    let (_token, cart) = repo.create_cart().await.expect("create");
    repo.add_item(&cart.token_hash, &new_item("prod-1", 1))
        .await
        .expect("add");
    let item_id = repo.list_items(&cart.token_hash).await.expect("list")[0].id;

    repo.update_qty(&cart.token_hash, item_id, 0)
        .await
        .expect("update to zero");

    let items = repo.list_items(&cart.token_hash).await.expect("list");
    assert!(items.is_empty());
}

#[tokio::test]
async fn update_qty_invalid_rejected() {
    let t = temp_db("update-qty-invalid").await;
    let repo = CartRepo::new(t.db.clone());
    let (_token, cart) = repo.create_cart().await.expect("create");
    repo.add_item(&cart.token_hash, &new_item("prod-1", 1))
        .await
        .expect("add");
    let item_id = repo.list_items(&cart.token_hash).await.expect("list")[0].id;

    let err = repo
        .update_qty(&cart.token_hash, item_id, 1000)
        .await
        .expect_err("qty>MAX_QTY should be rejected");
    assert!(matches!(err, CartError::InvalidQty(1000)), "got {err:?}");
}

#[tokio::test]
async fn update_qty_does_not_affect_other_carts_item() {
    let t = temp_db("update-qty-isolated").await;
    let repo = CartRepo::new(t.db.clone());

    let (_token_a, cart_a) = repo.create_cart().await.expect("create a");
    let (_token_b, cart_b) = repo.create_cart().await.expect("create b");

    repo.add_item(&cart_b.token_hash, &new_item("prod-1", 1))
        .await
        .expect("add to b");
    let item_b_id = repo.list_items(&cart_b.token_hash).await.expect("list b")[0].id;

    // Attempt to update cart B's item using cart A's id — must be a no-op (IDOR guard).
    repo.update_qty(&cart_a.token_hash, item_b_id, 5)
        .await
        .expect("update (no-op, not an error)");

    let items_b = repo.list_items(&cart_b.token_hash).await.expect("list b");
    assert_eq!(items_b[0].qty, 1, "cart B item must be unchanged");
}

// -----------------------------------------------------------------
// remove_item / clear / list_items
// -----------------------------------------------------------------

#[tokio::test]
async fn remove_item_removes_only_that_row() {
    let t = temp_db("remove-item").await;
    let repo = CartRepo::new(t.db.clone());
    let (_token, cart) = repo.create_cart().await.expect("create");

    repo.add_item(&cart.token_hash, &new_item("prod-1", 1))
        .await
        .expect("add 1");
    repo.add_item(&cart.token_hash, &new_item("prod-2", 1))
        .await
        .expect("add 2");
    let items = repo.list_items(&cart.token_hash).await.expect("list");
    let first_id = items[0].id;

    repo.remove_item(&cart.token_hash, first_id)
        .await
        .expect("remove");

    let items = repo.list_items(&cart.token_hash).await.expect("list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].product_id, "prod-2");
}

#[tokio::test]
async fn remove_item_does_not_affect_other_carts_item() {
    let t = temp_db("remove-isolated").await;
    let repo = CartRepo::new(t.db.clone());

    let (_token_a, cart_a) = repo.create_cart().await.expect("create a");
    let (_token_b, cart_b) = repo.create_cart().await.expect("create b");

    repo.add_item(&cart_b.token_hash, &new_item("prod-1", 1))
        .await
        .expect("add to b");
    let item_b_id = repo.list_items(&cart_b.token_hash).await.expect("list b")[0].id;

    repo.remove_item(&cart_a.token_hash, item_b_id)
        .await
        .expect("remove (no-op, not an error)");

    let items_b = repo.list_items(&cart_b.token_hash).await.expect("list b");
    assert_eq!(items_b.len(), 1, "cart B item must remain");
}

#[tokio::test]
async fn list_items_preserves_insertion_order() {
    let t = temp_db("list-order").await;
    let repo = CartRepo::new(t.db.clone());
    let (_token, cart) = repo.create_cart().await.expect("create");

    for i in 0..3 {
        repo.add_item(&cart.token_hash, &new_item(&format!("prod-{i}"), 1))
            .await
            .expect("add");
    }

    let items = repo.list_items(&cart.token_hash).await.expect("list");
    let ids: Vec<&str> = items.iter().map(|i| i.product_id.as_str()).collect();
    assert_eq!(ids, vec!["prod-0", "prod-1", "prod-2"]);
}

#[tokio::test]
async fn clear_removes_all_items_but_keeps_cart() {
    let t = temp_db("clear").await;
    let repo = CartRepo::new(t.db.clone());
    let (token, cart) = repo.create_cart().await.expect("create");

    repo.add_item(&cart.token_hash, &new_item("prod-1", 1))
        .await
        .expect("add 1");
    repo.add_item(&cart.token_hash, &new_item("prod-2", 2))
        .await
        .expect("add 2");

    repo.clear(&cart.token_hash).await.expect("clear");

    let items = repo.list_items(&cart.token_hash).await.expect("list");
    assert!(items.is_empty());

    // Cart row itself still exists (only items cleared).
    let still_present = repo
        .find_by_token(token.as_str())
        .await
        .expect("find")
        .is_some();
    assert!(still_present, "cart row must survive clear()");
}
