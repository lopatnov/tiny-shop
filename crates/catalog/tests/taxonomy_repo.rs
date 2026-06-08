//! Интеграционные тесты T1a-3: дерево категорий, атрибуты/опции/фильтры, i18n-резолв,
//! ограничения уникальности (`design-1a.md` §2.1, ADR O5).

use db::{ContextDb, migrate_catalog, open};

use catalog::{
    Attribute, AttributeOption, Category, DataType, Filter, FilterType, Lang, TaxonomyRepo,
};

use std::sync::atomic::{AtomicUsize, Ordering};

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
    migrate_catalog(&db.writer).await.expect("migrate");
    TempDb { path, db }
}

fn category(id: &str, parent_id: Option<&str>, name: &str, slug: &str, path: &str) -> Category {
    Category {
        id: id.to_string(),
        parent_id: parent_id.map(str::to_string),
        name: name.to_string(),
        slug: slug.to_string(),
        path: path.to_string(),
        position: 0,
    }
}

#[tokio::test]
async fn category_tree_create_and_read() {
    let t = temp_db("cat-tree").await;
    let repo = TaxonomyRepo::new(t.db.clone());

    let root = category(
        "c-electronics",
        None,
        "Електроніка",
        "electronics",
        "/electronics",
    );
    let child = category(
        "c-phones",
        Some("c-electronics"),
        "Телефони",
        "phones",
        "/electronics/phones",
    );
    repo.create_category(&root).await.unwrap();
    repo.create_category(&child).await.unwrap();

    // чтение по id
    let got = repo
        .get_category("c-phones", Lang::Uk)
        .await
        .unwrap()
        .expect("found");
    assert_eq!(got.name, "Телефони");
    assert_eq!(got.parent_id.as_deref(), Some("c-electronics"));

    // чтение по materialized path
    let by_path = repo
        .get_category_by_path("/electronics/phones", Lang::Uk)
        .await
        .unwrap()
        .expect("found by path");
    assert_eq!(by_path.id, "c-phones");

    // дети узла
    let roots = repo
        .list_categories_by_parent(None, Lang::Uk)
        .await
        .unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].id, "c-electronics");

    let children = repo
        .list_categories_by_parent(Some("c-electronics"), Lang::Uk)
        .await
        .unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].id, "c-phones");

    // несуществующий id/path
    assert!(
        repo.get_category("missing", Lang::Uk)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        repo.get_category_by_path("/no/such/path", Lang::Uk)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn parent_slug_uniqueness_is_enforced() {
    let t = temp_db("cat-slug-uniq").await;
    let repo = TaxonomyRepo::new(t.db.clone());

    let root = category("c-root", None, "Корінь", "root", "/root");
    repo.create_category(&root).await.unwrap();

    let a = category("c-a", Some("c-root"), "Категорія А", "dup", "/root/a");
    let b = category("c-b", Some("c-root"), "Категорія Б", "dup", "/root/b");
    repo.create_category(&a).await.unwrap();

    let err = repo.create_category(&b).await.unwrap_err();
    assert!(
        matches!(err, catalog::TaxonomyError::Db(_)),
        "нарушение уникальности slug в пределах родителя должно дать ошибку БД: {err:?}"
    );
}

#[tokio::test]
async fn root_level_slug_uniqueness_is_enforced_despite_null_parent() {
    // SQLite считает NULL != NULL — table-level UNIQUE(parent_id, slug) пропустил бы дубль
    // среди корневых категорий. Индекс на COALESCE(parent_id, '') обязан его поймать.
    let t = temp_db("cat-root-slug-uniq").await;
    let repo = TaxonomyRepo::new(t.db.clone());

    let a = category("c-root-a", None, "Корінь А", "root", "/root-a");
    let b = category("c-root-b", None, "Корінь Б", "root", "/root-b");
    repo.create_category(&a).await.unwrap();

    let err = repo.create_category(&b).await.unwrap_err();
    assert!(
        matches!(err, catalog::TaxonomyError::Db(_)),
        "дубль slug среди корневых категорий (parent_id IS NULL) должен дать ошибку БД: {err:?}"
    );
}

#[tokio::test]
async fn attribute_option_filter_crud_and_unique_filter_pair() {
    let t = temp_db("attr-crud").await;
    let repo = TaxonomyRepo::new(t.db.clone());

    let cat = category("c-phones", None, "Телефони", "phones", "/phones");
    repo.create_category(&cat).await.unwrap();

    let attr = Attribute {
        id: "a-color".to_string(),
        category_id: "c-phones".to_string(),
        name: "Колір".to_string(),
        data_type: DataType::Enum,
        unit: None,
        is_required: true,
        position: 0,
    };
    repo.create_attribute(&attr).await.unwrap();

    let attrs = repo
        .list_attributes_by_category("c-phones", Lang::Uk)
        .await
        .unwrap();
    assert_eq!(attrs.len(), 1);
    assert_eq!(attrs[0].name, "Колір");
    assert_eq!(attrs[0].data_type, DataType::Enum);
    assert!(attrs[0].is_required);

    let opt_black = AttributeOption {
        id: "o-black".to_string(),
        attribute_id: "a-color".to_string(),
        value: "Чорний".to_string(),
        position: 0,
    };
    let opt_white = AttributeOption {
        id: "o-white".to_string(),
        attribute_id: "a-color".to_string(),
        value: "Білий".to_string(),
        position: 1,
    };
    repo.create_attribute_option(&opt_black).await.unwrap();
    repo.create_attribute_option(&opt_white).await.unwrap();

    let options = repo
        .list_attribute_options("a-color", Lang::Uk)
        .await
        .unwrap();
    assert_eq!(options.len(), 2);
    assert_eq!(options[0].value, "Чорний");
    assert_eq!(options[1].value, "Білий");

    let filter = Filter {
        id: "f-color".to_string(),
        category_id: "c-phones".to_string(),
        attribute_id: "a-color".to_string(),
        filter_type: FilterType::EnumAnd,
        position: 0,
    };
    repo.create_filter(&filter).await.unwrap();

    let filters = repo.list_filters_by_category("c-phones").await.unwrap();
    assert_eq!(filters.len(), 1);
    assert_eq!(filters[0].filter_type, FilterType::EnumAnd);

    // UNIQUE(category_id, attribute_id) — повторная привязка того же атрибута запрещена
    let dup_filter = Filter {
        id: "f-color-2".to_string(),
        category_id: "c-phones".to_string(),
        attribute_id: "a-color".to_string(),
        filter_type: FilterType::CheckboxOr,
        position: 1,
    };
    let err = repo.create_filter(&dup_filter).await.unwrap_err();
    assert!(
        matches!(err, catalog::TaxonomyError::Db(_)),
        "нарушение UNIQUE(category_id, attribute_id) должно дать ошибку БД: {err:?}"
    );
}

#[tokio::test]
async fn translation_resolves_with_coalesce_fallback() {
    let t = temp_db("i18n").await;
    let repo = TaxonomyRepo::new(t.db.clone());

    let cat = category("c-books", None, "Книги", "books", "/books");
    repo.create_category(&cat).await.unwrap();

    // без перевода — fallback на канон (uk) при чтении на ru
    let got = repo
        .get_category("c-books", Lang::Ru)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.name, "Книги", "без override читаем канон (uk)");

    // явный резолв с fallback
    let resolved = repo
        .resolve_translation("category", "c-books", Lang::Ru, "name", "Книги")
        .await
        .unwrap();
    assert_eq!(resolved, "Книги");

    // добавляем перевод на ru
    repo.set_translation("category", "c-books", Lang::Ru, "name", "Книги (рус)")
        .await
        .unwrap();

    let got_ru = repo
        .get_category("c-books", Lang::Ru)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got_ru.name, "Книги (рус)", "теперь читаем override");

    // канон (uk) не должен зависеть от наличия ru-перевода
    let got_uk = repo
        .get_category("c-books", Lang::Uk)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got_uk.name, "Книги");

    // обновление перевода (upsert) заменяет значение
    repo.set_translation("category", "c-books", Lang::Ru, "name", "Книги (обновлено)")
        .await
        .unwrap();
    let got_ru2 = repo
        .get_category("c-books", Lang::Ru)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got_ru2.name, "Книги (обновлено)");

    let resolved2 = repo
        .resolve_translation("category", "c-books", Lang::Ru, "name", "Книги")
        .await
        .unwrap();
    assert_eq!(resolved2, "Книги (обновлено)");
}
