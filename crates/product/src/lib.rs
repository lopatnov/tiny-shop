//! `product` — контекст товара продавца + цифровая конфигурация (формат/опции/способ выдачи).
//!
//! Модель и репозиторий — T1a-4 (см. `.claude/docs/design-1a.md` §3 + ADR O5/T1a-4 для i18n).
//! Схема — `migrations/product/0002_product.sql`. Модули: [`product`] (доменные типы),
//! [`repository`] (CRUD товара, статусы, листинг), [`extras`] (digital-конфигурация/варианты/
//! медиа/атрибуты/переводы — вынесены ради размера файла, см. `repository.rs`).

pub mod extras;
pub mod product;
pub mod repository;

pub use product::{
    DataType, DeliveryKind, DigitalConfig, DigitalVariant, Lang, LicenseKind, MediaKind, Product,
    ProductAttributeValue, ProductMedia, ProductStatus,
};
pub use repository::{ProductError, ProductRepo};
