//! `tiny-shop` — bootstrap бинарника: поднимает контексты `catalog`/`product`/`orders`, relay
//! проекции каталога и Axum SSR-сервер (T1a-6 + T1b-1 корзина).
//!
//! `identity` — отдельный контекст со своим файлом БД (см. `.claude/docs/design-1a.md` §1), но
//! пока не используется ни одним маршрутом — его открытие/миграция и подключение к auth-роутам
//! остаются на потом, чтобы не заводить неиспользуемое состояние раньше необходимости (Простота).

use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let data_dir = std::env::var("TINY_SHOP_DATA_DIR").unwrap_or_else(|_| "./data".to_string());
    std::fs::create_dir_all(&data_dir)?;

    let catalog_db = db::open("catalog", format!("{data_dir}/catalog.db")).await?;
    let product_db = db::open("product", format!("{data_dir}/product.db")).await?;
    let orders_db = db::open("orders", format!("{data_dir}/orders.db")).await?;

    db::migrate_catalog(&catalog_db.writer).await?;
    db::migrate_product(&product_db.writer).await?;
    db::migrate_orders(&orders_db.writer).await?;

    // Relay: события Product* из product.outbox → проекция каталога (search/projection).
    let projection = catalog::CatalogProjection::new(catalog_db.clone());
    let relay_sources = vec![db::relay::RelaySource {
        name: "product".into(),
        pool: product_db.writer.clone(),
    }];
    // `run_relay` сама по себе работает вечно и логирует ошибки тиков; этот внешний
    // `spawn` нужен только чтобы заметить аварийную панику задачи (иначе проекция каталога
    // молча перестанет обновляться, а сервер продолжит отвечать устаревшими данными).
    let relay_task = tokio::spawn(db::relay::run_relay(
        relay_sources,
        projection,
        Duration::from_millis(300),
    ));
    tokio::spawn(async move {
        if let Err(e) = relay_task.await {
            tracing::error!(error = %e, "catalog projection relay task exited unexpectedly");
        }
    });

    let state = web::AppState {
        search: catalog::SqliteCatalogSearch::new(catalog_db.clone()),
        taxonomy: catalog::TaxonomyRepo::new(catalog_db.clone()),
        carts: orders::CartRepo::new(orders_db.clone()),
        orders: orders::OrderRepo::new(orders_db.clone()),
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
