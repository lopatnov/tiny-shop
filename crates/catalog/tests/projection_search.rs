//! Интеграционные тесты T1a-5: проекция каталога + CatalogSearch (SQLite/FTS5).
//!
//! Проверяемые сценарии:
//! - ProductCreated → строка в product_projection, запись в product_fts.
//! - ProductUpdated (core) → обновление title/price в projection и FTS.
//! - ProductPublished → статус published, поиск возвращает карточку.
//! - attribute_value_set → product_attr_index, FTS attrs, фильтр checkbox_or.
//! - attribute_value_cleared → удаление из product_attr_index.
//! - ProductDeleted → очистка всех трёх таблиц.
//! - SqliteCatalogSearch::search() — текст, фильтры, пагинация.
//! - SqliteCatalogSearch::upsert() / remove() — прямой API.

use std::sync::atomic::{AtomicUsize, Ordering};

use catalog::{
    Attribute, CatalogProjection, CatalogSearch, Category, DataType, FilterCond, ProductDoc,
    SearchQuery, Sort, SqliteCatalogSearch, TaxonomyRepo,
};
use db::{ContextDb, migrate_catalog, open, relay::Dispatcher};
use shared::{DomainEvent, Pagination, now_ms};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
    // `tag` не входит в имя файла (CodeQL rust/path-injection).
    let path = std::env::temp_dir().join(format!("tinyshop-{nanos}-{n}.db"));
    let _ = std::fs::remove_file(&path);
    let db = open(tag, &path).await.expect("open");
    migrate_catalog(&db.writer).await.expect("migrate");
    TempDb { path, db }
}

/// Создать синтетическое событие для тестов.
fn event(id: i64, event_type: &str, payload: serde_json::Value) -> DomainEvent {
    DomainEvent {
        id,
        aggregate: "product".into(),
        aggregate_id: payload["id"].as_str().unwrap_or("p1").to_string(),
        event_type: event_type.into(),
        payload,
        created_at: now_ms(),
    }
}

fn default_query() -> SearchQuery {
    SearchQuery {
        text: None,
        category_id: None,
        filters: vec![],
        sort: Sort::Newest,
        page: Pagination::default(),
    }
}

// ---------------------------------------------------------------------------
// Тесты CatalogProjection
// ---------------------------------------------------------------------------

#[tokio::test]
async fn product_created_appears_in_projection() {
    let t = temp_db("proj-created").await;
    let proj = CatalogProjection::new(t.db.clone());

    let ev = event(
        1,
        "ProductCreated",
        serde_json::json!({
            "id": "p1",
            "seller_id": "s1",
            "title": "Cool Widget",
            "slug": "cool-widget",
            "description": "A great widget",
            "price_minor": 1000,
            "currency": "UAH",
            "status": "draft",
            "created_at": 1000,
            "updated_at": 1000,
        }),
    );

    proj.dispatch("product", &ev).await.expect("dispatch");

    let row: (String, i64, String) =
        sqlx::query_as("SELECT title, price_minor, status FROM product_projection WHERE id = 'p1'")
            .fetch_one(&t.db.reader)
            .await
            .expect("row");

    assert_eq!(row.0, "Cool Widget");
    assert_eq!(row.1, 1000);
    assert_eq!(row.2, "draft");

    // FTS entry created
    let fts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM product_fts WHERE product_id = 'p1'")
        .fetch_one(&t.db.reader)
        .await
        .expect("fts count");
    assert_eq!(fts, 1);
}

#[tokio::test]
async fn product_published_changes_status() {
    let t = temp_db("proj-published").await;
    let proj = CatalogProjection::new(t.db.clone());

    proj.dispatch(
        "product",
        &event(
            1,
            "ProductCreated",
            serde_json::json!({
                "id": "p1", "seller_id": "s1", "title": "T", "slug": "t",
                "description": "", "price_minor": 500, "currency": "UAH",
                "status": "draft", "created_at": 1, "updated_at": 1,
            }),
        ),
    )
    .await
    .expect("created");

    proj.dispatch(
        "product",
        &event(
            2,
            "ProductPublished",
            serde_json::json!({
                "id": "p1", "from": "draft", "to": "published", "updated_at": 2,
            }),
        ),
    )
    .await
    .expect("published");

    let status: String =
        sqlx::query_scalar("SELECT status FROM product_projection WHERE id = 'p1'")
            .fetch_one(&t.db.reader)
            .await
            .expect("status");

    assert_eq!(status, "published");
}

