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
- **SSR category page (T1a-6 chunk 2)**: Axum+maud `GET /c/{slug}` category listing page with
  a product grid, breadcrumb navigation, pagination (`?page=`), and `ItemList`/`BreadcrumbList`
  JSON-LD (Schema.org); `catalog::TaxonomyRepo::get_category_by_slug` resolves a category by its
  slug.
- **SSR home page, sitemap, robots.txt, brand assets (T1a-6 chunk 3)**: `GET /` home page
  linking to root categories; `GET /sitemap.xml` listing the home page, full category tree, and
  all published products; `GET /robots.txt` pointing at the sitemap; static brand assets
  (favicons, logo) served at `/assets/brand/*` via `tower-http::ServeDir`, plus a shared
  `<header>` with the site logo and favicon `<link>` tags added to `page_shell`.
- **Category filters (T1a-6 chunk 3)**: `GET /c/{slug}` now renders a `<form>` of
  category-specific facet/attribute filters (per `catalog::taxonomy::Filter`/`FilterType`) —
  `checkbox_or`/`enum_and` as `<fieldset>`s of checkboxes (`?attr_<attribute_id>=value`, repeatable),
  `range_generic` as numeric "from"/"to" inputs (`?attr_<attribute_id>_min`/`_max`), and
  `range_price` as a price range in major currency units (`?price_min`/`?price_max`, converted to
  `*_minor` for `catalog::FilterCond::RangePrice`). The form is a plain `GET` form (no JS
  required, WCAG 2.1 AA `<fieldset>`/`<legend>`/`<label>`), and active filters are carried through
  the "Попередня"/"Наступна" pagination links.
- **Anonymous cart (T1b-1)**: new `carts`/`cart_items` schema in `orders.db`
  (`migrations/orders/0003_cart.sql`) and `orders::CartRepo` (create/find-by-token, add item
  with increment-on-repeat, update quantity, remove, list, clear; quantities capped at
  `MAX_QTY = 999`). The cart is identified by an opaque cart-token cookie (`cart`, `HttpOnly`,
  `SameSite=Lax`, 30 days) whose BLAKE3 hash is stored server-side — same pattern as
  `identity::SessionRepo`; the cookie also gets a `Secure` attribute automatically when
  `TINY_SHOP_BASE_URL` is `https://`. New routes: `GET /cart` (view), `POST /cart/add` (from
  the product page's "Додати в кошик" form; `slug` + `qty`), `POST /cart/update` (`qty=0`
  removes the item), `POST /cart/remove`. The `tiny-shop` binary now opens/migrates
  `orders.db` and wires `CartRepo` into `AppState`.
- **Guest checkout (T1b-2)**: `orders::OrderRepo::checkout` turns cart items into an `Order` +
  `OrderItem[]` (status `created`, no payment yet) in one atomic transaction, taking a fresh
  price/`seller_id` snapshot from `catalog::SqliteCatalogSearch::get_card_by_id` rather than the
  cart's snapshot. Guest contact details (email + optional name) are stored in the new
  `order_contact` table (`migrations/orders/0004_order_contact.sql`), kept separate from
  `orders` so PII doesn't bloat the main table and isn't shown on the confirmation page. New
  routes: `GET /checkout` (order summary + contact form; redirects to `/cart` if empty),
  `POST /checkout` (validates the contact form, creates the order, clears the cart, expires the
  `cart` cookie, redirects to the confirmation page), `GET /checkout/done/{order_id}`
  (confirmation page with order number, items, and total). Guests get a synthetic
  `buyer_id` of the form `guest:<uuid>`; login-checkout is a separate follow-up.

### Security

- **CodeQL: scan GitHub Actions workflows**: `.github/workflows/codeql.yml` now also analyzes
  `.github/workflows/*.yml` with the `actions` CodeQL language (in addition to `rust`), catching
  supply-chain issues such as unpinned third-party action tags (`actions/unpinned-tag`) on future
  changes.

[Unreleased]: https://github.com/lopatnov/tiny-shop/compare/93ebb8e...HEAD
