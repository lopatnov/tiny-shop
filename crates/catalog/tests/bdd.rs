//! BDD-тесты: приёмочные критерии управления таксономией каталога (T1a-3).
//!
//! Формат: Gherkin / Given-When-Then через cucumber 0.21.
//! Feature-файлы: `tests/features/catalog_taxonomy.feature`.
//! Запуск: `cargo test --test bdd`

use std::sync::atomic::{AtomicUsize, Ordering};

use catalog::{Category, Lang, TaxonomyRepo};
use cucumber::{World, given, then, when};
use db::{migrate_catalog, open};

static BDD_CTR: AtomicUsize = AtomicUsize::new(0);

// ─── temporary database ──────────────────────────────────────────────────────

struct TempDb {
    path: std::path::PathBuf,
    pub repo: TaxonomyRepo,
}

impl std::fmt::Debug for TempDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TempDb({})", self.path.display())
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        for s in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(format!("{}{s}", self.path.display()));
        }
    }
}

async fn fresh_db() -> TempDb {
    let n = BDD_CTR.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    // `n` + `nanos` гарантируют уникальность без участия любых внешних данных.
    let path = std::env::temp_dir().join(format!("tinyshop-bdd-{nanos}-{n}.db"));
    let _ = std::fs::remove_file(&path);
    let db = open("bdd", &path).await.expect("open db");
    migrate_catalog(&db.writer).await.expect("migrate catalog");
    TempDb { path, repo: TaxonomyRepo::new(db) }
}

// ─── World ───────────────────────────────────────────────────────────────────

#[derive(Debug, World)]
#[world(init = Self::new)]
struct CatalogWorld {
    db: TempDb,
    last_error: Option<String>,
    last_root_id: String,
    resolved_name: String,
}

impl CatalogWorld {
    async fn new() -> Self {
        Self {
            db: fresh_db().await,
            last_error: None,
            last_root_id: String::new(),
            resolved_name: String::new(),
        }
    }
}

// ─── helpers ─────────────────────────────────────────────────────────────────

fn mk_root(id: &str, name: &str, slug: &str) -> Category {
    Category {
        id: id.to_string(),
        parent_id: None,
        name: name.to_string(),
        slug: slug.to_string(),
        path: format!("/{slug}"),
        position: 0,
    }
}

fn mk_sub(id: &str, parent_id: &str, name: &str, slug: &str, parent_slug: &str) -> Category {
    Category {
        id: id.to_string(),
        parent_id: Some(parent_id.to_string()),
        name: name.to_string(),
        slug: slug.to_string(),
        path: format!("/{parent_slug}/{slug}"),
        position: 0,
    }
}

// ─── steps ───────────────────────────────────────────────────────────────────

#[given("a fresh catalog")]
async fn fresh_catalog(_world: &mut CatalogWorld) {
    // World::new() already provisions a clean empty database per scenario.
}

#[when(regex = r#"I create a root category "([^"]+)" with slug "([^"]+)""#)]
async fn create_root(world: &mut CatalogWorld, name: String, slug: String) {
    let id = format!("cat-{slug}");
    world.last_root_id.clone_from(&id);
    let cat = mk_root(&id, &name, &slug);
    world.last_error = world.db.repo.create_category(&cat).await.err().map(|e| e.to_string());
}

#[given(regex = r#"a root category "([^"]+)" with slug "([^"]+)" exists"#)]
async fn given_root(world: &mut CatalogWorld, name: String, slug: String) {
    let id = format!("cat-{slug}");
    world.last_root_id.clone_from(&id);
    let cat = mk_root(&id, &name, &slug);
    world.db.repo.create_category(&cat).await.expect("setup: create root category");
}

#[when(regex = r#"I create a subcategory "([^"]+)" with slug "([^"]+)" under "([^"]+)""#)]
async fn create_sub(world: &mut CatalogWorld, name: String, slug: String, parent_name: String) {
    let parent_slug = parent_name.to_lowercase();
    let parent_id = format!("cat-{parent_slug}");
    let id = format!("cat-{slug}");
    let cat = mk_sub(&id, &parent_id, &name, &slug, &parent_slug);
    world.last_error = world.db.repo.create_category(&cat).await.err().map(|e| e.to_string());
}

#[then(regex = r"the catalog root has (\d+) categor")]
async fn root_count(world: &mut CatalogWorld, count: usize) {
    let roots = world.db.repo.list_categories_by_parent(None, Lang::Uk).await.unwrap();
    assert_eq!(roots.len(), count, "root category count");
}

#[then(regex = r#"the first root category is named "([^"]+)""#)]
async fn first_root_named(world: &mut CatalogWorld, name: String) {
    let roots = world.db.repo.list_categories_by_parent(None, Lang::Uk).await.unwrap();
    assert!(!roots.is_empty(), "no root categories found");
    assert_eq!(roots[0].name, name);
}

#[then(regex = r#""([^"]+)" has (\d+) child categor"#)]
async fn child_count(world: &mut CatalogWorld, parent_name: String, count: usize) {
    let parent_id = format!("cat-{}", parent_name.to_lowercase());
    let children = world
        .db
        .repo
        .list_categories_by_parent(Some(&parent_id), Lang::Uk)
        .await
        .unwrap();
    assert_eq!(children.len(), count, "child count for '{parent_name}'");
}

#[then(regex = r#"the child is named "([^"]+)""#)]
async fn child_named(world: &mut CatalogWorld, name: String) {
    let children = world
        .db
        .repo
        .list_categories_by_parent(Some(&world.last_root_id), Lang::Uk)
        .await
        .unwrap();
    assert!(
        children.iter().any(|c| c.name == name),
        "child '{name}' not found among {:?}",
        children.iter().map(|c| &c.name).collect::<Vec<_>>()
    );
}

#[when(regex = r#"I look up the category "([^"]+)" in (Ukrainian|Russian)"#)]
async fn look_up(world: &mut CatalogWorld, slug: String, lang_str: String) {
    let lang = if lang_str == "Russian" { Lang::Ru } else { Lang::Uk };
    let path = format!("/{slug}");
    let cat = world
        .db
        .repo
        .get_category_by_path(&path, lang)
        .await
        .unwrap()
        .expect("category exists");
    world.resolved_name.clone_from(&cat.name);
}

#[given(regex = r#"I add a Russian translation "([^"]+)" for category "([^"]+)""#)]
async fn add_ru_translation(world: &mut CatalogWorld, translation: String, slug: String) {
    let id = format!("cat-{slug}");
    world
        .db
        .repo
        .set_translation("category", &id, Lang::Ru, "name", &translation)
        .await
        .expect("set_translation");
}

#[then(regex = r#"the resolved name is "([^"]+)""#)]
async fn resolved_name_eq(world: &mut CatalogWorld, expected: String) {
    assert_eq!(world.resolved_name, expected);
}

#[when(regex = r#"I try to create another root category with slug "([^"]+)""#)]
async fn try_dup_root(world: &mut CatalogWorld, slug: String) {
    let cat = mk_root(&format!("cat-dup-{slug}"), "Duplicate", &slug);
    world.last_error = world.db.repo.create_category(&cat).await.err().map(|e| e.to_string());
}

#[then("the operation fails")]
async fn op_fails(world: &mut CatalogWorld) {
    assert!(
        world.last_error.is_some(),
        "expected an error but the operation succeeded"
    );
}

// ─── entry point ─────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    CatalogWorld::run("tests/features").await;
}
