//! `identity` — контекст Identity & Access: аккаунты, мульти-роль, сессии, продавцы (T1a-2).
//!
//! Схема: `accounts`, `account_roles`, `sessions`, `sellers` (migration `0002_identity.sql`).
//! Крипто: Argon2id (m=19456, t=2, p=1) для паролей; BLAKE3-хэш для хранения токенов сессий.

mod account;
mod account_repo;
mod session;
mod session_repo;

pub use account::{Account, AccountRole, NewAccount, NewSeller, Seller};
pub use account_repo::{AccountError, AccountRepo};
pub use session::{NewSession, Session, SessionToken};
pub use session_repo::{SessionError, SessionRepo};
