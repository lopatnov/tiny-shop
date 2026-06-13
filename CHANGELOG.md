# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Workspace foundation (T1a-1)**: Cargo workspace with bounded-context crates (`shared`, `db`,
  `catalog`, `product`, `identity`, `orders`, `payments`, `web`, `tiny-shop`); per-context SQLite
  databases (WAL mode), migrations, transactional outbox/inbox tables, and an in-process outbox
  relay.
- **Port contracts (T1a-7)**: `Payments` and `Scanner` trait definitions (no provider adapters
  yet; `Scanner` defaults to a no-op implementation).
- **Catalog taxonomy (T1a-3)**: categories, attributes, attribute options, and filters schema
  with `uk`/`ru` i18n support and materialized category paths.
- **Product schema (T1a-4)**: products, media, and digital delivery configuration
  (`digital_config`/`digital_variant`) for seller-owned items.
- **Catalog projection & search (T1a-5)**: denormalized `product_projection` /
  `product_attr_index` tables updated via the outbox relay, plus a SQLite FTS5-backed
  `CatalogSearch` adapter.
- **Orders skeleton (T1a-8)**: `orders`/`order_items` schema with an immutable per-item
  configuration snapshot (price, currency, selected options) for future checkout.
- **Identity & access (T1a-2)**: account registration/login with Argon2id password hashing,
  server-side sessions backed by BLAKE3-hashed tokens, and `customer`/`seller`/`admin` roles.
- **SSR product page (T1a-6 chunk 1)**: Axum+maud `GET /p/{slug}` product page with semantic
  HTML, breadcrumb navigation, and `Product`/`Offer`/`BreadcrumbList` JSON-LD (Schema.org);
  the outbox relay task's `JoinHandle` is now tracked so a panic is logged instead of
  silently stopping the relay (restart-on-panic is a follow-up, not implemented here).

[Unreleased]: https://github.com/lopatnov/tiny-shop/compare/93ebb8e...HEAD
