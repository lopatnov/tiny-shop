-- Контекст Catalog: и источник событий (outbox), и главный потребитель проекций (inbox).
-- design-1a.md §1.2.
CREATE TABLE outbox (
  id           INTEGER PRIMARY KEY AUTOINCREMENT,
  aggregate    TEXT    NOT NULL,
  aggregate_id TEXT    NOT NULL,
  event_type   TEXT    NOT NULL,
  payload      TEXT    NOT NULL,
  created_at   INTEGER NOT NULL,
  published_at INTEGER
);
CREATE INDEX outbox_unpublished ON outbox(id) WHERE published_at IS NULL;

-- Идемпотентность потребления событий из других контекстов.
CREATE TABLE inbox_processed (
  source       TEXT    NOT NULL,   -- контекст-источник, напр. 'product'
  event_id     INTEGER NOT NULL,   -- outbox.id источника
  processed_at INTEGER NOT NULL,
  PRIMARY KEY (source, event_id)
);
