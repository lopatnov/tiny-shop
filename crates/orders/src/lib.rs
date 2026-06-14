//! `orders` — контекст заказов.
//!
//! Скелет в 1a (схема `orders`/`order_items` со снимком конфигурации, T1a-8); реальный
//! checkout и выдача — фаза 1b (см. `.claude/docs/design-1a.md` §3). Корзина (T1b-1) живёт
//! здесь же — отдельный файл `orders.db`, без транзакций/JOIN с `orders`/`order_items`.

mod cart;
mod cart_repo;
mod order;
mod repository;

pub use cart::{Cart, CartItem, CartToken, NewCartItem};
pub use cart_repo::{CartError, CartRepo, MAX_QTY};
pub use order::{
    NewOrder, NewOrderContact, NewOrderItem, Order, OrderContact, OrderItem, OrderStatus,
};
pub use repository::{OrderError, OrderRepo};
