//! `orders` — контекст заказов.
//!
//! Скелет в 1a (схема `orders`/`order_items` со снимком конфигурации, T1a-8); реальный
//! checkout и выдача — фаза 1b (см. `.claude/docs/design-1a.md` §3).

mod order;
mod repository;

pub use order::{NewOrder, NewOrderItem, Order, OrderItem, OrderStatus};
pub use repository::{OrderError, OrderRepo};
