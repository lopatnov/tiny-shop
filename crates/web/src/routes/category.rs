//! `GET /c/{slug}` — страница листинга категории (design-1a.md §6: `ItemList`+`BreadcrumbList`).
//!
//! ## Фільтри (T1a-6 chunk 3)
//!
//! Конвенція query-параметрів (форма `GET`, без JS):
//! - `attr_<attribute_id>` (можна повторювати) — обрані значення для `checkbox_or`/`enum_and`.
//! - `attr_<attribute_id>_min` / `attr_<attribute_id>_max` — межі діапазону для `range_generic`
//!   (число, `f64`; нерозбірні значення ігноруються).
//! - `price_min` / `price_max` — межі ціни для `range_price`, **у гривнях** (як показується
//!   користувачу), переводяться у мінорні одиниці (`* 100`) для [`catalog::FilterCond::RangePrice`].
//!
//! `FilterType::String`/`FilterType::Number` поки не підтримуються — пропускаються без помилки
//! (наперед-сумісний `_ =>`, на випадок майбутніх типів фільтрів).

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::response::{Html, IntoResponse, Response};
use catalog::{Attribute, CatalogSearch, Filter, FilterCond, FilterType, Lang, SearchQuery, Sort};
use maud::{Markup, html};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Deserialize;
use shared::Pagination;

use crate::AppState;
use crate::error::WebError;
use crate::jsonld::{
    self, ItemList, ItemListElement, ItemProduct, absolute_url, breadcrumb_list_ld, jsonld_script,
};
use crate::view::breadcrumb::breadcrumb_nav;
use crate::view::layout::page_shell;

#[derive(Debug, Deserialize)]
pub struct ListingQuery {
    page: Option<u32>,
}

/// Обработчик `GET /c/{slug}`.
pub async fn show(
    State(state): State<AppState>,
    Path(slug): Path<String>,
    Query(query): Query<ListingQuery>,
    Query(raw): Query<Vec<(String, String)>>,
) -> Response {
    match render(&state, &slug, query.page.unwrap_or(1).max(1), &raw).await {
        Ok(html) => Html(html).into_response(),
        Err(err) => err.into_response(),
    }
}

