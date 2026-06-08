-- Catalog projection (T1a-5) — design-1a.md §1.4, §2.2, §2.3.
-- Обновляется по событиям Product* из product context (eventual consistency, relay).
-- Три таблицы: product_projection (карточки листинга), product_attr_index (типизированный EAV
-- для фасетных фильтров), product_fts (полнотекстовый поиск через FTS5).

-- Денормализованная проекция товара. Обновляется событиями ProductCreated/Updated/Published…/Deleted.
-- category_id — NULL до явного назначения через CatalogSearch::upsert(); фильтрация по категории
-- работает через product_attr_index.category_id (атрибут принадлежит категории).
CREATE TABLE product_projection (
  id           TEXT    PRIMARY KEY,
  seller_id    TEXT    NOT NULL DEFAULT '',
  title        TEXT    NOT NULL,
  slug         TEXT    NOT NULL,
  description  TEXT    NOT NULL DEFAULT '',
  price_minor  INTEGER NOT NULL DEFAULT 0,
  currency     TEXT    NOT NULL DEFAULT 'UAH',
  status       TEXT    NOT NULL DEFAULT 'draft',  -- draft|published|archived
  category_id  TEXT,                              -- NULL until assigned via upsert(ProductDoc)
  thumb        TEXT,                              -- URL первой медиа (NULL до добавления)
  created_at   INTEGER NOT NULL DEFAULT 0,
  updated_at   INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX pp_status_price   ON product_projection(status, price_minor);
CREATE INDEX pp_status_created ON product_projection(status, created_at DESC);
CREATE UNIQUE INDEX pp_slug    ON product_projection(slug);

-- Денормализованные значения атрибутов для быстрых фасетных фильтров.
-- category_id здесь — категория атрибута (из catalog.db.attributes), не товара.
-- Обновляется событиями ProductUpdated{reason:"attribute_value_set/cleared"}.
CREATE TABLE product_attr_index (
  product_id   TEXT NOT NULL,
  category_id  TEXT NOT NULL,
  attribute_id TEXT NOT NULL,
  val_text     TEXT,
  val_num      REAL,
  PRIMARY KEY (product_id, attribute_id)
);

-- Составной индекс: checkbox_or, enum_and, string (IN + val_text), number/range (val_num).
CREATE INDEX pai_filter   ON product_attr_index(attribute_id, val_text, val_num, product_id);
CREATE INDEX pai_num      ON product_attr_index(attribute_id, val_num) WHERE val_num IS NOT NULL;
CREATE INDEX pai_category ON product_attr_index(category_id, attribute_id);

-- Полнотекстовый поиск (FTS5). product_id — UNINDEXED: хранится, но не FTS-индексируется;
-- нужен для поиска rowid при удалении (полный скан приемлем для каталога малого масштаба).
-- Зависимость от товаров: FTS уделяет приоритет title > description > attrs.
CREATE VIRTUAL TABLE product_fts USING fts5(
  product_id  UNINDEXED,
  title,
  description,
  attrs,
  tokenize    = 'unicode61'
);
