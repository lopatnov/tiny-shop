//! Integration tests T1a-2: account registration, authentication, role management,
//! seller profiles, session lifecycle, outbox emission.

use std::sync::atomic::{AtomicUsize, Ordering};

use db::{ContextDb, migrate_identity, open};
use identity::{AccountRepo, AccountRole, NewAccount, NewSeller, NewSession, SessionRepo};
use shared::now_ms;

struct TempDb {
    path: std::path::PathBuf,
    db: ContextDb,
}

impl Drop for TempDb {
    fn drop(&mut self) {
        for suffix in ["", "-wal", "-shm"] {
            let p = format!("{}{}", self.path.display(), suffix);
            let _ = std::fs::remove_file(p);
        }
    }
}

async fn temp_db(tag: &str) -> TempDb {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("tinyshop-identity-{nanos}-{n}.db"));
    let _ = std::fs::remove_file(&path);
    let db = open(tag, &path).await.expect("open");
    migrate_identity(&db.writer).await.expect("migrate");
    TempDb { path, db }
}

fn new_account(id: &str, email: &str) -> NewAccount {
    NewAccount {
        id: id.to_string(),
        email: email.to_string(),
        password: "hunter2!Secret".to_string(),
    }
}

// -----------------------------------------------------------------
// Account tests
// -----------------------------------------------------------------

#[tokio::test]
async fn register_and_find_by_email() {
    let t = temp_db("reg-find-email").await;
    let repo = AccountRepo::new(t.db.clone());

    repo.register(&new_account("acc-1", "alice@example.com"))
        .await
        .expect("register");

    let acc = repo
        .find_by_email("alice@example.com")
        .await
        .expect("find")
        .expect("present");
    assert_eq!(acc.id, "acc-1");
    assert_eq!(acc.email, "alice@example.com");
    assert!(acc.email_verified_at.is_none());
    assert!(acc.roles.contains(&AccountRole::Customer));
}

#[tokio::test]
async fn register_and_find_by_id() {
    let t = temp_db("reg-find-id").await;
    let repo = AccountRepo::new(t.db.clone());

    repo.register(&new_account("acc-2", "bob@example.com"))
        .await
        .expect("register");

    let acc = repo
        .find_by_id("acc-2")
        .await
        .expect("find")
        .expect("present");
    assert_eq!(acc.email, "bob@example.com");
    assert_eq!(acc.roles.len(), 1);
    assert_eq!(acc.roles[0], AccountRole::Customer);
}

#[tokio::test]
async fn register_duplicate_email_is_rejected() {
    let t = temp_db("dup-email").await;
    let repo = AccountRepo::new(t.db.clone());

    repo.register(&new_account("acc-a", "dup@example.com"))
        .await
        .expect("first ok");
    let err = repo
        .register(&new_account("acc-b", "dup@example.com"))
        .await
        .expect_err("should fail");
    assert!(
        matches!(err, identity::AccountError::EmailTaken),
        "expected EmailTaken, got {err}"
    );
}

#[tokio::test]
async fn authenticate_correct_password() {
    let t = temp_db("auth-ok").await;
    let repo = AccountRepo::new(t.db.clone());
    repo.register(&new_account("acc-3", "carol@example.com"))
        .await
        .expect("register");

    let result = repo
        .authenticate("carol@example.com", "hunter2!Secret")
        .await
        .expect("no db error");
    assert!(result.is_some(), "expected Some(account)");
    assert_eq!(result.unwrap().email, "carol@example.com");
}

#[tokio::test]
async fn authenticate_wrong_password_returns_none() {
    let t = temp_db("auth-bad-pw").await;
    let repo = AccountRepo::new(t.db.clone());
    repo.register(&new_account("acc-4", "dave@example.com"))
        .await
        .expect("register");

    let result = repo
        .authenticate("dave@example.com", "wrongpassword")
        .await
        .expect("no db error");
    assert!(result.is_none());
}

#[tokio::test]
async fn grant_and_revoke_role() {
    let t = temp_db("roles").await;
    let repo = AccountRepo::new(t.db.clone());
    repo.register(&new_account("acc-5", "eve@example.com"))
        .await
        .expect("register");

    repo.grant_role("acc-5", AccountRole::Admin)
        .await
        .expect("grant admin");
    let acc = repo.find_by_id("acc-5").await.expect("find").unwrap();
    assert!(acc.roles.contains(&AccountRole::Admin));
    assert!(acc.roles.contains(&AccountRole::Customer));

    repo.revoke_role("acc-5", AccountRole::Admin)
        .await
        .expect("revoke admin");
    let acc = repo.find_by_id("acc-5").await.expect("find").unwrap();
    assert!(!acc.roles.contains(&AccountRole::Admin));
    assert!(acc.roles.contains(&AccountRole::Customer));
}

