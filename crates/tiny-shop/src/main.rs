//! `tiny-shop` — bootstrap бинарника: поднимает контексты `catalog`/`product`, relay
//! проекции каталога и Axum SSR-сервер (T1a-6).
//!
//! `identity`/`orders` — отдельные контексты с собственными файлами БД (см.
//! `.claude/docs/design-1a.md` §1), но в T1a-6 ещё не используются ни одним маршрутом —
//! их открытие/миграция и подключение к auth/orders-роутам остаются для 1b, чтобы не заводить
//! неиспользуемое состояние раньше необходимости (Простота).

use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let data_dir = std::env::var("TINY_SHOP_DATA_DIR").unwrap_or_else(|_| "./data".to_string());
    std::fs::create_dir_all(&data_dir)?;

    let catalog_db = db::open("catalog", format!("{data_dir}/catalog.db")).await?;
    let product_db = db::open("product", format!("{data_dir}/product.db")).await?;

    db::migrate_catalog(&catalog_db.writer).await?;
    db::migrate_product(&product_db.writer).await?;

    // Relay: события Product* из product.outbox → проекция каталога (search/projection).
    let projection = catalog::CatalogProjection::new(catalog_db.clone());
    let relay_sources = vec![db::relay::RelaySource {
        name: "product".into(),
        pool: product_db.writer.clone(),
    }];
    tokio::spawn(db::relay::run_relay(
        relay_sources,
        projection,
        Duration::from_millis(300),
    ));

    let state = web::AppState {
        search: catalog::SqliteCatalogSearch::new(catalog_db.clone()),
        taxonomy: catalog::TaxonomyRepo::new(catalog_db.clone()),
        base_url: std::env::var("TINY_SHOP_BASE_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8080".into()),
    };
    let app = web::router(state).layer(tower_http::trace::TraceLayer::new_for_http());

    let addr = std::env::var("TINY_SHOP_ADDR").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, "tiny-shop listening");
    axum::serve(listener, app).await?;
    Ok(())
}
