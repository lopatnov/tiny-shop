//! Интеграционные тесты T1a-4: товар продавца, цифровая конфигурация/варианты/медиа/атрибуты,
//! i18n-резолв, transactional outbox, отсутствие сирот в `translations` (`design-1a.md` §3,
//! ADR O5/T1a-4).

use std::sync::atomic::{AtomicUsize, Ordering};

use db::{ContextDb, migrate_product, open, outbox};

use product::{
    DataType, DeliveryKind, DigitalConfig, DigitalVariant, Lang, LicenseKind, MediaKind, Product,
    ProductAttributeValue, ProductMedia, ProductRepo, ProductStatus,
};

/// Уникальный временный файл БД на тест; чистим за собой (включая WAL/SHM).
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
    // `tag` НЕ участвует в имени файла (CodeQL rust/path-injection: параметр статически
    // считается недоверенным, хотя по факту всегда литерал из тестов) — уникальность и так
    // гарантирована nanos+counter; `tag` используется только как метка соединения ниже.
    let path = std::env::temp_dir().join(format!("tinyshop-{nanos}-{n}.db"));
    let _ = std::fs::remove_file(&path);
    let db = open(tag, &path).await.expect("open");
    migrate_product(&db.writer).await.expect("migrate");
    TempDb { path, db }
}

fn product(id: &str, seller_id: &str, title: &str, slug: &str, price_minor: i64) -> Product {
    Product {
        id: id.to_string(),
        seller_id: seller_id.to_string(),
        title: title.to_string(),
        slug: slug.to_string(),
        description: "Опис товару".to_string(),
        price_minor,
        currency: "UAH".to_string(),
        status: ProductStatus::Draft,
        created_at: 1_000,
        updated_at: 1_000,
    }
}

/// Последний `event_type` в outbox для агрегата `aggregate_id` — удобный помощник для проверки
/// эмиссии (читаем через `db::outbox::fetch_unpublished`, как в существующих outbox-тестах T1a-1).
async fn last_event_type_for(db: &ContextDb, aggregate_id: &str) -> Option<String> {
    let events = outbox::fetch_unpublished(&db.reader, 1000).await.unwrap();
    events
        .into_iter()
        .rfind(|e| e.aggregate_id == aggregate_id)
        .map(|e| e.event_type)
}

async fn event_types_for(db: &ContextDb, aggregate_id: &str) -> Vec<String> {
    outbox::fetch_unpublished(&db.reader, 1000)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.aggregate_id == aggregate_id)
        .map(|e| e.event_type)
        .collect()
}

#[tokio::test]
async fn create_read_update_and_slug_uniqueness() {
    let t = temp_db("product-crud").await;
    let repo = ProductRepo::new(t.db.clone());

    let p1 = product("p-1", "seller-1", "Курс Rust", "rust-course", 50_000);
    repo.create_product(&p1).await.unwrap();

    let got = repo
        .get_product("p-1", Lang::Uk)
        .await
        .unwrap()
        .expect("found");
    assert_eq!(got.title, "Курс Rust");
    assert_eq!(got.status, ProductStatus::Draft);

    let by_slug = repo
        .get_product_by_slug("seller-1", "rust-course", Lang::Uk)
        .await
        .unwrap()
        .expect("found by slug");
    assert_eq!(by_slug.id, "p-1");

    repo.update_product(
        "p-1",
        "Курс Rust (оновлено)",
        "rust-course",
        "Новий опис",
        60_000,
        "UAH",
    )
    .await
    .unwrap();
    let updated = repo.get_product("p-1", Lang::Uk).await.unwrap().unwrap();
    assert_eq!(updated.title, "Курс Rust (оновлено)");
    assert_eq!(updated.price_minor, 60_000);
    assert_eq!(updated.description, "Новий опис");

    // UNIQUE(seller_id, slug): дубль у того же продавца — ошибка
    let dup = product("p-2", "seller-1", "Інший курс", "rust-course", 10_000);
    let err = repo.create_product(&dup).await.unwrap_err();
    assert!(
        matches!(err, product::ProductError::Db(_)),
        "дубль slug у одного продавца должен дать ошибку БД: {err:?}"
    );

    // тот же slug у другого продавца — ок
    let other_seller = product("p-3", "seller-2", "Курс Rust", "rust-course", 50_000);
    repo.create_product(&other_seller).await.unwrap();
    let found = repo
        .get_product_by_slug("seller-2", "rust-course", Lang::Uk)
        .await
        .unwrap();
    assert!(found.is_some(), "тот же slug у другого продавца допустим");

    // создание/обновление эмитят события
    assert_eq!(
        last_event_type_for(&t.db, "p-1").await.as_deref(),
        Some("ProductUpdated")
    );
    let p1_events = event_types_for(&t.db, "p-1").await;
    assert!(p1_events.contains(&"ProductCreated".to_string()));
    assert!(p1_events.contains(&"ProductUpdated".to_string()));
}

