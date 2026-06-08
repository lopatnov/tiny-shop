-- Transactional outbox для контекста Product (design-1a.md §1.2).
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
