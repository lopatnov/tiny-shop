//! `payments` — порт платежей со сплитом (design-1a.md §4).
//!
//! Модель: площадка создаёт инвойс с разбиением суммы по получателям-продавцам
//! (monobank `splitReceiver` / LiqPay fallback — см. ADR в roadmap), удерживая комиссию.
//! В 1a здесь ТОЛЬКО контракт и типы — без реализации провайдера (она в 1b).

use std::future::Future;

#[derive(Debug, thiserror::Error)]
pub enum PaymentError {
    #[error("provider error: {0}")]
    Provider(String),
    #[error("invalid webhook: {0}")]
    Webhook(String),
}

/// Получатель части платежа (продавец) + его сумма в минорных единицах.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitReceiver {
    pub merchant_id: String,
    pub amount_minor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceRequest {
    pub order_id: String,
    pub amount_minor: i64,
    pub currency: String,
    /// Разбиение по продавцам; комиссия площадки — отдельной строкой/остатком.
    pub splits: Vec<SplitReceiver>,
    pub redirect_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invoice {
    pub invoice_id: String,
    pub pay_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaymentStatus {
    Pending,
    Paid,
    Failed,
    Refunded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefundRequest {
    pub invoice_id: String,
    pub amount_minor: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefundStatus {
    Pending,
    Done,
    Failed,
}

/// Событие из вебхука провайдера (после верификации подписи).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaymentEvent {
    pub invoice_id: String,
    pub status: PaymentStatus,
}

/// Порт платежей. Нативный async-fn-in-trait (без `async-trait`).
/// Реализация конкретного провайдера — 1b за этим трейтом.
pub trait Payments {
    fn create_invoice(
        &self,
        req: InvoiceRequest,
    ) -> impl Future<Output = Result<Invoice, PaymentError>> + Send;

    fn capture(
        &self,
        invoice_id: &str,
    ) -> impl Future<Output = Result<PaymentStatus, PaymentError>> + Send;

    fn refund(
        &self,
        req: RefundRequest,
    ) -> impl Future<Output = Result<RefundStatus, PaymentError>> + Send;

    /// Верификация и разбор вебхука провайдера (синхронно).
    /// `headers` — пары (имя, значение) как `&str`, чтобы не тянуть HTTP-крейт в порт и не
    /// заставлять вызывающую сторону аллоцировать `String` на каждый заголовок.
    fn parse_webhook(
        &self,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<PaymentEvent, PaymentError>;
}
