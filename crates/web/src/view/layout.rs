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
                link rel="stylesheet" href="/assets/vendor/bootstrap/bootstrap.min.css";
                link rel="stylesheet" href="/assets/css/vuriy.css";
                (head_extra)
            }
            body {
                header style="background-color: var(--vuriy-surface); border-bottom: 1px solid var(--vuriy-border);" {
                    div class="container d-flex align-items-center justify-content-between py-2 py-md-3" {
                        a href="/" class="d-inline-flex align-items-center text-decoration-none" {
                            img src="/assets/brand/Vuriy_logo.webp" alt="Vuriy" class="brand-logo";
                        }
                    }
                }
                main class="container py-4" {
                    (main)
                }
                footer class="border-top mt-5 py-3 text-center text-muted small" {
                    "© 2026 Vuriy"
                }
            }
        }
    }
}
