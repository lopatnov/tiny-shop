//! Интеграционный тест T1a-6 (chunk 2): `GET /c/{slug}` сквозь весь стек
//! (БД → repo/search → HTML+JSON-LD).

use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use catalog::{
    Attribute, CatalogProjection, Category, DataType, SqliteCatalogSearch, TaxonomyRepo,
};
use db::{ContextDb, migrate_catalog, open, relay::Dispatcher};
use shared::{DomainEvent, now_ms};
use tower::ServiceExt;
use web::{AppState, router};

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
    let path = std::env::temp_dir().join(format!("tinyshop-web-cat-{nanos}-{n}.db"));
    let _ = std::fs::remove_file(&path);
    let db = open(tag, &path).await.expect("open");
    migrate_catalog(&db.writer).await.expect("migrate");
    TempDb { path, db }
}

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

async fn app_state(db: &ContextDb) -> AppState {
    AppState {
        search: SqliteCatalogSearch::new(db.clone()),
        taxonomy: TaxonomyRepo::new(db.clone()),
        base_url: "http://127.0.0.1:8080".to_string(),
    }
}

/// Завести категорию + атрибут (для привязки товара к категории через `product_attr_index`,
/// см. `crates/catalog/src/projection.rs`).
async fn seed_category(tax: &TaxonomyRepo) -> (Category, Attribute) {
    let category = Category {
        id: "cat1".into(),
        parent_id: None,
        name: "Електроніка".into(),
        slug: "electronics".into(),
        path: "/electronics".into(),
        position: 0,
    };
    tax.create_category(&category).await.expect("category");

    let attribute = Attribute {
        id: "attr1".into(),
        category_id: category.id.clone(),
        name: "Колір".into(),
        data_type: DataType::Enum,
        unit: None,
        is_required: false,
        position: 0,
    };
    tax.create_attribute(&attribute).await.expect("attribute");

    (category, attribute)
}

/// Создати + опублікувати товар, прив'язаний до категорії через атрибут.
async fn create_published_product(
    proj: &CatalogProjection,
    attribute_id: &str,
    id: &str,
    title: &str,
    slug: &str,
    price_minor: i64,
) {
    proj.dispatch(
        "product",
        &event(
            1,
            "ProductCreated",
            serde_json::json!({
                "id": id, "seller_id": "s1", "title": title, "slug": slug,
                "description": "Опис товару", "price_minor": price_minor,
                "currency": "UAH", "status": "draft", "created_at": 1, "updated_at": 1,
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
                "id": id,
                "reason": "attribute_value_set",
                "attribute_id": attribute_id,
                "data_type": "enum",
                "val_text": "blue",
                "updated_at": 2,
            }),
        ),
    )
    .await
    .expect("attribute set");

    proj.dispatch(
        "product",
        &event(
            3,
            "ProductPublished",
            serde_json::json!({"id": id, "from": "draft", "to": "published", "updated_at": 3}),
        ),
    )
    .await
    .expect("published");
}

#[tokio::test]
async fn category_page_with_products_returns_html_with_jsonld() {
    let t = temp_db("web-category-ok").await;
    let tax = TaxonomyRepo::new(t.db.clone());
    let proj = CatalogProjection::new(t.db.clone());

    let (category, attribute) = seed_category(&tax).await;
    create_published_product(
        &proj,
        &attribute.id,
        "p1",
        "Синій віджет",
        "blue-widget",
        19999,
    )
    .await;

    let app = router(app_state(&t.db).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/c/{}", category.slug))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();

    assert!(body.contains("<h1>"), "body should contain <h1>: {body}");
    assert!(
        body.contains("Електроніка"),
        "body should contain category name: {body}"
    );
    assert!(
        body.contains("/p/blue-widget"),
        "body should link to product page: {body}"
    );
    assert!(
        body.contains("199.99"),
        "body should contain formatted price: {body}"
    );

    let jsonld = extract_jsonld_blocks(&body);

    let breadcrumb_ld = jsonld
        .iter()
        .find(|v| v["@type"] == "BreadcrumbList")
        .expect("BreadcrumbList JSON-LD block");
    let crumb_items = breadcrumb_ld["itemListElement"]
        .as_array()
        .expect("itemListElement array");
    assert!(
        crumb_items
            .iter()
            .any(|item| item["item"] == "http://127.0.0.1:8080/c/electronics"),
        "BreadcrumbList items should use absolute URLs from base_url: {breadcrumb_ld}"
    );

    let item_list_ld = jsonld
        .iter()
        .find(|v| v["@type"] == "ItemList")
        .expect("ItemList JSON-LD block");
    let items = item_list_ld["itemListElement"]
        .as_array()
        .expect("itemListElement array");
    assert_eq!(items.len(), 1);
    assert_eq!(
        items[0]["item"]["url"],
        "http://127.0.0.1:8080/p/blue-widget"
    );
    assert_eq!(items[0]["item"]["name"], "Синій віджет");
}

#[tokio::test]
async fn category_page_without_products_renders_empty_state() {
    let t = temp_db("web-category-empty").await;
    let tax = TaxonomyRepo::new(t.db.clone());

    let (category, _attribute) = seed_category(&tax).await;

    let app = router(app_state(&t.db).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/c/{}", category.slug))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();

    assert!(body.contains("<h1>"), "body should contain <h1>: {body}");
    assert!(
        body.contains("Електроніка"),
        "body should contain category name: {body}"
    );

    let jsonld = extract_jsonld_blocks(&body);
    assert!(
        jsonld.iter().all(|v| v["@type"] != "ItemList"),
        "no ItemList JSON-LD expected for empty category: {body}"
    );
}

#[tokio::test]
async fn unknown_category_slug_returns_404() {
    let t = temp_db("web-category-404").await;
    let app = router(app_state(&t.db).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/c/unknown")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// Извлечь и распарсить все блоки `<script type="application/ld+json">` из HTML-страницы.
fn extract_jsonld_blocks(body: &str) -> Vec<serde_json::Value> {
    let marker = r#"<script type="application/ld+json">"#;
    let mut blocks = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find(marker) {
        let after = &rest[start + marker.len()..];
        let end = after.find("</script>").expect("script end");
        blocks.push(serde_json::from_str(&after[..end]).expect("valid JSON-LD"));
        rest = &after[end..];
    }
    blocks
}