#[tokio::test]
async fn status_transitions_emit_expected_events_and_delete_emits_deleted() {
    let t = temp_db("product-status").await;
    let repo = ProductRepo::new(t.db.clone());

    let p = product("p-1", "seller-1", "Курс Rust", "rust-course", 50_000);
    repo.create_product(&p).await.unwrap();

    repo.set_status("p-1", ProductStatus::Published)
        .await
        .unwrap();
    assert_eq!(
        last_event_type_for(&t.db, "p-1").await.as_deref(),
        Some("ProductPublished")
    );
    let got = repo.get_product("p-1", Lang::Uk).await.unwrap().unwrap();
    assert_eq!(got.status, ProductStatus::Published);

    repo.set_status("p-1", ProductStatus::Archived)
        .await
        .unwrap();
    assert_eq!(
        last_event_type_for(&t.db, "p-1").await.as_deref(),
        Some("ProductArchived")
    );

    // archived -> draft не определён маппингом переходов — события не появляется,
    // но статус всё равно меняется (репозиторий доверяет вызывающей стороне валидацию переходов).
    let before = event_types_for(&t.db, "p-1").await.len();
    repo.set_status("p-1", ProductStatus::Draft).await.unwrap();
    let after = event_types_for(&t.db, "p-1").await;
    assert_eq!(
        after.len(),
        before,
        "archived -> draft не порождает событие перехода"
    );
    let got = repo.get_product("p-1", Lang::Uk).await.unwrap().unwrap();
    assert_eq!(got.status, ProductStatus::Draft);

    // published -> draft эмитит ProductUnpublished
    repo.set_status("p-1", ProductStatus::Published)
        .await
        .unwrap();
    repo.set_status("p-1", ProductStatus::Draft).await.unwrap();
    assert_eq!(
        last_event_type_for(&t.db, "p-1").await.as_deref(),
        Some("ProductUnpublished")
    );

    repo.delete_product("p-1").await.unwrap();
    assert!(repo.get_product("p-1", Lang::Uk).await.unwrap().is_none());
    assert_eq!(
        last_event_type_for(&t.db, "p-1").await.as_deref(),
        Some("ProductDeleted")
    );
}

#[tokio::test]
async fn translation_resolves_with_coalesce_fallback() {
    let t = temp_db("product-i18n").await;
    let repo = ProductRepo::new(t.db.clone());

    let p = product("p-1", "seller-1", "Курс Rust", "rust-course", 50_000);
    repo.create_product(&p).await.unwrap();

    // без перевода — fallback на канон (uk) при чтении на ru
    let got = repo.get_product("p-1", Lang::Ru).await.unwrap().unwrap();
    assert_eq!(got.title, "Курс Rust", "без override читаем канон (uk)");

    repo.set_translation(
        "p-1",
        "product",
        "p-1",
        Lang::Ru,
        "title",
        "Курс Rust (рос.)",
    )
    .await
    .unwrap();

    let got_ru = repo.get_product("p-1", Lang::Ru).await.unwrap().unwrap();
    assert_eq!(got_ru.title, "Курс Rust (рос.)", "теперь читаем override");

    let got_uk = repo.get_product("p-1", Lang::Uk).await.unwrap().unwrap();
    assert_eq!(
        got_uk.title, "Курс Rust",
        "канон (uk) не зависит от ru-перевода"
    );

    // upsert заменяет значение
    repo.set_translation(
        "p-1",
        "product",
        "p-1",
        Lang::Ru,
        "title",
        "Курс Rust (оновл. рос.)",
    )
    .await
    .unwrap();
    let got_ru2 = repo.get_product("p-1", Lang::Ru).await.unwrap().unwrap();
    assert_eq!(got_ru2.title, "Курс Rust (оновл. рос.)");
}

