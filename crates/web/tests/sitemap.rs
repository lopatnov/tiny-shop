//! Интеграционные тесты T1a-6 (chunk 3): `GET /sitemap.xml` и `GET /robots.txt`.

use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
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
    let path = std::env::temp_dir().join(format!("tinyshop-web-sitemap-{nanos}-{n}.db"));
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

async fn get_response(app: axum::Router, uri: &str) -> (StatusCode, Option<String>, String) {
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_string());
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        content_type,
        String::from_utf8(body.to_vec()).unwrap(),
    )
}

#[tokio::test]
async fn sitemap_lists_home_categories_and_products() {
    let t = temp_db("web-sitemap-ok").await;
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

    let (status, content_type, body) = get_response(app, "/sitemap.xml").await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        content_type.as_deref().is_some_and(|ct| ct.contains("xml")),
        "Content-Type should mention xml: {content_type:?}"
    );
    assert!(
        body.contains("<urlset"),
        "body should contain <urlset: {body}"
    );
    assert!(
        body.contains("http://127.0.0.1:8080/</loc>"),
        "body should contain home URL: {body}"
    );
    assert!(
        body.contains(&format!("/c/{}", category.slug)),
        "body should contain category URL: {body}"
    );
    assert!(
        body.contains("/p/blue-widget"),
        "body should contain product URL: {body}"
    );
}

#[tokio::test]
async fn sitemap_without_data_still_lists_home() {
    let t = temp_db("web-sitemap-empty").await;
    let app = router(app_state(&t.db).await);

    let (status, content_type, body) = get_response(app, "/sitemap.xml").await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        content_type.as_deref().is_some_and(|ct| ct.contains("xml")),
        "Content-Type should mention xml: {content_type:?}"
    );
    assert!(
        body.contains("http://127.0.0.1:8080/</loc>"),
        "body should contain home URL: {body}"
    );
}

#[tokio::test]
async fn robots_txt_points_to_sitemap() {
    let t = temp_db("web-robots").await;
    let app = router(app_state(&t.db).await);

    let (status, content_type, body) = get_response(app, "/robots.txt").await;

    assert_eq!(status, StatusCode::OK);
    assert!(
        content_type
            .as_deref()
            .is_some_and(|ct| ct.contains("text/plain")),
        "Content-Type should be text/plain: {content_type:?}"
    );
    assert!(
        body.contains("Sitemap: http://127.0.0.1:8080/sitemap.xml"),
        "body should reference sitemap with base_url: {body}"
    );
    assert!(body.contains("User-agent: *"), "body: {body}");
    assert!(body.contains("Allow: /"), "body: {body}");
}