#[tokio::test]
async fn core_update_refreshes_projection_and_fts() {
    let t = temp_db("proj-core-update").await;
    let proj = CatalogProjection::new(t.db.clone());

    proj.dispatch(
        "product",
        &event(
            1,
            "ProductCreated",
            serde_json::json!({
                "id": "p1", "seller_id": "s1", "title": "Old Title", "slug": "old",
                "description": "old desc", "price_minor": 100, "currency": "UAH",
                "status": "draft", "created_at": 1, "updated_at": 1,
            }),
        ),
    )
    .await
    .expect("created");

    proj.dispatch(
        "product",
        &event(
            2,
            "ProductUpdated",
            serde_json::json!({
                "id": "p1",
                "title": "New Title",
                "slug": "new",
                "description": "new desc",
                "price_minor": 200,
                "currency": "UAH",
                "updated_at": 2,
            }),
        ),
    )
    .await
    .expect("updated");

    let row: (String, String, i64) =
        sqlx::query_as("SELECT title, slug, price_minor FROM product_projection WHERE id = 'p1'")
            .fetch_one(&t.db.reader)
            .await
            .expect("row");

    assert_eq!(row.0, "New Title");
    assert_eq!(row.1, "new");
    assert_eq!(row.2, 200);

    // FTS still has exactly one entry
    let fts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM product_fts WHERE product_id = 'p1'")
        .fetch_one(&t.db.reader)
        .await
        .expect("fts count");
    assert_eq!(fts, 1);
}

#[tokio::test]
async fn attribute_value_set_populates_attr_index() {
    let t = temp_db("proj-attr-set").await;
    let proj = CatalogProjection::new(t.db.clone());
    let tax = TaxonomyRepo::new(t.db.clone());

    // Seed taxonomy: category + attribute
    tax.create_category(&Category {
        id: "cat1".into(),
        parent_id: None,
        name: "Electronics".into(),
        slug: "electronics".into(),
        path: "/electronics".into(),
        position: 0,
    })
    .await
    .expect("category");

    tax.create_attribute(&Attribute {
        id: "attr1".into(),
        category_id: "cat1".into(),
        name: "Color".into(),
        data_type: DataType::Enum,
        unit: None,
        is_required: false,
        position: 0,
    })
    .await
    .expect("attribute");

    // Create product
    proj.dispatch(
        "product",
        &event(
            1,
            "ProductCreated",
            serde_json::json!({
                "id": "p1", "seller_id": "s1", "title": "Gadget", "slug": "gadget",
                "description": "", "price_minor": 300, "currency": "UAH",
                "status": "draft", "created_at": 1, "updated_at": 1,
            }),
        ),
    )
    .await
    .expect("created");

    // Set attribute value
    proj.dispatch(
        "product",
        &event(
            2,
            "ProductUpdated",
            serde_json::json!({
                "id": "p1",
                "reason": "attribute_value_set",
                "attribute_id": "attr1",
                "data_type": "enum",
                "val_text": "blue",
                "val_num": null,
                "updated_at": 2,
            }),
        ),
    )
    .await
    .expect("attr set");

    let row: (String, Option<String>) = sqlx::query_as(
        "SELECT category_id, val_text FROM product_attr_index WHERE product_id = 'p1' AND attribute_id = 'attr1'",
    )
    .fetch_one(&t.db.reader)
    .await
    .expect("attr row");

    assert_eq!(row.0, "cat1");
    assert_eq!(row.1.as_deref(), Some("blue"));

    // FTS attrs updated
    let attrs: String = sqlx::query_scalar("SELECT attrs FROM product_fts WHERE product_id = 'p1'")
        .fetch_one(&t.db.reader)
        .await
        .expect("fts attrs");
    assert!(
        attrs.contains("blue"),
        "attrs should contain 'blue', got: {attrs}"
    );
}

