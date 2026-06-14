//! Интеграционный тест T1a-6 (chunk 3): `GET /` сквозь весь стек (БД → repo → HTML).

use std::sync::atomic::{AtomicUsize, Ordering};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use catalog::{Category, SqliteCatalogSearch, TaxonomyRepo};
use db::{ContextDb, migrate_catalog, migrate_orders, open};
use orders::CartRepo;
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
    let path = std::env::temp_dir().join(format!("tinyshop-web-home-{nanos}-{n}.db"));
    let _ = std::fs::remove_file(&path);
    let db = open(tag, &path).await.expect("open");
    migrate_catalog(&db.writer).await.expect("migrate");

    let orders_path = std::env::temp_dir().join(format!("tinyshop-web-home-orders-{nanos}-{n}.db"));
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

async fn app_state(t: &TempDb) -> AppState {
    AppState {
        search: SqliteCatalogSearch::new(t.db.clone()),
        taxonomy: TaxonomyRepo::new(t.db.clone()),
        carts: CartRepo::new(t.orders_db.clone()),
        base_url: "http://127.0.0.1:8080".to_string(),
    }
}

async fn get_body(app: axum::Router, uri: &str) -> (StatusCode, String) {
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

#[tokio::test]
async fn home_page_lists_root_categories() {
    let t = temp_db("web-home-categories").await;
    let tax = TaxonomyRepo::new(t.db.clone());

    let electronics = Category {
        id: "cat1".into(),
        parent_id: None,
        name: "Електроніка".into(),
        slug: "electronics".into(),
        path: "/electronics".into(),
        position: 0,
    };
    tax.create_category(&electronics).await.expect("category");

    let books = Category {
        id: "cat2".into(),
        parent_id: None,
        name: "Книги".into(),
        slug: "books".into(),
        path: "/books".into(),
        position: 1,
    };
    tax.create_category(&books).await.expect("category");

    // Дочерняя категория не должна попасть в навигацию главной страницы.
    let phones = Category {
        id: "cat3".into(),
        parent_id: Some(electronics.id.clone()),
        name: "Телефони".into(),
        slug: "phones".into(),
        path: "/electronics/phones".into(),
        position: 0,
    };
    tax.create_category(&phones).await.expect("category");

    let app = router(app_state(&t).await);

    let (status, body) = get_body(app, "/").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<h1>"), "body should contain <h1>: {body}");
    assert!(
        body.contains("/c/electronics"),
        "body should link to root category: {body}"
    );
    assert!(
        body.contains("Електроніка"),
        "body should contain root category name: {body}"
    );
    assert!(
        body.contains("/c/books"),
        "body should link to second root category: {body}"
    );
    assert!(
        !body.contains("/c/phones"),
        "body should not link to non-root category: {body}"
    );
}

#[tokio::test]
async fn home_page_without_categories_renders_placeholder() {
    let t = temp_db("web-home-empty").await;
    let app = router(app_state(&t).await);

    let (status, body) = get_body(app, "/").await;

    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("<h1>"), "body should contain <h1>: {body}");
    assert!(
        body.contains("Категорії з'являться тут найближчим часом."),
        "body should contain empty-state placeholder: {body}"
    );
}
