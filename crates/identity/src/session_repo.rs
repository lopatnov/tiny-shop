//! SessionRepo: opaque session token lifecycle.
//!
//! Raw token (48 alphanumeric chars) is returned to the caller once; only its
//! BLAKE3 hash is stored. Replay attack if DB file leaks: attacker gets hashes,
//! not usable tokens (security-engineer recommendation T1a-2).

use sqlx::Row;

use db::ContextDb;
use shared::now_ms;

use crate::session::{NewSession, Session, SessionToken};

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("session expiry must be in the future")]
    InvalidExpiry,
    #[error("db: {0}")]
    Db(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct SessionRepo {
    db: ContextDb,
}

impl SessionRepo {
    pub fn new(db: ContextDb) -> Self {
        Self { db }
    }

    /// Generate a random token, store its BLAKE3 hash, return the raw token.
    ///
    /// Rejects `new.expires_at` that is already in the past — an immediately-dead
    /// session would just be junk in the table (CodeRabbit review on PR #10).
    pub async fn create(&self, new: &NewSession) -> Result<SessionToken, SessionError> {
        let ts = now_ms();
        if new.expires_at <= ts {
            return Err(SessionError::InvalidExpiry);
        }
        let raw = generate_token();
        let token_hash = hash_token(&raw);
        sqlx::query(
            "INSERT INTO sessions \
             (token_hash, account_id, expires_at, user_agent, ip_addr, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&token_hash)
        .bind(&new.account_id)
        .bind(new.expires_at)
        .bind(&new.user_agent)
        .bind(&new.ip_addr)
        .bind(ts)
        .execute(&self.db.writer)
        .await?;
        Ok(SessionToken(raw))
    }

    /// Verify token and return the session if it exists and has not expired.
    pub async fn verify(&self, raw_token: &str) -> Result<Option<Session>, SessionError> {
        let token_hash = hash_token(raw_token);
        let row = sqlx::query(
            "SELECT token_hash, account_id, expires_at, user_agent, ip_addr, created_at \
             FROM sessions WHERE token_hash = ? AND expires_at > ?",
        )
        .bind(&token_hash)
        .bind(now_ms())
        .fetch_optional(&self.db.reader)
        .await?;
        Ok(row.map(|r| Session {
            token_hash: r.get("token_hash"),
            account_id: r.get("account_id"),
            expires_at: r.get("expires_at"),
            user_agent: r.get("user_agent"),
            ip_addr: r.get("ip_addr"),
            created_at: r.get("created_at"),
        }))
    }

    /// Revoke a single session by raw token.
    pub async fn revoke(&self, raw_token: &str) -> Result<(), SessionError> {
        let token_hash = hash_token(raw_token);
        sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
            .bind(&token_hash)
            .execute(&self.db.writer)
            .await?;
        Ok(())
    }

    /// Revoke all sessions for an account (e.g. on password change or logout-all).
    pub async fn revoke_all_for_account(&self, account_id: &str) -> Result<(), SessionError> {
        sqlx::query("DELETE FROM sessions WHERE account_id = ?")
            .bind(account_id)
            .execute(&self.db.writer)
            .await?;
        Ok(())
    }
}

fn generate_token() -> String {
    use rand::Rng;
    use rand::distributions::Alphanumeric;
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(48)
        .map(char::from)
        .collect()
}

fn hash_token(raw: &str) -> String {
    format!("{}", blake3::hash(raw.as_bytes()))
}