#[tokio::test]
async fn attribute_value_cleared_removes_from_index() {
    let t = temp_db("proj-attr-clear").await;
    let proj = CatalogProjection::new(t.db.clone());
    let tax = TaxonomyRepo::new(t.db.clone());

    tax.create_category(&Category {
        id: "cat1".into(),
        parent_id: None,
        name: "C".into(),
        slug: "c".into(),
        path: "/c".into(),
        position: 0,
    })
    .await
    .expect("cat");
    tax.create_attribute(&Attribute {
        id: "attr1".into(),
        category_id: "cat1".into(),
        name: "Size".into(),
        data_type: DataType::String,
        unit: None,
        is_required: false,
        position: 0,
    })
    .await
    .expect("attr");

    proj.dispatch(
        "product",
        &event(
            1,
            "ProductCreated",
            serde_json::json!({
                "id": "p1", "seller_id": "s1", "title": "T", "slug": "t",
                "description": "", "price_minor": 0, "currency": "UAH",
                "status": "draft", "created_at": 1, "updated_at": 1,
            }),
        ),
    )
    .await
    .expect("created");

    proj.dispatch(
        "product",
        &event(
            2,
            "ProductUpdated",
            serde_json::json!({
                "id": "p1", "reason": "attribute_value_set",
                "attribute_id": "attr1", "data_type": "string",
                "val_text": "large", "val_num": null, "updated_at": 2,
            }),
        ),
    )
    .await
    .expect("attr set");

    proj.dispatch(
        "product",
        &event(
            3,
            "ProductUpdated",
            serde_json::json!({
                "id": "p1", "reason": "attribute_value_cleared",
                "attribute_id": "attr1", "updated_at": 3,
            }),
        ),
    )
    .await
    .expect("attr clear");

    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM product_attr_index WHERE product_id = 'p1' AND attribute_id = 'attr1'",
    )
    .fetch_one(&t.db.reader)
    .await
    .expect("count");
    assert_eq!(count, 0);
}

#[tokio::test]
async fn product_deleted_clears_all_tables() {
    let t = temp_db("proj-deleted").await;
    let proj = CatalogProjection::new(t.db.clone());
    let tax = TaxonomyRepo::new(t.db.clone());

    tax.create_category(&Category {
        id: "cat1".into(),
        parent_id: None,
        name: "C".into(),
        slug: "c".into(),
        path: "/c".into(),
        position: 0,
    })
    .await
    .expect("cat");
    tax.create_attribute(&Attribute {
        id: "attr1".into(),
        category_id: "cat1".into(),
        name: "A".into(),
        data_type: DataType::Enum,
        unit: None,
        is_required: false,
        position: 0,
    })
    .await
    .expect("attr");

    proj.dispatch(
        "product",
        &event(
            1,
            "ProductCreated",
            serde_json::json!({
                "id": "p1", "seller_id": "s1", "title": "T", "slug": "t",
                "description": "", "price_minor": 0, "currency": "UAH",
                "status": "draft", "created_at": 1, "updated_at": 1,
            }),
        ),
    )
    .await
    .expect("created");

    proj.dispatch(
        "product",
        &event(
            2,
            "ProductUpdated",
            serde_json::json!({
                "id": "p1", "reason": "attribute_value_set",
                "attribute_id": "attr1", "data_type": "enum",
                "val_text": "red", "val_num": null, "updated_at": 2,
            }),
        ),
    )
    .await
    .expect("attr");

    proj.dispatch(
        "product",
        &event(3, "ProductDeleted", serde_json::json!({"id": "p1"})),
    )
    .await
    .expect("deleted");

    let pp: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM product_projection WHERE id = 'p1'")
        .fetch_one(&t.db.reader)
        .await
        .expect("pp");
    let ai: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM product_attr_index WHERE product_id = 'p1'")
            .fetch_one(&t.db.reader)
            .await
            .expect("ai");
    let fts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM product_fts WHERE product_id = 'p1'")
        .fetch_one(&t.db.reader)
        .await
        .expect("fts");

    assert_eq!(pp, 0);
    assert_eq!(ai, 0);
    assert_eq!(fts, 0);
}

// ---------------------------------------------------------------------------
// Тесты SqliteCatalogSearch
// ---------------------------------------------------------------------------

async fn setup_published_product(
    _t: &TempDb,
    proj: &CatalogProjection,
    id: &str,
    title: &str,
    price: i64,
) {
    let slug = id.replace('_', "-");
    proj.dispatch("product", &event(1, "ProductCreated", serde_json::json!({
        "id": id, "seller_id": "s1", "title": title, "slug": slug,
        "description": format!("Description of {title}"), "price_minor": price, "currency": "UAH",
        "status": "draft", "created_at": 1, "updated_at": 1,
    }))).await.expect("created");
    proj.dispatch(
        "product",
        &event(
            2,
            "ProductPublished",
            serde_json::json!({
                "id": id, "from": "draft", "to": "published", "updated_at": 2,
            }),
        ),
    )
    .await
    .expect("published");
}

#[tokio::test]
async fn search_returns_published_products_only() {
    let t = temp_db("search-published").await;
    let proj = CatalogProjection::new(t.db.clone());
    let search = SqliteCatalogSearch::new(t.db.clone());

    // Published product
    setup_published_product(&t, &proj, "p1", "Blue Widget", 1000).await;

    // Draft product (should not appear)
    proj.dispatch(
        "product",
        &event(
            3,
            "ProductCreated",
            serde_json::json!({
                "id": "p2", "seller_id": "s1", "title": "Hidden Draft", "slug": "hidden-draft",
                "description": "", "price_minor": 500, "currency": "UAH",
                "status": "draft", "created_at": 2, "updated_at": 2,
            }),
        ),
    )
    .await
    .expect("draft created");

    let result = search.search(&default_query()).await.expect("search");
    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].product_id, "p1");
}