#[tokio::test]
async fn digital_config_variant_media_attribute_crud_and_cascade() {
    let t = temp_db("product-digital").await;
    let repo = ProductRepo::new(t.db.clone());

    let p = product("p-1", "seller-1", "Курс Rust", "rust-course", 50_000);
    repo.create_product(&p).await.unwrap();

    // digital_config upsert
    let cfg = DigitalConfig {
        product_id: "p-1".to_string(),
        delivery_kind: DeliveryKind::Download,
        license_kind: Some(LicenseKind::Single),
        notes: Some("PDF + відео".to_string()),
    };
    repo.upsert_digital_config(&cfg).await.unwrap();
    let got_cfg = repo
        .get_digital_config("p-1")
        .await
        .unwrap()
        .expect("config");
    assert_eq!(got_cfg.delivery_kind, DeliveryKind::Download);
    assert_eq!(got_cfg.license_kind, Some(LicenseKind::Single));

    // upsert заменяет
    let cfg2 = DigitalConfig {
        delivery_kind: DeliveryKind::PlatformView,
        license_kind: None,
        notes: None,
        ..cfg.clone()
    };
    repo.upsert_digital_config(&cfg2).await.unwrap();
    let got_cfg2 = repo.get_digital_config("p-1").await.unwrap().unwrap();
    assert_eq!(got_cfg2.delivery_kind, DeliveryKind::PlatformView);
    assert_eq!(got_cfg2.license_kind, None);

    // variants
    let v1 = DigitalVariant {
        id: "v-1".to_string(),
        product_id: "p-1".to_string(),
        label: "Базовий".to_string(),
        format: Some("PDF".to_string()),
        price_delta_minor: 0,
        position: 0,
    };
    let v2 = DigitalVariant {
        id: "v-2".to_string(),
        product_id: "p-1".to_string(),
        label: "Розширений".to_string(),
        format: Some("PDF+EPUB".to_string()),
        price_delta_minor: 5_000,
        position: 1,
    };
    repo.add_variant(&v1).await.unwrap();
    repo.add_variant(&v2).await.unwrap();

    let variants = repo.list_variants("p-1", Lang::Uk).await.unwrap();
    assert_eq!(variants.len(), 2);
    assert_eq!(variants[0].label, "Базовий");
    assert_eq!(variants[1].price_delta_minor, 5_000);

    let mut v1_updated = v1.clone();
    v1_updated.label = "Базовий (оновл.)".to_string();
    v1_updated.price_delta_minor = 1_000;
    repo.update_variant(&v1_updated).await.unwrap();
    let variants = repo.list_variants("p-1", Lang::Uk).await.unwrap();
    assert_eq!(variants[0].label, "Базовий (оновл.)");
    assert_eq!(variants[0].price_delta_minor, 1_000);

    // i18n на варианте
    repo.set_translation(
        "p-1",
        "digital_variant",
        "v-1",
        Lang::Ru,
        "label",
        "Базовый",
    )
    .await
    .unwrap();
    let variants_ru = repo.list_variants("p-1", Lang::Ru).await.unwrap();
    assert_eq!(variants_ru[0].label, "Базовый");
    let variants_uk = repo.list_variants("p-1", Lang::Uk).await.unwrap();
    assert_eq!(variants_uk[0].label, "Базовий (оновл.)");

    repo.remove_variant("p-1", "v-2").await.unwrap();
    let variants = repo.list_variants("p-1", Lang::Uk).await.unwrap();
    assert_eq!(variants.len(), 1);
    assert_eq!(variants[0].id, "v-1");

    // media
    let m1 = ProductMedia {
        id: "m-1".to_string(),
        product_id: "p-1".to_string(),
        kind: MediaKind::Image,
        url: "https://example.test/cover.png".to_string(),
        position: 0,
    };
    repo.add_media(&m1).await.unwrap();
    let media = repo.list_media("p-1").await.unwrap();
    assert_eq!(media.len(), 1);
    assert_eq!(media[0].kind, MediaKind::Image);

    repo.remove_media("p-1", "m-1").await.unwrap();
    assert!(repo.list_media("p-1").await.unwrap().is_empty());

    // attribute values
    let attr = ProductAttributeValue {
        product_id: "p-1".to_string(),
        attribute_id: "a-level".to_string(),
        data_type: DataType::Enum,
        val_text: Some("beginner".to_string()),
        val_num: None,
    };
    repo.set_attribute_value(&attr).await.unwrap();
    let values = repo.list_attribute_values("p-1").await.unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].val_text.as_deref(), Some("beginner"));

    let mut attr_updated = attr.clone();
    attr_updated.val_text = Some("advanced".to_string());
    repo.set_attribute_value(&attr_updated).await.unwrap();
    let values = repo.list_attribute_values("p-1").await.unwrap();
    assert_eq!(values.len(), 1, "upsert заменяет значение, не дублирует");
    assert_eq!(values[0].val_text.as_deref(), Some("advanced"));

    repo.clear_attribute_value("p-1", "a-level").await.unwrap();
    assert!(repo.list_attribute_values("p-1").await.unwrap().is_empty());

    // ON DELETE CASCADE: вернём медиа (v1 и digital_config уже на месте) и удалим товар
    repo.add_media(&m1).await.unwrap();
    repo.delete_product("p-1").await.unwrap();

    assert!(repo.get_digital_config("p-1").await.unwrap().is_none());
    assert!(
        repo.list_variants("p-1", Lang::Uk)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(repo.list_media("p-1").await.unwrap().is_empty());
    assert!(repo.list_attribute_values("p-1").await.unwrap().is_empty());
}