#[tokio::test]
async fn register_seller_grants_seller_role() {
    let t = temp_db("seller").await;
    let repo = AccountRepo::new(t.db.clone());
    repo.register(&new_account("acc-6", "frank@example.com"))
        .await
        .expect("register");

    repo.register_seller(&NewSeller {
        account_id: "acc-6".to_string(),
        display_name: "Frank's Shop".to_string(),
    })
    .await
    .expect("register seller");

    let seller = repo
        .get_seller("acc-6")
        .await
        .expect("get")
        .expect("present");
    assert_eq!(seller.display_name, "Frank's Shop");
    assert!(seller.split_receiver_id.is_none());
    assert!(seller.verified_at.is_none());

    let acc = repo.find_by_id("acc-6").await.expect("find").unwrap();
    assert!(acc.roles.contains(&AccountRole::Seller));
    assert!(acc.roles.contains(&AccountRole::Customer));
}

// -----------------------------------------------------------------
// Session tests
// -----------------------------------------------------------------

async fn register_acc(repo: &AccountRepo, id: &str, email: &str) {
    repo.register(&new_account(id, email))
        .await
        .expect("register");
}

#[tokio::test]
async fn create_and_verify_session() {
    let t = temp_db("sess-verify").await;
    let acc_repo = AccountRepo::new(t.db.clone());
    let sess_repo = SessionRepo::new(t.db.clone());

    register_acc(&acc_repo, "acc-s1", "sess1@example.com").await;

    let token = sess_repo
        .create(&NewSession {
            account_id: "acc-s1".to_string(),
            expires_at: now_ms() + 3_600_000,
            user_agent: Some("TestAgent/1.0".to_string()),
            ip_addr: None,
        })
        .await
        .expect("create");

    let sess = sess_repo
        .verify(token.as_str())
        .await
        .expect("no db error")
        .expect("session found");
    assert_eq!(sess.account_id, "acc-s1");
    assert_eq!(sess.user_agent.as_deref(), Some("TestAgent/1.0"));
}

#[tokio::test]
async fn create_with_past_expiry_is_rejected() {
    let t = temp_db("sess-expired").await;
    let acc_repo = AccountRepo::new(t.db.clone());
    let sess_repo = SessionRepo::new(t.db.clone());

    register_acc(&acc_repo, "acc-s2", "sess2@example.com").await;

    let err = sess_repo
        .create(&NewSession {
            account_id: "acc-s2".to_string(),
            expires_at: now_ms() - 1000,
            user_agent: None,
            ip_addr: None,
        })
        .await
        .expect_err("should reject already-expired session");
    assert!(matches!(err, identity::SessionError::InvalidExpiry));
}

#[tokio::test]
async fn expired_session_returns_none() {
    let t = temp_db("sess-expired-verify").await;
    let acc_repo = AccountRepo::new(t.db.clone());
    let sess_repo = SessionRepo::new(t.db.clone());

    register_acc(&acc_repo, "acc-s2b", "sess2b@example.com").await;

    let token = sess_repo
        .create(&NewSession {
            account_id: "acc-s2b".to_string(),
            expires_at: now_ms() + 3_600_000,
            user_agent: None,
            ip_addr: None,
        })
        .await
        .expect("create");

    // Backdate the session's expiry directly, simulating time passing.
    sqlx::query("UPDATE sessions SET expires_at = ? WHERE token_hash = ?")
        .bind(now_ms() - 1000)
        .bind(blake3::hash(token.as_str().as_bytes()).to_string())
        .execute(&t.db.writer)
        .await
        .expect("backdate expiry");

    let result = sess_repo.verify(token.as_str()).await.expect("no db error");
    assert!(result.is_none(), "expired session should not verify");
}

#[tokio::test]
async fn revoked_session_returns_none() {
    let t = temp_db("sess-revoke").await;
    let acc_repo = AccountRepo::new(t.db.clone());
    let sess_repo = SessionRepo::new(t.db.clone());

    register_acc(&acc_repo, "acc-s3", "sess3@example.com").await;

    let token = sess_repo
        .create(&NewSession {
            account_id: "acc-s3".to_string(),
            expires_at: now_ms() + 3_600_000,
            user_agent: None,
            ip_addr: None,
        })
        .await
        .expect("create");

    sess_repo.revoke(token.as_str()).await.expect("revoke");

    let result = sess_repo.verify(token.as_str()).await.expect("no db error");
    assert!(result.is_none(), "revoked session should not verify");
}

// -----------------------------------------------------------------
// Outbox
// -----------------------------------------------------------------

#[tokio::test]
async fn account_registered_event_emitted() {
    let t = temp_db("outbox").await;
    let repo = AccountRepo::new(t.db.clone());

    repo.register(&new_account("acc-ev", "events@example.com"))
        .await
        .expect("register");

    let events = db::outbox::fetch_unpublished(&t.db.reader, 10)
        .await
        .expect("fetch");
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, "AccountRegistered");
    assert_eq!(events[0].aggregate_id, "acc-ev");
    assert_eq!(events[0].payload["email"], "events@example.com");
}
