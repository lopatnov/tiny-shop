//! AccountRepo: registration, authentication, role management, seller profiles.
//!
//! ## Password hashing
//! Argon2id with m=19456 KiB, t=2, p=1 (security-engineer recommendation T1a-2).
//! CPU-intensive hash/verify are offloaded to `tokio::task::spawn_blocking`.
//!
//! ## Outbox
//! `register` emits `AccountRegistered` atomically with the INSERT.

use sqlx::Row;

use db::{ContextDb, outbox};
use shared::{NewEvent, now_ms};

use crate::account::{Account, AccountRole, NewAccount, NewSeller, Seller};

const AGGREGATE: &str = "account";

#[derive(Debug, thiserror::Error)]
pub enum AccountError {
    #[error("email already taken")]
    EmailTaken,
    #[error("db: {0}")]
    Db(#[from] sqlx::Error),
    #[error("password hashing: {0}")]
    HashError(String),
    #[error("task join: {0}")]
    Join(String),
}

impl From<db::DbError> for AccountError {
    fn from(e: db::DbError) -> Self {
        match e {
            db::DbError::Sqlx(e) => AccountError::Db(e),
            other => AccountError::Db(sqlx::Error::Protocol(other.to_string())),
        }
    }
}

#[derive(Clone)]
pub struct AccountRepo {
    db: ContextDb,
}

impl AccountRepo {
    pub fn new(db: ContextDb) -> Self {
        Self { db }
    }