#[tokio::test]
async fn delete_product_leaves_no_orphan_translations() {
    let t = temp_db("product-translations-orphans").await;
    let repo = ProductRepo::new(t.db.clone());

    let p = product("p-1", "seller-1", "Курс Rust", "rust-course", 50_000);
    repo.create_product(&p).await.unwrap();

    let v1 = DigitalVariant {
        id: "v-1".to_string(),
        product_id: "p-1".to_string(),
        label: "Базовий".to_string(),
        format: None,
        price_delta_minor: 0,
        position: 0,
    };
    repo.add_variant(&v1).await.unwrap();

    repo.set_translation(
        "p-1",
        "product",
        "p-1",
        Lang::Ru,
        "title",
        "Курс Rust (рос.)",
    )
    .await
    .unwrap();
    repo.set_translation(
        "p-1",
        "product",
        "p-1",
        Lang::Ru,
        "description",
        "Опис рос.",
    )
    .await
    .unwrap();
    repo.set_translation(
        "p-1",
        "digital_variant",
        "v-1",
        Lang::Ru,
        "label",
        "Базовый",
    )
    .await
    .unwrap();

    let count_translations = || {
        let pool = t.db.reader.clone();
        async move {
            sqlx::query("SELECT COUNT(*) AS n FROM translations")
                .fetch_one(&pool)
                .await
                .unwrap()
                .get::<i64, _>("n")
        }
    };
    use sqlx::Row;
    assert_eq!(count_translations().await, 3);

    repo.delete_product("p-1").await.unwrap();

    assert_eq!(
        count_translations().await,
        0,
        "после удаления товара не должно оставаться сирот в translations \
         (полиморфная таблица без FK на products/digital_variant)"
    );
}

#[tokio::test]
async fn list_products_by_seller_paginates_and_filters_by_status() {
    let t = temp_db("product-list").await;
    let repo = ProductRepo::new(t.db.clone());

    for i in 0..5 {
        let mut p = product(
            &format!("p-{i}"),
            "seller-1",
            &format!("Товар {i}"),
            &format!("item-{i}"),
            1_000 * (i as i64 + 1),
        );
        p.created_at = 1_000 + i as i64;
        p.updated_at = 1_000 + i as i64;
        repo.create_product(&p).await.unwrap();
    }
    // другой продавец — не должен попадать в выборку
    let other = product("p-other", "seller-2", "Чужий товар", "other", 1_000);
    repo.create_product(&other).await.unwrap();

    let all = repo
        .list_products_by_seller(
            "seller-1",
            None,
            shared::Pagination::clamped(0, 10),
            Lang::Uk,
        )
        .await
        .unwrap();
    assert_eq!(all.total, 5);
    assert_eq!(all.items.len(), 5);

    let page1 = repo
        .list_products_by_seller(
            "seller-1",
            None,
            shared::Pagination::clamped(0, 2),
            Lang::Uk,
        )
        .await
        .unwrap();
    assert_eq!(page1.items.len(), 2);
    assert_eq!(page1.total, 5);
    // сортировка по updated_at DESC — самый свежий (наибольший updated_at) первым
    assert_eq!(page1.items[0].id, "p-4");
    assert_eq!(page1.items[1].id, "p-3");

    let page2 = repo
        .list_products_by_seller(
            "seller-1",
            None,
            shared::Pagination::clamped(2, 2),
            Lang::Uk,
        )
        .await
        .unwrap();
    assert_eq!(page2.items.len(), 2);
    assert_eq!(page2.items[0].id, "p-2");
    assert_eq!(page2.items[1].id, "p-1");

    // смена статуса бьёт updated_at -> публикуем после проверки сортировки по исходным меткам
    repo.set_status("p-2", ProductStatus::Published)
        .await
        .unwrap();
    repo.set_status("p-3", ProductStatus::Published)
        .await
        .unwrap();

    let published = repo
        .list_products_by_seller(
            "seller-1",
            Some(ProductStatus::Published),
            shared::Pagination::clamped(0, 10),
            Lang::Uk,
        )
        .await
        .unwrap();
    assert_eq!(published.total, 2);
    assert!(
        published
            .items
            .iter()
            .all(|p| p.status == ProductStatus::Published)
    );
}
