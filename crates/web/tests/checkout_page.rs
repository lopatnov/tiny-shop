//! Интеграционные тесты T1b-2: `GET /checkout`, `POST /checkout`, `GET /checkout/done/{id}`
//! сквозь весь стек (БД → repo/search → HTML, cart-cookie).

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
    let path = std::env::temp_dir().join(format!("tinyshop-web-checkout-{nanos}-{n}.db"));
    let _ = std::fs::remove_file(&path);
    let db = open(tag, &path).await.expect("open");
    migrate_catalog(&db.writer).await.expect("migrate");

    let orders_path =
        std::env::temp_dir().join(format!("tinyshop-web-checkout-orders-{nanos}-{n}.db"));
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

/// Создать + опублікувати товар без категорії.
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

/// Изменить цену опублікованого товару напрямую в проекции (имитация переопубликации с новой
/// ценой между add-to-cart и checkout).
async fn update_published_price(t: &TempDb, product_id: &str, price_minor: i64) {
    sqlx::query("UPDATE product_projection SET price_minor = ? WHERE id = ?")
        .bind(price_minor)
        .bind(product_id)
        .execute(&t.db.writer)
        .await
        .expect("update price");
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

/// `true`, если заголовок `Set-Cookie` истекает cart-cookie (`Max-Age=0`).
fn cart_cookie_expired(response: &axum::response::Response) -> bool {
    response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("cart=;") && v.contains("Max-Age=0"))
}

/// Добавить товар в корзину, вернуть `(app, cookie)`.
async fn add_to_cart(t: &TempDb, slug: &str, qty: i64) -> (axum::Router, String) {
    let app = router(app_state(t).await);

    let add_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/cart/add")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(format!("slug={slug}&qty={qty}")))
                .unwrap(),
        )
        .await
        .unwrap();
    let cookie = cart_cookie_value(&add_response).expect("Set-Cookie expected");

    (app, cookie)
}

// -----------------------------------------------------------------
// GET /checkout — пустая корзина
// -----------------------------------------------------------------

#[tokio::test]
async fn get_checkout_without_cart_redirects_to_cart() {
    let t = temp_db("checkout-empty").await;
    let app = router(app_state(&t).await);

    let response = app
        .oneshot(
            Request::builder()
                .uri("/checkout")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/cart");
}

#[tokio::test]
async fn post_checkout_with_no_cookie_redirects_to_cart() {
    let t = temp_db("checkout-empty-cart").await;
    let app = router(app_state(&t).await);

    // POST /checkout without a cart-cookie — same "no cart" path as the GET test above,
    // but for the form-submit handler.
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/checkout")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("email=buyer%40example.com&name=Buyer"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(response.headers().get(header::LOCATION).unwrap(), "/cart");
}

// -----------------------------------------------------------------
// Полный флоу: add -> GET /checkout -> POST /checkout -> GET /checkout/done/{id}
// -----------------------------------------------------------------

#[tokio::test]
async fn full_checkout_flow_creates_order_and_clears_cart() {
    let t = temp_db("checkout-flow").await;
    let proj = CatalogProjection::new(t.db.clone());
    create_published_product(&proj, "p1", "Синій віджет", "blue-widget", 19999).await;

    let (app, cookie) = add_to_cart(&t, "blue-widget", 2).await;

    // GET /checkout — позиции + форма.
    let checkout_get = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/checkout")
                .header(header::COOKIE, cookie.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(checkout_get.status(), StatusCode::OK);
    let checkout_html = body_string(checkout_get).await;
    assert!(
        checkout_html.contains("Синій віджет"),
        "checkout page should show cart item title: {checkout_html}"
    );
    assert!(
        checkout_html.contains("type=\"email\""),
        "checkout page should show a contact form: {checkout_html}"
    );
    // unit price 199.99 * qty 2 = 399.98
    assert!(
        checkout_html.contains("399.98"),
        "checkout page should show line total: {checkout_html}"
    );

    // POST /checkout — submit contact form.
    let submit_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/checkout")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, cookie.clone())
                .body(Body::from("email=buyer%40example.com&name=Buyer"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(submit_response.status(), StatusCode::SEE_OTHER);
    assert!(
        cart_cookie_expired(&submit_response),
        "POST /checkout should expire the cart cookie (Max-Age=0): {:?}",
        submit_response.headers().get(header::SET_COOKIE)
    );
    let location = submit_response
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        location.starts_with("/checkout/done/"),
        "should redirect to confirmation: {location}"
    );

    // GET /checkout/done/{order_id} — confirmation page shows items/sum/order number.
    let done_response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(&location)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(done_response.status(), StatusCode::OK);
    let done_html = body_string(done_response).await;
    assert!(
        done_html.contains("Замовлення прийнято"),
        "confirmation page should greet the buyer: {done_html}"
    );
    let order_id = location.trim_start_matches("/checkout/done/");
    assert!(
        done_html.contains(order_id),
        "confirmation page should show the order id: {done_html}"
    );
    assert!(
        done_html.contains("Синій віджет"),
        "confirmation page should show ordered item title: {done_html}"
    );
    assert!(
        done_html.contains("399.98"),
        "confirmation page should show order total: {done_html}"
    );

    // GET /cart — empty after checkout.
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
    let cart_html = body_string(cart_response).await;
    assert!(
        cart_html.contains("Кошик порожній"),
        "cart should be empty after checkout: {cart_html}"
    );
}

// -----------------------------------------------------------------
// POST /checkout — валидация контактной формы
// -----------------------------------------------------------------

#[tokio::test]
async fn post_checkout_without_email_returns_bad_request_with_form() {
    let t = temp_db("checkout-no-email").await;
    let proj = CatalogProjection::new(t.db.clone());
    create_published_product(&proj, "p1", "Синій віджет", "blue-widget", 19999).await;

    let (app, cookie) = add_to_cart(&t, "blue-widget", 1).await;

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/checkout")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, cookie)
                .body(Body::from("email=&name=Buyer"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = body_string(response).await;
    assert!(
        body.contains("type=\"email\""),
        "form should be redisplayed: {body}"
    );
}

// -----------------------------------------------------------------
// Цена в заказе — свежий снимок проекции, не цена на момент добавления
// -----------------------------------------------------------------

#[tokio::test]
async fn checkout_uses_fresh_price_snapshot_not_cart_snapshot() {
    let t = temp_db("checkout-fresh-price").await;
    let proj = CatalogProjection::new(t.db.clone());
    create_published_product(&proj, "p1", "Синій віджет", "blue-widget", 10000).await;

    let (app, cookie) = add_to_cart(&t, "blue-widget", 1).await;

    // Republish with a new price between add-to-cart and checkout.
    update_published_price(&t, "p1", 15000).await;

    let submit_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/checkout")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(header::COOKIE, cookie)
                .body(Body::from("email=buyer%40example.com"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(submit_response.status(), StatusCode::SEE_OTHER);
    let location = submit_response
        .headers()
        .get(header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let done_html = body_string(
        app.oneshot(
            Request::builder()
                .uri(&location)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap(),
    )
    .await;

    // Old price (100.00) must NOT appear; new price (150.00) must.
    assert!(
        done_html.contains("150.00"),
        "order should reflect fresh price snapshot: {done_html}"
    );
    assert!(
        !done_html.contains("100.00"),
        "order should not show stale cart-snapshot price: {done_html}"
    );
}
