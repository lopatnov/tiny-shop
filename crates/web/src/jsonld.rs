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
    pub image: Option<&'a str>,
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
pub struct BreadcrumbList {
    #[serde(rename = "@context")]
    pub context: &'static str,
    #[serde(rename = "@type")]
    pub type_: &'static str,
    pub item_list_element: Vec<ListItem>,
}

/// Перевести цену в минорных единицах (копейки) в строку гривен без `f64`
/// (целочисленная арифметика — без погрешности округления).
pub fn format_price_minor(price_minor: i64) -> String {
    format!("{}.{:02}", price_minor / 100, price_minor % 100)
}

/// Сериализовать значение в `<script type="application/ld+json">`.
///
/// `<` экранируется как `<` — защита от инъекции `</script>` внутрь JSON-LD
/// (XSS через преждевременное закрытие тега скрипта пользовательскими данными,
/// например названием товара).
pub fn jsonld_script<T: Serialize>(value: &T) -> Markup {
    let json = serde_json::to_string(value).unwrap_or_default();
    let escaped = json.replace('<', "\\u003c");
    html! {
        script type="application/ld+json" {
            (PreEscaped(escaped))
        }
    }
}
