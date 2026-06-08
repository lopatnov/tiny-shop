//! Доменные типы товара продавца + цифровой конфигурации (T1a-4) — design-1a.md §3 + i18n
//! (ADR O5/T1a-4).
//!
//! Схема — `migrations/product/0002_product.sql`. Канон title/description/label хранится
//! на `uk` непосредственно в полях сущностей; переводы на другие языки — в `translations`
//! (резолв см. [`crate::repository`]).

/// Поддерживаемые языки интерфейса (i18n названий товара, ADR O5/T1a-4).
///
/// Локальная копия `catalog::Lang` — контекст самодостаточен, кросс-крейтовая зависимость
/// product → catalog не нужна (правило bounded contexts: контексты не делят типы домена).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Uk,
    Ru,
}

impl Lang {
    pub fn as_str(self) -> &'static str {
        match self {
            Lang::Uk => "uk",
            Lang::Ru => "ru",
        }
    }
}

/// Статус товара (жизненный цикл публикации).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProductStatus {
    Draft,
    Published,
    Archived,
}

impl ProductStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            ProductStatus::Draft => "draft",
            ProductStatus::Published => "published",
            ProductStatus::Archived => "archived",
        }
    }

    /// См. [`crate::product::DataType::parse`] — та же терпимость к неожиданным строкам:
    /// CHECK в БД должен предотвращать мусор, но парсер обязан вернуть `None`, а не паниковать.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(ProductStatus::Draft),
            "published" => Some(ProductStatus::Published),
            "archived" => Some(ProductStatus::Archived),
            _ => None,
        }
    }
}

/// Способ выдачи цифрового товара.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryKind {
    Download,
    PlatformView,
}

impl DeliveryKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DeliveryKind::Download => "download",
            DeliveryKind::PlatformView => "platform_view",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "download" => Some(DeliveryKind::Download),
            "platform_view" => Some(DeliveryKind::PlatformView),
            _ => None,
        }
    }
}

/// Тип лицензии цифрового товара.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LicenseKind {
    Single,
    Multi,
}

impl LicenseKind {
    pub fn as_str(self) -> &'static str {
        match self {
            LicenseKind::Single => "single",
            LicenseKind::Multi => "multi",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "single" => Some(LicenseKind::Single),
            "multi" => Some(LicenseKind::Multi),
            _ => None,
        }
    }
}

/// Тип медиа-вложения товара.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaKind {
    Image,
    Video,
}

impl MediaKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MediaKind::Image => "image",
            MediaKind::Video => "video",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "image" => Some(MediaKind::Image),
            "video" => Some(MediaKind::Video),
            _ => None,
        }
    }
}

/// Тип значения атрибута товара (типизированный EAV, design-1a.md §3) — общий технический
/// value-type, см. [`shared::DataType`] (вынесен туда из дублировавшихся локальных копий
/// в `catalog`/`product`; это не доменная сущность контекста, изоляция bounded contexts
/// не нарушена).
pub use shared::DataType;

/// Товар продавца — единица каталога/витрины (истина о товаре принадлежит продавцу).
#[derive(Debug, Clone, PartialEq)]
pub struct Product {
    pub id: String,
    pub seller_id: String,
    pub title: String,
    pub slug: String,
    pub description: String,
    pub price_minor: i64,
    pub currency: String,
    pub status: ProductStatus,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Медиа-вложение товара (изображение/видео), упорядоченное по `position`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductMedia {
    pub id: String,
    pub product_id: String,
    pub kind: MediaKind,
    pub url: String,
    pub position: i64,
}

/// Цифровая конфигурация товара (1:1 с товаром, способ выдачи + лицензия).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigitalConfig {
    pub product_id: String,
    pub delivery_kind: DeliveryKind,
    pub license_kind: Option<LicenseKind>,
    pub notes: Option<String>,
}

/// Вариант цифрового товара (формат/издание), модифицирующий цену через дельту.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigitalVariant {
    pub id: String,
    pub product_id: String,
    pub label: String,
    pub format: Option<String>,
    pub price_delta_minor: i64,
    pub position: i64,
}

/// Значение атрибута товара (типизированный EAV, перенесённое из таксономии каталога).
#[derive(Debug, Clone, PartialEq)]
pub struct ProductAttributeValue {
    pub product_id: String,
    pub attribute_id: String,
    pub data_type: DataType,
    pub val_text: Option<String>,
    pub val_num: Option<f64>,
}
