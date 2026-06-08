-- Catalog: таксономия (T1a-3) — design-1a.md §2.1 + i18n названий (ADR O5).
-- Владелец: площадка. id — строковые (ULID), как outbox.aggregate_id.
-- foreign_keys=ON выставляется пулом (crates/db/src/lib.rs), ON DELETE активен.
-- Базовое поле name/value хранит канон на языке-по-умолчанию (uk);
-- translations — только переопределения для прочих языков (ru, ...).

-- Дерево категорий. Materialized path '/root/sub/...' для хлебных крошек/поддерева.
CREATE TABLE categories (
  id        TEXT    PRIMARY KEY,
  parent_id TEXT    REFERENCES categories(id) ON DELETE CASCADE,
  name      TEXT    NOT NULL,                  -- канон (uk); ru — в translations
  slug      TEXT    NOT NULL,
  path      TEXT    NOT NULL,                  -- '/electronics/phones'
  position  INTEGER NOT NULL DEFAULT 0,
  UNIQUE (parent_id, slug),                    -- slug уникален в пределах родителя
  UNIQUE (path)                                -- путь уникален глобально
);

-- Атрибуты категории. data_type фиксирует тип значения (типизированный EAV).
CREATE TABLE attributes (
  id          TEXT    PRIMARY KEY,
  category_id TEXT    NOT NULL REFERENCES categories(id) ON DELETE CASCADE,
  name        TEXT    NOT NULL,                -- канон (uk); ru — в translations
  data_type   TEXT    NOT NULL CHECK (data_type IN ('string','number','enum','bool')),
  unit        TEXT,                            -- 'GB','kg',... технич., НЕ переводится
  is_required INTEGER NOT NULL DEFAULT 0 CHECK (is_required IN (0,1)),
  position    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX attributes_by_category ON attributes(category_id, position);

-- Допустимые значения для enum-атрибутов.
CREATE TABLE attribute_options (
  id           TEXT    PRIMARY KEY,
  attribute_id TEXT    NOT NULL REFERENCES attributes(id) ON DELETE CASCADE,
  value        TEXT    NOT NULL,              -- отображаемое имя опции, канон (uk)
  position     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX attribute_options_by_attribute ON attribute_options(attribute_id, position);

-- Привязка атрибута к категории как фильтра + его тип в UI.
CREATE TABLE filters (
  id           TEXT    PRIMARY KEY,
  category_id  TEXT    NOT NULL REFERENCES categories(id) ON DELETE CASCADE,
  attribute_id TEXT    NOT NULL REFERENCES attributes(id) ON DELETE CASCADE,
  filter_type  TEXT    NOT NULL CHECK (filter_type IN (
                 'checkbox_or','enum_and','string','number','range_price','range_generic')),
  position     INTEGER NOT NULL DEFAULT 0,
  UNIQUE (category_id, attribute_id)          -- один атрибут — один фильтр в категории
);
CREATE INDEX filters_by_category ON filters(category_id, position);

-- i18n переопределения названий. Полиморфно: ссылочную целостность держит приложение
-- (FK по entity_id невозможен). Базовое name/value на сущности = язык-по-умолчанию.
CREATE TABLE translations (
  entity_type TEXT NOT NULL CHECK (entity_type IN ('category','attribute','attribute_option')),
  entity_id   TEXT NOT NULL,
  lang        TEXT NOT NULL CHECK (lang IN ('uk','ru')),
  field       TEXT NOT NULL CHECK (field IN ('name','value')),
  value       TEXT NOT NULL,
  PRIMARY KEY (entity_type, entity_id, lang, field)
);
-- Lookup перевода для набора сущностей одного типа на одном языке (путь чтения списков).
CREATE INDEX translations_lookup ON translations(entity_type, lang, entity_id);
