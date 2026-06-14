-- Guest checkout contact (T1b-2; design-1a.md §3). PII (email/name) isolated in its own
-- table so it doesn't bloat `orders`, and a future login-checkout order can simply have no
-- row here. Does NOT emit an outbox event — `OrderCreated` (from `orders`) already covers
-- order creation.
CREATE TABLE order_contact (
    order_id   TEXT    PRIMARY KEY REFERENCES orders(id) ON DELETE CASCADE,
    email      TEXT    NOT NULL,
    name       TEXT,
    created_at INTEGER NOT NULL
);
