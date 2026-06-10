//! Хлебные крошки (`<nav aria-label>`, design-1a.md §6).

use maud::{Markup, html};

/// Построить навигацию-крошки. Каждый элемент — `(name, url)`; элемент без `url`
/// считается текущей страницей (`aria-current="page"`, без ссылки).
pub fn breadcrumb_nav(items: &[(String, Option<String>)]) -> Markup {
    html! {
        nav aria-label="Хлібні крихти" {
            ol {
                @for (name, url) in items {
                    @match url {
                        Some(href) => li { a href=(href) { (name) } },
                        None => li aria-current="page" { (name) },
                    }
                }
            }
        }
    }
}