#[tokio::test]
async fn search_text_fts_match() {
    let t = temp_db("search-fts").await;
    let proj = CatalogProjection::new(t.db.clone());
    let search = SqliteCatalogSearch::new(t.db.clone());

    setup_published_product(&t, &proj, "p1", "Wireless Keyboard", 2000).await;
    setup_published_product(&t, &proj, "p2", "USB Mouse", 800).await;

    let result = search
        .search(&SearchQuery {
            text: Some("keyboard".into()),
            ..default_query()
        })
        .await
        .expect("search");

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].product_id, "p1");
}

#[tokio::test]
async fn search_price_range_filter() {
    let t = temp_db("search-price").await;
    let proj = CatalogProjection::new(t.db.clone());
    let search = SqliteCatalogSearch::new(t.db.clone());

    setup_published_product(&t, &proj, "p1", "Cheap", 100).await;
    setup_published_product(&t, &proj, "p2", "Medium", 500).await;
    setup_published_product(&t, &proj, "p3", "Expensive", 2000).await;

    let result = search
        .search(&SearchQuery {
            filters: vec![FilterCond::RangePrice {
                min_minor: Some(200),
                max_minor: Some(1000),
            }],
            ..default_query()
        })
        .await
        .expect("search");

    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].product_id, "p2");
}

#[tokio::test]
async fn search_checkbox_or_filter() {
    let t = temp_db("search-checkbox-or").await;
    let proj = CatalogProjection::new(t.db.clone());
    let search = SqliteCatalogSearch::new(t.db.clone());
    let tax = TaxonomyRepo::new(t.db.clone());

    tax.create_category(&Category {
        id: "cat1".into(),
        parent_id: None,
        name: "Electronics".into(),
        slug: "electronics".into(),
        path: "/electronics".into(),
        position: 0,
    })
    .await
    .expect("cat");
    tax.create_attribute(&Attribute {
        id: "color".into(),
        category_id: "cat1".into(),
        name: "Color".into(),
        data_type: DataType::Enum,
        unit: None,
        is_required: false,
        position: 0,
    })
    .await
    .expect("attr");

    setup_published_product(&t, &proj, "p1", "Blue Headphones", 1500).await;
    setup_published_product(&t, &proj, "p2", "Red Headphones", 1200).await;
    setup_published_product(&t, &proj, "p3", "Green Headphones", 900).await;

    // Set color attributes
    for (pid, color, ev_id) in [("p1", "blue", 10), ("p2", "red", 11), ("p3", "green", 12)] {
        proj.dispatch(
            "product",
            &event(
                ev_id,
                "ProductUpdated",
                serde_json::json!({
                    "id": pid, "reason": "attribute_value_set",
                    "attribute_id": "color", "data_type": "enum",
                    "val_text": color, "val_num": null, "updated_at": ev_id,
                }),
            ),
        )
        .await
        .expect("attr set");
    }

    let result = search
        .search(&SearchQuery {
            filters: vec![FilterCond::CheckboxOr {
                attribute_id: "color".into(),
                values: vec!["blue".into(), "red".into()],
            }],
            ..default_query()
        })
        .await
        .expect("search");

    assert_eq!(result.total, 2);
    let ids: Vec<_> = result.items.iter().map(|c| c.product_id.as_str()).collect();
    assert!(ids.contains(&"p1"));
    assert!(ids.contains(&"p2"));
}

#[tokio::test]
async fn search_sort_price_asc() {
    let t = temp_db("search-sort").await;
    let proj = CatalogProjection::new(t.db.clone());
    let search = SqliteCatalogSearch::new(t.db.clone());

    setup_published_product(&t, &proj, "p1", "A", 300).await;
    setup_published_product(&t, &proj, "p2", "B", 100).await;
    setup_published_product(&t, &proj, "p3", "C", 200).await;

    let result = search
        .search(&SearchQuery {
            sort: Sort::PriceAsc,
            ..default_query()
        })
        .await
        .expect("search");

    assert_eq!(result.total, 3);
    assert_eq!(result.items[0].product_id, "p2");
    assert_eq!(result.items[1].product_id, "p3");
    assert_eq!(result.items[2].product_id, "p1");
}

