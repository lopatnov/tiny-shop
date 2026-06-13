//! Общий каркас страницы (semantic HTML5, design-1a.md §6).
//!
//! Landmarks: общий `<header>` (логотип/ссылка на главную) и `<main>` с содержимым конкретной
//! страницы; заголовок `<h1>` формируется страницей и передаётся как часть `main`.

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
                link rel="icon" type="image/x-icon" href="/assets/brand/favicon.ico";
                link rel="icon" type="image/png" sizes="32x32" href="/assets/brand/favicon-32.png";
                link rel="icon" type="image/png" sizes="16x16" href="/assets/brand/favicon-16.png";
                link rel="apple-touch-icon" sizes="180x180" href="/assets/brand/apple-touch-icon.png";
                (head_extra)
            }
            body {
                header {
                    a href="/" {
                        img src="/assets/brand/Vuriy_logo.webp" alt="Vuriy";
                    }
                }
                main {
                    (main)
                }
            }
        }
    }
}
