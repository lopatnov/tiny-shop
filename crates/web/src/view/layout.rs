//! Общий каркас страницы (semantic HTML5, design-1a.md §6).
//!
//! `<main>` — единственный landmark на страницу; заголовок `<h1>` формируется конкретной
//! страницей и передаётся как часть `main`.

use maud::{DOCTYPE, Markup, html};

/// Обернуть содержимое страницы в общий HTML-каркас.
///
/// `head_extra` — дополнительная разметка `<head>` (например, JSON-LD `<script>`-теги).
/// `main` — содержимое `<main>`, включая `<h1>` страницы.
pub fn page_shell(title: &str, head_extra: Markup, main: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="uk" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) }
                (head_extra)
            }
            body {
                main {
                    (main)
                }
            }
        }
    }
}
