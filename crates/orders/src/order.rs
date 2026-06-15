//! Доменные типы контекста Orders (T1a-8).

use serde::{Deserialize, Serialize};

/// Статус заказа. Наполняется переходами в Phase 1b (checkout/payment/delivery).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    Created,
    Paid,
    Fulfilled,
    Cancelled,
}

impl OrderStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            OrderStatus::Created => "created",
            OrderStatus::Paid => "paid",
            OrderStatus::Fulfilled => "fulfilled",
            OrderStatus::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "created" => Some(OrderStatus::Created),
            "paid" => Some(OrderStatus::Paid),
            "fulfilled" => Some(OrderStatus::Fulfilled),
            "cancelled" => Some(OrderStatus::Cancelled),
            _ => None,
        }
    }
}

/// Заказ (агрегат). Создаётся в `created`, checkout наполняет через 1b.
#[derive(Debug, Clone)]
pub struct Order {
    pub id: String,
    pub buyer_id: String,
    pub status: OrderStatus,
    /// Итого в минорных единицах валюты (копейки для UAH).
    pub total_minor: i64,
    pub currency: String,
    /// Unix-миллисекунды создания.
    pub created_at: i64,
    pub items: Vec<OrderItem>,
}

/// Позиция заказа — неизменяемый снимок выбранного товара/варианта.
#[derive(Debug, Clone)]
pub struct OrderItem {
    pub id: String,
    pub order_id: String,
    pub product_id: String,
    pub seller_id: String,
    /// Вариант/опция; `None` если товар без вариантов.
    pub variant_id: Option<String>,
    /// Снимок названия на момент оформления.
    pub title: String,
    /// Снимок цены в минорных единицах.
    pub unit_price_minor: i64,
    pub currency: String,
    /// JSON-снимок выбранных опций конфигурации (может быть `null`).
    pub config_snapshot: Option<serde_json::Value>,
}

/// Входные данные для создания заказа.
#[derive(Debug)]
pub struct NewOrder {
    pub id: String,
    pub buyer_id: String,
    pub currency: String,
}

/// Входные данные для добавления позиции в заказ.
#[derive(Debug)]
pub struct NewOrderItem {
    pub id: String,
    pub order_id: String,
    pub product_id: String,
    pub seller_id: String,
    pub variant_id: Option<String>,
    pub title: String,
    pub unit_price_minor: i64,
    pub currency: String,
    pub config_snapshot: Option<serde_json::Value>,
}

/// Контактные данные гостя для выдачи/чека (T1b-2). Изолированы в `order_contact` — отдельно
/// от `orders`, чтобы PII (email) не раздувало основную таблицу.
///
/// Без `Debug` — содержит email (PII), чтобы случайный `{:?}`-лог не утёк в логи.
#[derive(Clone)]
pub struct OrderContact {
    pub order_id: String,
    pub email: String,
    pub name: Option<String>,
    /// Unix-миллисекунды создания.
    pub created_at: i64,
}

/// Входные данные для контакта гостя на checkout.
///
/// Без `Debug` — содержит email (PII), чтобы случайный `{:?}`-лог не утёк в логи.
#[derive(Clone)]
pub struct NewOrderContact {
    pub email: String,
    pub name: Option<String>,
}
