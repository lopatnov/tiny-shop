//! Обобщённый порт хранилища (design-1a.md §4).
//!
//! Per-context: каждый контекст реализует `Repository` для своих агрегатов через sqlx-адаптер
//! на свой файл БД. Здесь — только контракт; реализации — в задачах контекстов (T1a-2/3/4…).
//!
//! Outbox реализован функциями в [`crate::outbox`] (а не трейтом `OutboxStore`) — осознанно,
//! ради простоты: enqueue принимает любой executor и ложится в доменную транзакцию.

use std::future::Future;

use shared::{Page, Pagination};

/// Параметры выборки списка. Конкретные фильтры/сортировки добавляет контекст по мере надобности.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListQuery {
    pub pagination: Pagination,
}

/// Порт хранилища агрегата `T` с идентификатором `Id`. Нативный async-fn-in-trait.
pub trait Repository<T, Id> {
    type Error;

    fn get(&self, id: &Id) -> impl Future<Output = Result<Option<T>, Self::Error>> + Send;
    fn list(&self, q: ListQuery) -> impl Future<Output = Result<Page<T>, Self::Error>> + Send;
    fn save(&self, entity: &T) -> impl Future<Output = Result<(), Self::Error>> + Send;
    fn delete(&self, id: &Id) -> impl Future<Output = Result<(), Self::Error>> + Send;
}