async fn render(
    state: &AppState,
    slug: &str,
    page: u32,
    raw: &[(String, String)],
) -> Result<String, WebError> {
    let category = state
        .taxonomy
        .get_category_by_slug(slug, Lang::Uk)
        .await
        .map_err(|e| WebError::Internal(e.to_string()))?
        .ok_or(WebError::NotFound)?;

    // Хлебные крошки: "Головна" → (батьківська категорія, якщо є) → назва категорії (поточна).
    let mut crumbs: Vec<(String, Option<String>)> =
        vec![("Головна".to_string(), Some("/".to_string()))];
    if let Some(parent_id) = &category.parent_id {
        let parent = state
            .taxonomy
            .get_category(parent_id, Lang::Uk)
            .await
            .map_err(|e| WebError::Internal(e.to_string()))?;
        if let Some(parent) = parent {
            crumbs.push((parent.name, Some(format!("/c/{}", parent.slug))));
        }
    }
    crumbs.push((category.name.clone(), None));

    let filters = state
        .taxonomy
        .list_filters_by_category(&category.id)
        .await
        .map_err(|e| WebError::Internal(e.to_string()))?;

    let limit = Pagination::DEFAULT_LIMIT;
    let offset = (page as u64)
        .saturating_sub(1)
        .saturating_mul(limit as u64)
        .try_into()
        .unwrap_or(u32::MAX);
    let pagination = Pagination::clamped(offset, limit);

    let result = state
        .search
        .search(&SearchQuery {
            text: None,
            category_id: Some(category.id.clone()),
            filters: build_filter_conds(&filters, raw),
            sort: Sort::default(),
            page: pagination,
        })
        .await
        .map_err(|e| WebError::Internal(e.to_string()))?;

    let breadcrumb_ld = breadcrumb_list_ld(&state.base_url, &crumbs, &format!("/c/{slug}"));

    let item_list_ld = ItemList {
        context: "https://schema.org",
        type_: "ItemList",
        item_list_element: result
            .items
            .iter()
            .enumerate()
            .map(|(i, item)| ItemListElement {
                type_: "ListItem",
                position: (i + 1) as u32,
                item: ItemProduct {
                    type_: "Product",
                    name: item.title.clone(),
                    url: absolute_url(&state.base_url, &format!("/p/{}", item.slug)),
                    image: item
                        .thumb
                        .as_deref()
                        .map(|thumb| absolute_url(&state.base_url, thumb)),
                },
            })
            .collect(),
    };

    let head_extra = html! {
        (jsonld_script(&breadcrumb_ld))
        @if !result.items.is_empty() {
            (jsonld_script(&item_list_ld))
        }
    };

    let filter_form = if filters.is_empty() {
        None
    } else {
        let attributes = state
            .taxonomy
            .list_attributes_by_category(&category.id, Lang::Uk)
            .await
            .map_err(|e| WebError::Internal(e.to_string()))?;
        let attrs_by_id: HashMap<&str, &Attribute> =
            attributes.iter().map(|a| (a.id.as_str(), a)).collect();
        render_filter_form(state, slug, &filters, &attrs_by_id, raw).await?
    };

    let has_prev = pagination.offset > 0;
    let has_next = (pagination.offset as u64 + result.items.len() as u64) < result.total;

    let main = html! {
        (breadcrumb_nav(&crumbs))
        h1 { (category.name) }
        @if let Some(form) = &filter_form {
            (form)
        }
        @if result.items.is_empty() {
            p { "У цій категорії ще немає товарів." }
        } @else {
            ul {
                @for item in &result.items {
                    li {
                        a href=(format!("/p/{}", item.slug)) {
                            @if let Some(thumb) = &item.thumb {
                                img src=(thumb) alt=(item.title);
                            }
                            span { (item.title) }
                        }
                        p {
                            (jsonld::format_price_minor(item.price_minor)) " " (item.currency)
                        }
                    }
                }
            }
            nav aria-label="Сторінки" {
                @if has_prev {
                    a href=(pagination_href(slug, raw, page - 1)) { "Попередня" }
                }
                @if has_next {
                    a href=(pagination_href(slug, raw, page.saturating_add(1))) { "Наступна" }
                }
            }
        }
    };

    Ok(page_shell(&category.name, head_extra, main).into_string())
}

// -----------------------------------------------------------------
// Разбор query-параметров → FilterCond
// -----------------------------------------------------------------

/// Все значения параметра `key` (для повторяемых `attr_<id>=...`), в порядке появления.
fn values_for<'a>(raw: &'a [(String, String)], key: &str) -> Vec<&'a str> {
    raw.iter()
        .filter(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .collect()
}

/// Первое значение параметра `key`, распознанное как `f64` (нерозбірні значення — `None`).
fn parsed_f64(raw: &[(String, String)], key: &str) -> Option<f64> {
    raw.iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| v.trim().parse::<f64>().ok())
}

/// Значения повторяемого параметра `attr_<attribute_id>`, или `None`, если ни одного не задано
/// (для `checkbox_or`/`enum_and` — отсутствие значений means no-op).
fn checkbox_values(raw: &[(String, String)], attribute_id: &str) -> Option<Vec<String>> {
    let values = values_for(raw, &format!("attr_{attribute_id}"));
    if values.is_empty() {
        None
    } else {
        Some(values.into_iter().map(str::to_string).collect())
    }
}

/// Границы диапазона из `min_key`/`max_key`, или `None`, если обе отсутствуют/нерозбірні
/// (для `range_generic`/`range_price` — отсутствие обеих границ means no-op).
fn range_bounds(
    raw: &[(String, String)],
    min_key: &str,
    max_key: &str,
) -> Option<(Option<f64>, Option<f64>)> {
    let min = parsed_f64(raw, min_key);
    let max = parsed_f64(raw, max_key);
    (min.is_some() || max.is_some()).then_some((min, max))
}

