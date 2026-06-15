# tiny-shop

A marketplace for digital goods, designed to run on low-resource hardware (target: an old
Celeron with ~6 GB RAM). Backend is a Rust modular monolith; persistence is per-context SQLite
files (WAL mode) with a transactional outbox + in-process relay for cross-context projections.

> Status: early development (Phase 1b — transactions, chunk 2: guest checkout). See "Current
> status & limitations" below before expecting a working storefront.

## Requirements

- Rust stable toolchain (edition 2024). No pinned `rust-toolchain` file yet — use the current
  `stable` channel (`rustup update stable`).
- SQLite is bundled via `sqlx`/`libsqlite3-sys`; no external database server is required.

## Build

```sh
cargo build --workspace
```

## Test, lint, format

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

## Run

```sh
cargo run -p tiny-shop
```

On startup the binary:
- creates the data directory (default `./data`) if it doesn't exist;
- opens/creates the per-context SQLite files (`catalog.db`, `product.db`, ...) inside it;
- applies pending migrations automatically;
- starts the in-process outbox relay (e.g. `product` → `catalog` projection);
- starts an Axum HTTP server.

### Environment variables

| Variable | Default | Purpose |
|---|---|---|
| `TINY_SHOP_DATA_DIR` | `./data` | Directory where SQLite database files are created/opened. |
| `TINY_SHOP_ADDR` | `127.0.0.1:8080` | Address/port the HTTP server binds to. |
| `TINY_SHOP_BASE_URL` | `http://127.0.0.1:8080` | Public base URL used to build absolute links/JSON-LD. |

## Current status & limitations

Phase 1a (foundation) is complete; Phase 1b (transactions) is underway. The server currently
exposes:
- `GET /` — home page with navigation links to root categories;
- `GET /p/{slug}` — product page with `Product`/`Offer`/`BreadcrumbList` JSON-LD and an
  "Додати в кошик" (add to cart) form;
- `GET /c/{slug}` — category listing page with a product grid, pagination (`?page=`),
  `ItemList`/`BreadcrumbList` JSON-LD, and a keyboard-operable `<form>` of category-specific
  facet/attribute filters (checkbox/multi-select via `?attr_<id>=`, numeric ranges via
  `?attr_<id>_min`/`_max`, and price range via `?price_min`/`?price_max`, all in major currency
  units); active filters are preserved across pagination links;
- `GET /cart` — view the current anonymous cart;
- `POST /cart/add` — add a product (`slug` + `qty`) to the cart, creating an anonymous cart
  (`cart` cookie, `HttpOnly`/`SameSite=Lax`, 30 days) on first use;
- `POST /cart/update` — change a cart item's quantity (`qty=0` removes the item);
- `POST /cart/remove` — remove a cart item;
- `GET /checkout` — order summary + guest contact form (email + optional name); redirects to
  `/cart` if the cart is empty;
- `POST /checkout` — validate the contact form, create the order (status `created`, no payment
  yet) from a fresh catalog price snapshot, clear the cart, and redirect to the confirmation
  page;
- `GET /checkout/done/{order_id}` — order confirmation page (order number, items, total);
- `GET /sitemap.xml` — sitemap listing the home page, the full category tree, and all
  published products;
- `GET /robots.txt` — crawler directives pointing at the sitemap;
- static brand assets (favicons, logo) served at `/assets/brand/*`.

**There is no seeding mechanism or admin UI yet.** Databases are empty on first run, so the
category/product pages will always return `404` until data is inserted manually (e.g. via tests
or direct SQL against `product.db`/`catalog.db`). This is expected at this stage — seller
onboarding, product creation UI, and digital delivery are planned for later phases. Guest
checkout creates real `created`-status orders, but there is no payment provider yet, so orders
cannot progress beyond `created`.

For the broader roadmap, see `.claude/backlog/roadmap.md`.

## Architecture

A modular monolith: one binary (`tiny-shop`), composed of bounded-context crates. Each context
owns its own SQLite file; cross-context communication goes through a transactional outbox and an
in-process relay (no shared transactions, no cross-file joins).

| Crate | Responsibility |
|---|---|
| `shared` | Core types: `Id<T>`, `Money`, `Email`, domain errors, event envelope. |
| `db` | SQLite pool setup (WAL/PRAGMA), migrations, outbox/inbox helpers, relay. |
| `identity` | Accounts, roles, sessions, sellers (Argon2id passwords, BLAKE3 session tokens). |
| `catalog` | Category/attribute/filter taxonomy, product projection, FTS5 search. |
| `product` | Seller-owned products and digital delivery configuration. |
| `orders` | Orders/order items, guest checkout (`OrderRepo::checkout`, `order_contact`), and anonymous cart (`carts`/`cart_items` in `orders.db`, cart-token cookie). |
| `payments` | Payment provider port/types (no adapter yet). |
| `web` | Axum + maud SSR pages, routing, JSON-LD. |
| `tiny-shop` | Binary: wires everything together and runs the server. |

For details on the SQLite-per-context layout, outbox/relay design, EAV catalog model, and SSR
SEO/a11y contract, see `.claude/docs/design-1a.md`.

## License

This project is licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE). Source is
open to view, but commercial use requires a separate license — see [COMMERCIAL.md](COMMERCIAL.md).
