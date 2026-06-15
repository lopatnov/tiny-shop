//! Интеграционные тесты T1b-1: `GET /cart`, `POST /cart/add`, `POST /cart/update`,
//! `POST /cart/remove` сквозь весь стек (БД → repo/search → HTML, cart-cookie).

use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use catalog::{CatalogProjection, SqliteCatalogSearch, TaxonomyRepo};
use db::{ContextDb, migrate_catalog, migrate_orders, open, relay::Dispatcher};
use orders::{CartRepo, OrderRepo};
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
    let path = std::env::temp_dir().join(format!("tinyshop-web-cart-{nanos}-{n}.db"));
    let _ = std::fs::remove_file(&path);
    let db = open(tag, &path).await.expect("open");
    migrate_catalog(&db.writer).await.expect("migrate");

    let orders_path = std::env::temp_dir().join(format!("tinyshop-web-cart-orders-{nanos}-{n}.db"));
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
        orders: OrderRepo::new(t.orders_db.clone()),
        base_url: "http://127.0.0.1:8080".to_string(),
    }
}

/// Создать + опублікувати товар без категорії (для `/cart/add` категорія не потрібна).
async fn create_published_product(
    proj: &CatalogProjection,
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
            "ProductPublished",
            serde_json::json!({"id": id, "from": "draft", "to": "published", "updated_at": 2}),
        ),
    )
    .await
    .expect("published");
}

async fn body_string(response: axum::response::Response) -> String {
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

/// Извлечь значение cart-cookie (`cart=<token>`) из заголовка `Set-Cookie` ответа.
fn cart_cookie_value(response: &axum::response::Response) -> Option<String> {
    let raw = response.headers().get(header::SET_COOKIE)?.to_str().ok()?;
    let (name, rest) = raw.split_once('=')?;
    if name != "cart" {
        return None;
    }
    let value = rest.split(';').next()?;
    Some(format!("cart={value}"))
}

// -----------------------------------------------------------------
// GET /cart — пустая корзина
// -----------------------------------------------------------------

#[tokio::test]
async fn empty_cart_without_cookie_shows_friendly_message() {
    let t = temp_db("web-cart-empty").await;
    let app = router(app_state(&t).await);

    let response = app
        .oneshot(Request::builder().uri("/cart").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers().get(header::SET_COOKIE).is_none(),
        "GET /cart must not set a cookie"
    );

    let body = body_string(response).await;
    assert!(body.contains("<h1>"), "body should contain <h1>: {body}");
    assert!(
        body.contains("Кошик порожній"),
        "body should show empty-cart message: {body}"
    );
    assert!(
        !body.contains("<table"),
        "empty cart should not render a table: {body}"
    );
}

// -----------------------------------------------------------------
// POST /cart/add — happy path + roundtrip
// -----------------------------------------------------------------

#[tokio::test]
async fn add_valid_product_redirects_and_sets_cart_cookie() {
    let t = temp_db("web-cart-add").await;
    let proj = CatalogProjection::new(t.db.clone());
    create_published_product(&proj, "p1", "Синій віджет", "blue-widget", 19999).await;

    let app = router(app_state(&t).await);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/cart/add")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("slug=blue-widget&qty=2"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/cart");
    let cookie = cart_cookie_value(&response).expect("Set-Cookie: cart=... expected");
    assert!(cookie.starts_with("cart="), "cookie: {cookie}");
}

#[tokio::test]
async fn cart_with_added_item_shows_title_qty_and_total() {
    let t = temp_db("web-cart-roundtrip").await;
    let proj = CatalogProjection::new(t.db.clone());
    create_published_product(&proj, "p1", "Синій віджет", "blue-widget", 19999).await;

    let app = router(app_state(&t).await);

    let add_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/cart/add")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("slug=blue-widget&qty=2"))
                .unwrap(),
        )
        .await
        .unwrap();
    let cookie = cart_cookie_value(&add_response).expect("Set-Cookie expected");

    let cart_response = app
        .oneshot(
            Request::builder()
                .uri("/cart")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(cart_response.status(), StatusCode::OK);
    let body = body_string(cart_response).await;

    assert!(
        body.contains("Синій віджет"),
        "cart should show added product title: {body}"
    );
    assert!(
        body.contains("value=\"2\""),
        "cart should show qty=2: {body}"
    );
    // unit price 199.99 * qty 2 = 399.98
    assert!(
        body.contains("399.98"),
        "cart should show line total: {body}"
    );
}