    /// Register a new account with default 'customer' role. Emits `AccountRegistered` to outbox.
    pub async fn register(&self, new: &NewAccount) -> Result<(), AccountError> {
        let pass_hash = hash_password(new.password.clone()).await?;
        let ts = now_ms();
        let payload = serde_json::json!({ "account_id": new.id, "email": new.email });
        let mut tx = self.db.writer.begin().await?;
        let result = sqlx::query(
            "INSERT INTO accounts (id, email, pass_hash, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&new.id)
        .bind(&new.email)
        .bind(&pass_hash)
        .bind(ts)
        .bind(ts)
        .execute(&mut *tx)
        .await;
        if let Err(ref e) = result
            && is_unique_violation(e)
        {
            return Err(AccountError::EmailTaken);
        }
        result?;
        sqlx::query(
            "INSERT INTO account_roles (account_id, role, granted_at) VALUES (?, 'customer', ?)",
        )
        .bind(&new.id)
        .bind(ts)
        .execute(&mut *tx)
        .await?;
        outbox::enqueue(
            &mut *tx,
            &NewEvent {
                aggregate: AGGREGATE.to_string(),
                aggregate_id: new.id.clone(),
                event_type: "AccountRegistered".to_string(),
                payload,
            },
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Find account by email (includes roles). Returns `None` if not found.
    pub async fn find_by_email(&self, email: &str) -> Result<Option<Account>, AccountError> {
        let row = sqlx::query(
            "SELECT id, email, pass_hash, email_verified_at, created_at, updated_at \
             FROM accounts WHERE email = ?",
        )
        .bind(email)
        .fetch_optional(&self.db.reader)
        .await?;
        self.hydrate(row).await
    }

    /// Find account by id (includes roles). Returns `None` if not found.
    pub async fn find_by_id(&self, id: &str) -> Result<Option<Account>, AccountError> {
        let row = sqlx::query(
            "SELECT id, email, pass_hash, email_verified_at, created_at, updated_at \
             FROM accounts WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.db.reader)
        .await?;
        self.hydrate(row).await
    }

    /// Verify email + password. Returns `Some(account)` on success, `None` on bad credentials.
    ///
    /// Always performs an Argon2id verification, even when the email is unknown (using a
    /// fixed dummy hash), so response time does not leak whether an account exists
    /// (timing attack / user enumeration — Gemini review on PR #10).
    pub async fn authenticate(
        &self,
        email: &str,
        password: &str,
    ) -> Result<Option<Account>, AccountError> {
        let account = self.find_by_email(email).await?;
        let hash = account
            .as_ref()
            .map(|a| a.pass_hash.clone())
            .unwrap_or_else(|| DUMMY_HASH.to_string());
        let pw = password.to_owned();
        let ok = tokio::task::spawn_blocking(move || verify_password_sync(&hash, &pw))
            .await
            .map_err(|e| AccountError::Join(e.to_string()))?;
        Ok(account.filter(|_| ok))
    }

    /// Grant a role to an account (idempotent; no-op if already granted).
    pub async fn grant_role(
        &self,
        account_id: &str,
        role: AccountRole,
    ) -> Result<(), AccountError> {
        sqlx::query(
            "INSERT INTO account_roles (account_id, role, granted_at) VALUES (?, ?, ?) \
             ON CONFLICT(account_id, role) DO NOTHING",
        )
        .bind(account_id)
        .bind(role.as_str())
        .bind(now_ms())
        .execute(&self.db.writer)
        .await?;
        Ok(())
    }

    /// Revoke a role (hard delete).
    pub async fn revoke_role(
        &self,
        account_id: &str,
        role: AccountRole,
    ) -> Result<(), AccountError> {
        sqlx::query("DELETE FROM account_roles WHERE account_id = ? AND role = ?")
            .bind(account_id)
            .bind(role.as_str())
            .execute(&self.db.writer)
            .await?;
        Ok(())
    }

    /// Register a seller profile and automatically grant the 'seller' role.
    pub async fn register_seller(&self, new: &NewSeller) -> Result<(), AccountError> {
        let ts = now_ms();
        let mut tx = self.db.writer.begin().await?;
        sqlx::query("INSERT INTO sellers (account_id, display_name, created_at) VALUES (?, ?, ?)")
            .bind(&new.account_id)
            .bind(&new.display_name)
            .bind(ts)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO account_roles (account_id, role, granted_at) \
             VALUES (?, 'seller', ?) \
             ON CONFLICT(account_id, role) DO NOTHING",
        )
        .bind(&new.account_id)
        .bind(ts)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Get seller profile. Returns `None` if account is not registered as a seller.
    pub async fn get_seller(&self, account_id: &str) -> Result<Option<Seller>, AccountError> {
        let row = sqlx::query(
            "SELECT account_id, display_name, split_receiver_id, verified_at, created_at \
             FROM sellers WHERE account_id = ?",
        )
        .bind(account_id)
        .fetch_optional(&self.db.reader)
        .await?;
        Ok(row.map(|r| Seller {
            account_id: r.get("account_id"),
            display_name: r.get("display_name"),
            split_receiver_id: r.get("split_receiver_id"),
            verified_at: r.get("verified_at"),
            created_at: r.get("created_at"),
        }))
    }

    async fn hydrate(
        &self,
        row: Option<sqlx::sqlite::SqliteRow>,
    ) -> Result<Option<Account>, AccountError> {
        let Some(r) = row else { return Ok(None) };
        let id: String = r.get("id");
        let roles = self.fetch_roles(&id).await?;
        Ok(Some(Account {
            id,
            email: r.get("email"),
            pass_hash: r.get("pass_hash"),
            email_verified_at: r.get("email_verified_at"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
            roles,
        }))
    }

    async fn fetch_roles(&self, account_id: &str) -> Result<Vec<AccountRole>, AccountError> {
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT role FROM account_roles WHERE account_id = ?")
                .bind(account_id)
                .fetch_all(&self.db.reader)
                .await?;
        Ok(rows.iter().filter_map(|s| AccountRole::parse(s)).collect())
    }
}

// ---------------------------------------------------------------------------
// Crypto helpers (sync, called inside spawn_blocking)
// ---------------------------------------------------------------------------

fn hash_password_sync(password: &str) -> Result<String, String> {
    use argon2::password_hash::{SaltString, rand_core::OsRng};
    use argon2::{Algorithm, Argon2, Params, PasswordHasher, Version};
    let params = Params::new(19456, 2, 1, None).map_err(|e| e.to_string())?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let salt = SaltString::generate(&mut OsRng);
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

fn verify_password_sync(hash: &str, password: &str) -> bool {
    use argon2::password_hash::PasswordHash;
    use argon2::{Argon2, PasswordVerifier};
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

async fn hash_password(password: String) -> Result<String, AccountError> {
    tokio::task::spawn_blocking(move || hash_password_sync(&password))
        .await
        .map_err(|e| AccountError::Join(e.to_string()))?
        .map_err(AccountError::HashError)
}

/// PHC-format Argon2id hash of a fixed dummy password, with the same params as
/// [`hash_password_sync`]. Used by `authenticate` to equalize timing for unknown emails.
const DUMMY_HASH: &str =
    "$argon2id$v=19$m=19456,t=2,p=1$Y2hvb2NvbGF0ZQ$MTIzNDU2Nzg5MDEyMzQ1Njc4OTAxMjM0NTY3ODkwMTI";

/// `true` only for a UNIQUE violation on `accounts.email` (the only UNIQUE column besides
/// the primary key `id`, which has its own integrity error path).
fn is_unique_violation(e: &sqlx::Error) -> bool {
    if let sqlx::Error::Database(ref db_err) = *e {
        let msg = db_err.message();
        return msg.contains("UNIQUE constraint failed") && msg.contains("accounts.email");
    }
    false
}
