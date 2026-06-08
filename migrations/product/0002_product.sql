-- Product: товар продавца + цифровая конфигурация (T1a-4) — design-1a.md §3 + i18n (ADR O5/T1a-4).
-- Владелец: продавец (истина о товаре). id — строковые (ULID), как outbox.aggregate_id.
-- foreign_keys=ON выставляется пулом (crates/db/src/lib.rs) → ON DELETE CASCADE активен.
-- seller_id / attribute_id — НЕпрозрачные ссылки на identity.sellers / catalog.attributes
--   (другие файлы БД) — без FK, целостность держит приложение/события.
-- i18n: канон title/description хранится прямо на products (язык uk); переводы (ru, ...) —
--   в локальной translations этого файла (резолв COALESCE(override, канон), как в catalog).

CREATE TABLE products (
  id          TEXT    PRIMARY KEY,
  seller_id   TEXT    NOT NULL,
  title       TEXT    NOT NULL,
  slug        TEXT    NOT NULL,
  description TEXT    NOT NULL DEFAULT '',
  price_minor INTEGER NOT NULL,
  currency    TEXT    NOT NULL DEFAULT 'UAH',
  status      TEXT    NOT NULL DEFAULT 'draft'
                CHECK (status IN ('draft','published','archived')),
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL
);
CREATE UNIQUE INDEX products_seller_slug ON products(seller_id, slug);
CREATE INDEX products_by_seller ON products(seller_id, status, updated_at DESC);

CREATE TABLE product_media (
  id         TEXT    PRIMARY KEY,
  product_id TEXT    NOT NULL REFERENCES products(id) ON DELETE CASCADE,
  kind       TEXT    NOT NULL CHECK (kind IN ('image','video')),
  url        TEXT    NOT NULL,
  position   INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX product_media_by_product ON product_media(product_id, position);

CREATE TABLE digital_config (
  product_id    TEXT PRIMARY KEY REFERENCES products(id) ON DELETE CASCADE,
  delivery_kind TEXT NOT NULL CHECK (delivery_kind IN ('download','platform_view')),
  license_kind  TEXT CHECK (license_kind IN ('single','multi') OR license_kind IS NULL),
  notes         TEXT
);

CREATE TABLE digital_variant (
  id          TEXT    PRIMARY KEY,
  product_id  TEXT    NOT NULL REFERENCES products(id) ON DELETE CASCADE,
  label       TEXT    NOT NULL,
  format      TEXT,
  price_delta_minor INTEGER NOT NULL DEFAULT 0,
  position    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX digital_variant_by_product ON digital_variant(product_id, position);

CREATE TABLE product_attribute_values (
  product_id   TEXT NOT NULL REFERENCES products(id) ON DELETE CASCADE,
  attribute_id TEXT NOT NULL,
  data_type    TEXT NOT NULL CHECK (data_type IN ('string','number','enum','bool')),
  val_text     TEXT,
  val_num      REAL,
  PRIMARY KEY (product_id, attribute_id)
);

CREATE TABLE translations (
  entity_type TEXT NOT NULL CHECK (entity_type IN ('product','digital_variant')),
  entity_id   TEXT NOT NULL,
  lang        TEXT NOT NULL CHECK (lang IN ('uk','ru')),
  field       TEXT NOT NULL CHECK (field IN ('title','description','label')),
  value       TEXT NOT NULL,
  PRIMARY KEY (entity_type, entity_id, lang, field)
);
-- PK покрывает точечный resolve и JOIN-условие как префиксную выборку — отдельный индекс не нужен.
