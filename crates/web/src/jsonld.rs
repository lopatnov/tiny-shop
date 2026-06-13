//! Schema.org JSON-LD (`Product`, `Offer`, `BreadcrumbList`) — design-1a.md §6.
//!
//! Встраивается в `<head>` как `<script type="application/ld+json">`. Валюта — `UAH`.

use maud::{Markup, PreEscaped, html};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer<'a> {
    #[serde(rename = "@type")]
    pub type_: &'static str,
    /// Цена в гривнах, отформатированная как строка (`"199.00"`) — без `f64`,
    /// чтобы избежать погрешности округления при работе с минорными единицами.
    pub price: String,
    pub price_currency: &'a str,
    pub availability: &'static str,
}

#[derive(Serialize)]
pub struct Product<'a> {
    #[serde(rename = "@context")]
    pub context: &'static str,
    #[serde(rename = "@type")]
    pub type_: &'static str,
    pub name: &'a str,
    pub description: &'a str,
    pub image: Option<String>,
    pub offers: Offer<'a>,
}

#[derive(Serialize)]
pub struct ListItem {
    #[serde(rename = "@type")]
    pub type_: &'static str,
    pub position: u32,
    pub name: String,
    pub item: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BreadcrumbList {
    #[serde(rename = "@context")]
    pub context: &'static str,
    #[serde(rename = "@type")]
    pub type_: &'static str,
    pub item_list_element: Vec<ListItem>,
}

/// `Product`-сводка внутри `ItemListElement` (упрощённая — без `Offer`, см. [`ItemList`]).
#[derive(Serialize)]
pub struct ItemProduct {
    #[serde(rename = "@type")]
    pub type_: &'static str,
    pub name: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

/// Элемент `ItemList` — позиция + вложенный `Product`.
#[derive(Serialize)]
pub struct ItemListElement {
    #[serde(rename = "@type")]
    pub type_: &'static str,
    pub position: u32,
    pub item: ItemProduct,
}

/// Schema.org `ItemList` для страниц-листингов (`/c/{slug}`, design-1a.md §6).
///
/// Минимальный вариант без `Offer`/доступности — только название, ссылка и (опц.) изображение
/// каждого товара; этого достаточно для Rich Results карусели/списка.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemList {
    #[serde(rename = "@context")]
    pub context: &'static str,
    #[serde(rename = "@type")]
    pub type_: &'static str,
    pub item_list_element: Vec<ItemListElement>,
}

/// Перевести цену в минорных единицах (копейки) в строку гривен без `f64`
/// (целочисленная арифметика — без погрешности округления).
///
/// Знак форматируется отдельно, а деление/остаток берутся от модуля — иначе
/// для отрицательных `price_minor` Rust даёт некорректные строки вида `-1.-50`
/// (остаток от деления отрицательного числа в Rust тоже отрицательный).
pub fn format_price_minor(price_minor: i64) -> String {
    let sign = if price_minor < 0 { "-" } else { "" };
    let abs = price_minor.unsigned_abs();
    format!("{sign}{}.{:02}", abs / 100, abs % 100)
}

/// Построить абсолютный URL из `base_url` (без хвостового `/`) и пути,
/// начинающегося с `/`. Schema.org/Rich Results требуют абсолютные URL
/// в JSON-LD (изображения, элементы `BreadcrumbList`).
pub fn absolute_url(base_url: &str, path: &str) -> String {
    format!("{base_url}{path}")
}

/// Сериализовать значение в `<script type="application/ld+json">`.
///
/// `<` экранируется как `<` — защита от инъекции `</script>` внутрь JSON-LD
/// (XSS через преждевременное закрытие тега скрипта пользовательскими данными,
/// например названием товара).
pub fn jsonld_script<T: Serialize>(value: &T) -> Markup {
    let json = serde_json::to_string(value)
        .expect("jsonld_script: serialization failed for JSON-LD payload");
    let escaped = json.replace('<', "\\u003c");
    html! {
        script type="application/ld+json" {
            (PreEscaped(escaped))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_price_minor_positive() {
        assert_eq!(format_price_minor(19999), "199.99");
        assert_eq!(format_price_minor(5), "0.05");
        assert_eq!(format_price_minor(0), "0.00");
    }

    #[test]
    fn format_price_minor_negative() {
        assert_eq!(format_price_minor(-19999), "-199.99");
        assert_eq!(format_price_minor(-5), "-0.05");
    }

    #[test]
    fn absolute_url_joins_base_and_path() {
        assert_eq!(
            absolute_url("http://127.0.0.1:8080", "/p/blue-widget"),
            "http://127.0.0.1:8080/p/blue-widget"
        );
    }
}
