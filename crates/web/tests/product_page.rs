//! Интеграционный тест T1a-6: `GET /p/{slug}` сквозь весь стек (БД → repo → HTML+JSON-LD).

use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use catalog::{CatalogProjection, SqliteCatalogSearch, TaxonomyRepo};
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
    let path = std::env::temp_dir().join(format!("tinyshop-web-{nanos}-{n}.db"));
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

#[tokio::test]
async fn product_page_returns_html_with_jsonld() {
    let t = temp_db("web-product-ok").await;
    let proj = CatalogProjection::new(t.db.clone());

    proj.dispatch(
        "product",
        &event(
            1,
            "ProductCreated",
            serde_json::json!({
                "id": "p1", "seller_id": "s1", "title": "Синій віджет", "slug": "blue-widget",
                "description": "Чудовий віджет синього кольору", "price_minor": 19999,
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
            "ProductPublished",
            serde_json::json!({"id": "p1", "from": "draft", "to": "published", "updated_at": 2}),
        ),
    )
    .await
    .expect("published");

    let app = router(app_state(&t.db).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/p/blue-widget")
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
        body.contains("199.99"),
        "body should contain formatted price: {body}"
    );

    let jsonld = extract_jsonld_blocks(&body);

    let product_ld = jsonld
        .iter()
        .find(|v| v["@type"] == "Product")
        .expect("Product JSON-LD block");
    assert_eq!(product_ld["offers"]["price"], "199.99");
    assert_eq!(product_ld["offers"]["priceCurrency"], "UAH");

    let breadcrumb_ld = jsonld
        .iter()
        .find(|v| v["@type"] == "BreadcrumbList")
        .expect("BreadcrumbList JSON-LD block");
    let items = breadcrumb_ld["itemListElement"]
        .as_array()
        .expect("itemListElement array");
    assert!(
        items
            .iter()
            .any(|item| item["item"] == "http://127.0.0.1:8080/p/blue-widget"),
        "BreadcrumbList items should use absolute URLs from base_url: {breadcrumb_ld}"
    );
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

#[tokio::test]
async fn unknown_slug_returns_404() {
    let t = temp_db("web-product-404").await;
    let app = router(app_state(&t.db).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/p/unknown")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
