# tiny-shop

A marketplace for digital goods, designed to run on low-resource hardware (target: an old
Celeron with ~6 GB RAM). Backend is a Rust modular monolith; persistence is per-context SQLite
files (WAL mode) with a transactional outbox + in-process relay for cross-context projections.

> Status: early development (Phase 1a — foundation). See "Current status & limitations" below
> before expecting a working storefront.

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

This is Phase 1a (foundation). The server currently exposes a single SSR page,
`GET /p/{slug}` (product page with `Product`/`Offer`/`BreadcrumbList` JSON-LD).

**There is no seeding mechanism or admin UI yet.** Databases are empty on first run, so
`/p/{slug}` will always return `404` until data is inserted manually (e.g. via tests or direct
SQL against `product.db`/`catalog.db`). This is expected at this stage — seller onboarding,
product creation UI, checkout, and digital delivery are planned for later phases.

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
| `orders` | Orders/order items skeleton (checkout logic comes later). |
| `payments` | Payment provider port/types (no adapter yet). |
| `web` | Axum + maud SSR pages, routing, JSON-LD. |
| `tiny-shop` | Binary: wires everything together and runs the server. |

For details on the SQLite-per-context layout, outbox/relay design, EAV catalog model, and SSR
SEO/a11y contract, see `.claude/docs/design-1a.md`.

## License

This project is licensed under the [PolyForm Noncommercial License 1.0.0](LICENSE). Source is
open to view, but commercial use requires a separate license — see [COMMERCIAL.md](COMMERCIAL.md).