/// Условие фильтрации для одного [`Filter`] из query-параметров, или `None` — нет значений в
/// `raw`, либо `FilterType::String`/`FilterType::Number` (пока не підтримуються).
fn filter_cond_for(filter: &Filter, raw: &[(String, String)]) -> Option<FilterCond> {
    match filter.filter_type {
        FilterType::CheckboxOr => {
            checkbox_values(raw, &filter.attribute_id).map(|values| FilterCond::CheckboxOr {
                attribute_id: filter.attribute_id.clone(),
                values,
            })
        }
        FilterType::EnumAnd => {
            checkbox_values(raw, &filter.attribute_id).map(|values| FilterCond::EnumAnd {
                attribute_id: filter.attribute_id.clone(),
                values,
            })
        }
        FilterType::RangeGeneric => {
            let min_key = format!("attr_{}_min", filter.attribute_id);
            let max_key = format!("attr_{}_max", filter.attribute_id);
            let (min, max) = range_bounds(raw, &min_key, &max_key)?;
            Some(FilterCond::RangeGeneric {
                attribute_id: filter.attribute_id.clone(),
                min,
                max,
            })
        }
        FilterType::RangePrice => {
            let (min, max) = range_bounds(raw, "price_min", "price_max")?;
            Some(FilterCond::RangePrice {
                min_minor: min.map(major_to_minor),
                max_minor: max.map(major_to_minor),
            })
        }
        FilterType::String | FilterType::Number => None,
    }
}

/// Построить условия фильтрации для [`SearchQuery::filters`] из конфигурации фильтров
/// категории и query-параметров. Фильтры без значений в `raw` не добавляются (no-op).
fn build_filter_conds(filters: &[Filter], raw: &[(String, String)]) -> Vec<FilterCond> {
    filters
        .iter()
        .filter_map(|filter| filter_cond_for(filter, raw))
        .collect()
}

/// Перевести гривны (как вводит пользователь) в минорные единицы (копейки), округляя до целого —
/// см. конвенцию `*_minor: i64` (design-1a.md, `jsonld::format_price_minor`).
fn major_to_minor(major: f64) -> i64 {
    (major * 100.0).round() as i64
}

// -----------------------------------------------------------------
// Рендер формы фильтров
// -----------------------------------------------------------------

/// Построить `<form>` фильтров категории (WCAG 2.1 AA: `<fieldset>`/`<legend>`/`<label>`,
/// `GET`-форма без JS). `None`, если ни один фильтр категории не дал renderable `<fieldset>`
/// (например, все — пока неподдерживаемых `FilterType::String`/`Number`).
async fn render_filter_form(
    state: &AppState,
    slug: &str,
    filters: &[Filter],
    attrs_by_id: &HashMap<&str, &Attribute>,
    raw: &[(String, String)],
) -> Result<Option<Markup>, WebError> {
    let mut fieldsets = Vec::with_capacity(filters.len());
    for filter in filters {
        let fieldset = match filter.filter_type {
            FilterType::CheckboxOr | FilterType::EnumAnd => {
                let Some(attribute) = attrs_by_id.get(filter.attribute_id.as_str()) else {
                    continue;
                };
                let options = state
                    .taxonomy
                    .list_attribute_options(&filter.attribute_id, Lang::Uk)
                    .await
                    .map_err(|e| WebError::Internal(e.to_string()))?;
                let param = format!("attr_{}", filter.attribute_id);
                let selected = values_for(raw, &param);
                checkbox_fieldset(&param, &attribute.name, &options, &selected)
            }
            FilterType::RangeGeneric => {
                let Some(attribute) = attrs_by_id.get(filter.attribute_id.as_str()) else {
                    continue;
                };
                let min_param = format!("attr_{}_min", filter.attribute_id);
                let max_param = format!("attr_{}_max", filter.attribute_id);
                range_fieldset(
                    &legend_with_unit(&attribute.name, attribute.unit.as_deref()),
                    &min_param,
                    &max_param,
                    raw,
                )
            }
            FilterType::RangePrice => range_fieldset("Ціна, ₴", "price_min", "price_max", raw),
            FilterType::String | FilterType::Number => continue,
        };
        fieldsets.push(fieldset);
    }

    if fieldsets.is_empty() {
        return Ok(None);
    }

    Ok(Some(html! {
        form role="search" method="get" action=(format!("/c/{slug}")) {
            @for fieldset in &fieldsets {
                (fieldset)
            }
            button type="submit" { "Застосувати" }
        }
    }))
}

