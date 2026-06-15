//! Хлебные крошки (`<nav aria-label>`, design-1a.md §6).

use maud::{Markup, html};

/// Построить навигацию-крошки. Каждый элемент — `(name, url)`; элемент без `url`
/// считается текущей страницей (`aria-current="page"`, без ссылки).
pub fn breadcrumb_nav(items: &[(String, Option<String>)]) -> Markup {
    html! {
        nav aria-label="Хлібні крихти" {
            ol class="breadcrumb" {
                @for (name, url) in items {
                    @match url {
                        Some(href) => li class="breadcrumb-item" { a href=(href) { (name) } },
                        None => li class="breadcrumb-item active" aria-current="page" { (name) },
                    }
                }
            }
        }
    }
}
