-- Orders skeleton (T1a-8; design-1a.md §3).
-- Checkout (Phase 1b) populates these via transactions; here we establish the schema contract.
CREATE TABLE orders (
    id          TEXT    PRIMARY KEY,
    buyer_id    TEXT    NOT NULL,
    status      TEXT    NOT NULL DEFAULT 'created'
                        CHECK (status IN ('created', 'paid', 'fulfilled', 'cancelled')),
    total_minor INTEGER NOT NULL DEFAULT 0,
    currency    TEXT    NOT NULL DEFAULT 'UAH',
    created_at  INTEGER NOT NULL
);

CREATE INDEX orders_buyer  ON orders(buyer_id);
CREATE INDEX orders_status ON orders(status);

CREATE TABLE order_items (
    id               TEXT    PRIMARY KEY,
    order_id         TEXT    NOT NULL REFERENCES orders(id),
    product_id       TEXT    NOT NULL,
    seller_id        TEXT    NOT NULL,
    variant_id       TEXT,
    title            TEXT    NOT NULL,
    unit_price_minor INTEGER NOT NULL,
    currency         TEXT    NOT NULL,
    config_snapshot  TEXT                -- immutable JSON snapshot of chosen options
);

CREATE INDEX order_items_order ON order_items(order_id);
