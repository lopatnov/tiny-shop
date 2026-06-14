//! Интеграционный тест T1a-6 (chunk 2 + chunk 3): `GET /c/{slug}` сквозь весь стек
//! (БД → repo/search → HTML+JSON-LD), включая фільтри (chunk 3).

use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use catalog::{
    Attribute, AttributeOption, CatalogProjection, Category, DataType, Filter, FilterType,
    SqliteCatalogSearch, TaxonomyRepo,
};
use db::{ContextDb, migrate_catalog, migrate_orders, open, relay::Dispatcher};
use orders::CartRepo;
use shared::{DomainEvent, now_ms};
use tower::ServiceExt;
use web::{AppState, router};

struct TempDb {
    path: std::path::PathBuf,
    db: ContextDb,
    orders_path: std::path::PathBuf,
    orders_db: ContextDb,
}

impl Drop for TempDb {
    fn drop(&mut self) {
        for base in [&self.path, &self.orders_path] {
            for suffix in ["", "-wal", "-shm"] {
                let p = format!("{}{}", base.display(), suffix);
                let _ = std::fs::remove_file(p);
            }
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

    let orders_path = std::env::temp_dir().join(format!("tinyshop-web-cat-orders-{nanos}-{n}.db"));
    let _ = std::fs::remove_file(&orders_path);
    let orders_db = open(format!("{tag}-orders"), &orders_path)
        .await
        .expect("open orders");
    migrate_orders(&orders_db.writer)
        .await
        .expect("migrate orders");

    TempDb {
        path,
        db,
        orders_path,
        orders_db,
    }
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

async fn app_state(t: &TempDb) -> AppState {
    AppState {
        search: SqliteCatalogSearch::new(t.db.clone()),
        taxonomy: TaxonomyRepo::new(t.db.clone()),
        carts: CartRepo::new(t.orders_db.clone()),
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
/// `val_text` — значення enum-атрибута (наприклад, колір) для `product_attr_index`.
async fn create_published_product(
    proj: &CatalogProjection,
    attribute_id: &str,
    id: &str,
    title: &str,
    slug: &str,
    price_minor: i64,
    val_text: &str,
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
                "val_text": val_text,
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
        "blue",
    )
    .await;

    let app = router(app_state(&t).await);

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

    let app = router(app_state(&t).await);

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
        !body.contains("<form"),
        "no filters configured for category — no <form> expected: {body}"
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
    let app = router(app_state(&t).await);

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

/// Прив'язати атрибут до категорії як `checkbox_or`-фільтр з двома опціями ("blue"/"red").
async fn seed_checkbox_filter(tax: &TaxonomyRepo, category_id: &str, attribute_id: &str) {
    tax.create_filter(&Filter {
        id: "filter1".into(),
        category_id: category_id.into(),
        attribute_id: attribute_id.into(),
        filter_type: FilterType::CheckboxOr,
        position: 0,
    })
    .await
    .expect("filter");

    for (id, value, position) in [("opt-blue", "blue", 0), ("opt-red", "red", 1)] {
        tax.create_attribute_option(&AttributeOption {
            id: id.into(),
            attribute_id: attribute_id.into(),
            value: value.into(),
            position,
        })
        .await
        .expect("attribute option");
    }
}

async fn body_string(response: axum::response::Response) -> String {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

#[tokio::test]
async fn category_filter_checkbox_or_narrows_results_and_marks_checked_option() {
    let t = temp_db("web-category-checkbox").await;
    let tax = TaxonomyRepo::new(t.db.clone());
    let proj = CatalogProjection::new(t.db.clone());

    let (category, attribute) = seed_category(&tax).await;
    seed_checkbox_filter(&tax, &category.id, &attribute.id).await;

    create_published_product(
        &proj,
        &attribute.id,
        "p1",
        "Синій віджет",
        "blue-widget",
        19999,
        "blue",
    )
    .await;
    create_published_product(
        &proj,
        &attribute.id,
        "p2",
        "Червоний віджет",
        "red-widget",
        29999,
        "red",
    )
    .await;

    let app = router(app_state(&t).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/c/{}?attr_attr1=blue", category.slug))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;

    assert!(
        body.contains("/p/blue-widget"),
        "filtered result should include the blue product: {body}"
    );
    assert!(
        !body.contains("/p/red-widget"),
        "filtered result should exclude the red product: {body}"
    );

    let item_list_ld = extract_jsonld_blocks(&body)
        .into_iter()
        .find(|v| v["@type"] == "ItemList")
        .expect("ItemList JSON-LD block");
    let items = item_list_ld["itemListElement"]
        .as_array()
        .expect("itemListElement array");
    assert_eq!(items.len(), 1, "expected a single filtered item: {body}");

    // Форма фільтрів: "blue" відмічений, "red" — ні.
    let blue_input = format!(
        "input type=\"checkbox\" name=\"attr_{}\" value=\"blue\" checked",
        attribute.id
    );
    let red_input = format!(
        "input type=\"checkbox\" name=\"attr_{}\" value=\"red\"",
        attribute.id
    );
    assert!(
        body.contains(&blue_input),
        "blue checkbox should be checked: {body}"
    );
    assert!(
        !body.contains(&format!("{red_input} checked")),
        "red checkbox should not be checked: {body}"
    );
}

#[tokio::test]
async fn category_page_without_filter_params_shows_all_with_unchecked_form() {
    let t = temp_db("web-category-checkbox-none").await;
    let tax = TaxonomyRepo::new(t.db.clone());
    let proj = CatalogProjection::new(t.db.clone());

    let (category, attribute) = seed_category(&tax).await;
    seed_checkbox_filter(&tax, &category.id, &attribute.id).await;

    create_published_product(
        &proj,
        &attribute.id,
        "p1",
        "Синій віджет",
        "blue-widget",
        19999,
        "blue",
    )
    .await;
    create_published_product(
        &proj,
        &attribute.id,
        "p2",
        "Червоний віджет",
        "red-widget",
        29999,
        "red",
    )
    .await;

    let app = router(app_state(&t).await);

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
    let body = body_string(response).await;

    assert!(
        body.contains("/p/blue-widget") && body.contains("/p/red-widget"),
        "without filter params both products should be listed: {body}"
    );
    assert!(
        body.contains("<form"),
        "category with configured filters should render a <form>: {body}"
    );
    assert!(
        !body.contains("checked"),
        "no checkbox should be pre-checked without filter params: {body}"
    );
    assert!(
        body.contains(&format!("name=\"attr_{}\"", attribute.id)),
        "form should expose the checkbox_or filter param: {body}"
    );
}

#[tokio::test]
async fn category_filter_range_price_narrows_results() {
    let t = temp_db("web-category-range-price").await;
    let tax = TaxonomyRepo::new(t.db.clone());
    let proj = CatalogProjection::new(t.db.clone());

    let (category, attribute) = seed_category(&tax).await;
    // attribute_id потрібен лише для задоволення FK `filters.attribute_id` —
    // запит RangePrice будує умову по `pp.price_minor`, не по `product_attr_index`.
    tax.create_filter(&Filter {
        id: "filter-price".into(),
        category_id: category.id.clone(),
        attribute_id: attribute.id.clone(),
        filter_type: FilterType::RangePrice,
        position: 0,
    })
    .await
    .expect("filter");

    create_published_product(
        &proj,
        &attribute.id,
        "p1",
        "Дешевий віджет",
        "cheap-widget",
        10000, // 100.00 UAH
        "blue",
    )
    .await;
    create_published_product(
        &proj,
        &attribute.id,
        "p2",
        "Дорогий віджет",
        "expensive-widget",
        20000, // 200.00 UAH
        "blue",
    )
    .await;

    let app = router(app_state(&t).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/c/{}?price_min=100&price_max=150", category.slug))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;

    assert!(
        body.contains("/p/cheap-widget"),
        "in-range product should be listed: {body}"
    );
    assert!(
        !body.contains("/p/expensive-widget"),
        "out-of-range product should not be listed: {body}"
    );

    // Форма зберігає введені межі (у гривнях, як вводив користувач).
    assert!(
        body.contains("name=\"price_min\" value=\"100\""),
        "price_min should be preserved in the form: {body}"
    );
    assert!(
        body.contains("name=\"price_max\" value=\"150\""),
        "price_max should be preserved in the form: {body}"
    );
}

#[tokio::test]
async fn category_filter_unknown_and_garbage_params_render_gracefully() {
    let t = temp_db("web-category-garbage-params").await;
    let tax = TaxonomyRepo::new(t.db.clone());
    let proj = CatalogProjection::new(t.db.clone());

    let (category, attribute) = seed_category(&tax).await;
    seed_checkbox_filter(&tax, &category.id, &attribute.id).await;

    create_published_product(
        &proj,
        &attribute.id,
        "p1",
        "Синій віджет",
        "blue-widget",
        19999,
        "blue",
    )
    .await;

    let app = router(app_state(&t).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/c/{}?attr_doesnotexist=x&attr_{}_min=notanumber",
                    category.slug, attribute.id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let body = body_string(response).await;

    assert!(
        body.contains("/p/blue-widget"),
        "unrecognized filter params should not exclude products: {body}"
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