#[tokio::test]
async fn search_pagination() {
    let t = temp_db("search-pagination").await;
    let proj = CatalogProjection::new(t.db.clone());
    let search = SqliteCatalogSearch::new(t.db.clone());

    for i in 1u32..=5 {
        setup_published_product(
            &t,
            &proj,
            &format!("p{i}"),
            &format!("Product {i}"),
            i as i64 * 100,
        )
        .await;
    }

    let page1 = search
        .search(&SearchQuery {
            sort: Sort::PriceAsc,
            page: Pagination {
                offset: 0,
                limit: 2,
            },
            ..default_query()
        })
        .await
        .expect("page1");

    let page2 = search
        .search(&SearchQuery {
            sort: Sort::PriceAsc,
            page: Pagination {
                offset: 2,
                limit: 2,
            },
            ..default_query()
        })
        .await
        .expect("page2");

    assert_eq!(page1.total, 5);
    assert_eq!(page1.items.len(), 2);
    assert_eq!(page2.items.len(), 2);
    // No overlap between pages
    let ids1: Vec<_> = page1.items.iter().map(|c| &c.product_id).collect();
    let ids2: Vec<_> = page2.items.iter().map(|c| &c.product_id).collect();
    assert!(ids1.iter().all(|id| !ids2.contains(id)));
}

#[tokio::test]
async fn upsert_and_remove_direct_api() {
    let t = temp_db("search-upsert-remove").await;
    let search = SqliteCatalogSearch::new(t.db.clone());

    let doc = ProductDoc {
        product_id: "p1".into(),
        category_id: "cat1".into(),
        title: "Direct Widget".into(),
        description: "A widget added directly".into(),
        price_minor: 999,
        currency: "UAH".into(),
        slug: "direct-widget".into(),
        attrs_text: "red large".into(),
    };

    search.upsert(&doc).await.expect("upsert");

    let result = search
        .search(&SearchQuery {
            text: Some("widget".into()),
            ..default_query()
        })
        .await
        .expect("search after upsert");
    assert_eq!(result.total, 1);
    assert_eq!(result.items[0].product_id, "p1");
    assert_eq!(result.items[0].price_minor, 999);

    search.remove("p1").await.expect("remove");

    let result2 = search
        .search(&default_query())
        .await
        .expect("search after remove");
    assert_eq!(result2.total, 0);
}

// ---------------------------------------------------------------------------
// Тесты SqliteCatalogSearch::get_card_by_slug (T1a-6)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_card_by_slug_returns_published_product() {
    let t = temp_db("card-by-slug-published").await;
    let proj = CatalogProjection::new(t.db.clone());
    let search = SqliteCatalogSearch::new(t.db.clone());

    setup_published_product(&t, &proj, "p1", "Blue Widget", 1999).await;

    let card = search
        .get_card_by_slug("p1")
        .await
        .expect("query")
        .expect("found");

    assert_eq!(card.id, "p1");
    assert_eq!(card.title, "Blue Widget");
    assert_eq!(card.slug, "p1");
    assert_eq!(card.price_minor, 1999);
    assert_eq!(card.currency, "UAH");
    assert_eq!(card.status, "published");
}

#[tokio::test]
async fn get_card_by_slug_returns_none_for_draft_or_archived() {
    let t = temp_db("card-by-slug-draft").await;
    let proj = CatalogProjection::new(t.db.clone());
    let search = SqliteCatalogSearch::new(t.db.clone());

    // Draft product (never published)
    proj.dispatch(
        "product",
        &event(
            1,
            "ProductCreated",
            serde_json::json!({
                "id": "p1", "seller_id": "s1", "title": "Draft Item", "slug": "draft-item",
                "description": "", "price_minor": 100, "currency": "UAH",
                "status": "draft", "created_at": 1, "updated_at": 1,
            }),
        ),
    )
    .await
    .expect("created");

    assert_eq!(
        search.get_card_by_slug("draft-item").await.expect("query"),
        None
    );

    // Archived product
    setup_published_product(&t, &proj, "p2", "Archived Item", 200).await;
    sqlx::query("UPDATE product_projection SET status = 'archived' WHERE id = 'p2'")
        .execute(&t.db.writer)
        .await
        .expect("archive");

    assert_eq!(search.get_card_by_slug("p2").await.expect("query"), None);
}

#[tokio::test]
async fn get_card_by_slug_returns_none_for_unknown_slug() {
    let t = temp_db("card-by-slug-unknown").await;
    let search = SqliteCatalogSearch::new(t.db.clone());

    assert_eq!(
        search
            .get_card_by_slug("does-not-exist")
            .await
            .expect("query"),
        None
    );
}
