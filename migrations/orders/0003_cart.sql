-- Cart (T1b-1; design-1a.md §3). Anonymous, cart-token-based — no outbox events
-- (cart is private pre-order state; only checkout creates `orders`/`order_items`).
CREATE TABLE carts (
    token_hash TEXT    PRIMARY KEY,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE cart_items (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    cart_id          TEXT    NOT NULL REFERENCES carts(token_hash) ON DELETE CASCADE,
    product_id       TEXT    NOT NULL,
    variant_id       TEXT,
    qty              INTEGER NOT NULL CHECK (qty > 0),
    -- Denormalized display snapshot, taken at add-item time.
    title            TEXT    NOT NULL,
    unit_price_minor INTEGER NOT NULL,
    currency         TEXT    NOT NULL,
    added_at         INTEGER NOT NULL
);

CREATE INDEX cart_items_cart ON cart_items(cart_id);
-- table-level UNIQUE(cart_id, product_id, variant_id) НЕ годится — SQLite считает NULL != NULL,
-- и повторное добавление товара без варианта (variant_id IS NULL) проскочит мимо UNIQUE и
-- ON CONFLICT (см. тот же приём в migrations/catalog/0002_taxonomy.sql).
-- COALESCE сводит NULL к '' — конфликт ловится и для variant_id IS NULL.
CREATE UNIQUE INDEX cart_items_unique
    ON cart_items(cart_id, product_id, COALESCE(variant_id, ''));