// -----------------------------------------------------------------
// POST /cart/add — несуществующий slug
// -----------------------------------------------------------------

#[tokio::test]
async fn add_unknown_slug_redirects_to_product_page_without_500() {
    let t = temp_db("web-cart-add-unknown").await;
    let app = router(app_state(&t).await);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/cart/add")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("slug=does-not-exist&qty=1"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response.headers().get(header::LOCATION).unwrap(),
        "/p/does-not-exist"
    );
    assert!(
        response.headers().get(header::SET_COOKIE).is_none(),
        "unknown product must not create a cart"
    );
}

// -----------------------------------------------------------------
// POST /cart/update / POST /cart/remove
// -----------------------------------------------------------------

/// Добавить `blue-widget` (qty=2) в новую корзину, вернуть `(app, cookie, item_id)`.
async fn seeded_cart(t: &TempDb) -> (axum::Router, String, i64) {
    let proj = CatalogProjection::new(t.db.clone());
    create_published_product(&proj, "p1", "Синій віджет", "blue-widget", 19999).await;

    let app = router(app_state(t).await);

    let add_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/cart/add")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("slug=blue-widget&qty=2"))
                .unwrap(),
        )
        .await
        .unwrap();
    let cookie = cart_cookie_value(&add_response).expect("Set-Cookie expected");

    let cart_html = body_string(
        app.clone()
            .oneshot(
                Request::builder()
                    .uri("/cart")
                    .header(header::COOKIE, cookie.clone())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap(),
    )
    .await;
    let item_id = extract_item_id(&cart_html);

    (app, cookie, item_id)
}

/// Извлечь `item_id` из `<input type="hidden" name="item_id" value="...">` в HTML корзины.
fn extract_item_id(html: &str) -> i64 {
    let marker = "name=\"item_id\" value=\"";
    let start = html.find(marker).expect("item_id hidden input") + marker.len();
    let rest = &html[start..];
    let end = rest.find('"').expect("closing quote");
    rest[..end].parse().expect("item_id is a number")
}

#[tokio::test]
async fn update_qty_changes_cart_display() {
    let t = temp_db("web-cart-update").await;
    let (app, cookie, item_id) = seeded_cart(&t).await;

    let update_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/cart/update")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, cookie.clone())
                .body(Body::from(format!("item_id={item_id}&qty=5")))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(update_response.status(), StatusCode::SEE_OTHER);

    let cart_html = body_string(
        app.oneshot(
            Request::builder()
                .uri("/cart")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap(),
    )
    .await;

    assert!(
        cart_html.contains("value=\"5\""),
        "cart should show updated qty=5: {cart_html}"
    );
    // unit price 199.99 * qty 5 = 999.95
    assert!(
        cart_html.contains("999.95"),
        "cart should show updated line total: {cart_html}"
    );
}

#[tokio::test]
async fn remove_item_empties_cart() {
    let t = temp_db("web-cart-remove").await;
    let (app, cookie, item_id) = seeded_cart(&t).await;

    let remove_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/cart/remove")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, cookie.clone())
                .body(Body::from(format!("item_id={item_id}")))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(remove_response.status(), StatusCode::SEE_OTHER);

    let cart_html = body_string(
        app.oneshot(
            Request::builder()
                .uri("/cart")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap(),
    )
    .await;

    assert!(
        cart_html.contains("Кошик порожній"),
        "cart should be empty after removing the only item: {cart_html}"
    );
    assert!(
        !cart_html.contains("Синій віджет"),
        "removed product title should not appear: {cart_html}"
    );
}

#[tokio::test]
async fn update_without_cart_cookie_redirects_without_error() {
    let t = temp_db("web-cart-update-no-cookie").await;
    let app = router(app_state(&t).await);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/cart/update")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("item_id=1&qty=5"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/cart");
}