/// `<legend>` для `range_generic`: название атрибута + (опц.) технический юнит в скобках.
fn legend_with_unit(name: &str, unit: Option<&str>) -> String {
    match unit {
        Some(unit) if !unit.is_empty() => format!("{name} ({unit})"),
        _ => name.to_string(),
    }
}

/// `<fieldset>` с чекбоксами для `checkbox_or`/`enum_and`.
fn checkbox_fieldset(
    param: &str,
    legend: &str,
    options: &[catalog::AttributeOption],
    selected: &[&str],
) -> Markup {
    html! {
        fieldset {
            legend { (legend) }
            @for option in options {
                label {
                    input
                        type="checkbox"
                        name=(param)
                        value=(option.value)
                        checked[selected.contains(&option.value.as_str())];
                    (option.value)
                }
            }
        }
    }
}

/// `<fieldset>` с парой числовых полей "от"/"до" для `range_generic`/`range_price`.
/// Значения полей сохраняются из `raw`, только если их можно распарсить как число
/// (нерозбірні значення не повертаються користувачу — graceful ignore).
fn range_fieldset(
    legend: &str,
    min_param: &str,
    max_param: &str,
    raw: &[(String, String)],
) -> Markup {
    let min_value = numeric_value_for(raw, min_param);
    let max_value = numeric_value_for(raw, max_param);
    html! {
        fieldset {
            legend { (legend) }
            label {
                "Від "
                input type="number" step="any" name=(min_param) value=[min_value];
            }
            label {
                "До "
                input type="number" step="any" name=(max_param) value=[max_value];
            }
        }
    }
}

/// Текущее значение query-параметра `key`, если оно распознаётся как число (для
/// предзаполнения `<input type="number">`). Возвращает исходную строку — без переформатирования
/// `f64`, чтобы не терять точность/вид введённого пользователем числа.
fn numeric_value_for<'a>(raw: &'a [(String, String)], key: &str) -> Option<&'a str> {
    raw.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
        .filter(|v| v.trim().parse::<f64>().is_ok())
}

// -----------------------------------------------------------------
// Ссылки пагинации с сохранением активных фильтров
// -----------------------------------------------------------------

/// Построить `href` для ссылки пагинации `/c/{slug}?...&page={page}`, сохраняя все активные
/// параметры фильтров из `raw` (кроме `page`). Значения процентно кодируются — фильтры могут
/// содержать кириллицу (например, `"Синій"`).
fn pagination_href(slug: &str, raw: &[(String, String)], page: u32) -> String {
    let mut qs = String::new();
    for (k, v) in raw {
        if k == "page" {
            continue;
        }
        if !qs.is_empty() {
            qs.push('&');
        }
        qs.push_str(&encode_qs(k));
        qs.push('=');
        qs.push_str(&encode_qs(v));
    }
    if !qs.is_empty() {
        qs.push('&');
    }
    qs.push_str("page=");
    qs.push_str(&page.to_string());
    format!("/c/{slug}?{qs}")
}

/// Множина символів для percent-encoding компонентів query-строки: як [`NON_ALPHANUMERIC`],
/// але без `_-.~` (RFC 3986 "unreserved") — щоб посилання пагінації лишались читабельними
/// (`attr_attr1_min=10`, а не `attr%5Fattr1%5Fmin=10`).
const QUERY_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'_')
    .remove(b'-')
    .remove(b'.')
    .remove(b'~');

/// Процентное кодирование одного компонента query-строки (ключа или значения).
fn encode_qs(s: &str) -> std::borrow::Cow<'_, str> {
    utf8_percent_encode(s, QUERY_ENCODE_SET).into()
}
