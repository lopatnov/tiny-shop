/// Account role (multi-role: one account may hold several).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountRole {
    Customer,
    Seller,
    Admin,
}

impl AccountRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            AccountRole::Customer => "customer",
            AccountRole::Seller => "seller",
            AccountRole::Admin => "admin",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "customer" => Some(AccountRole::Customer),
            "seller" => Some(AccountRole::Seller),
            "admin" => Some(AccountRole::Admin),
            _ => None,
        }
    }
}

#[derive(Clone)]
pub struct Account {
    pub id: String,
    pub email: String,
    pub pass_hash: String,
    pub email_verified_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub roles: Vec<AccountRole>,
}

/// `Debug` redacts `pass_hash` so it never ends up in logs/panics.
impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account")
            .field("id", &self.id)
            .field("email", &self.email)
            .field("pass_hash", &"<redacted>")
            .field("email_verified_at", &self.email_verified_at)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("roles", &self.roles)
            .finish()
    }
}

/// Input for registering a new account. Password is plaintext — repo hashes it.
#[derive(Clone)]
pub struct NewAccount {
    pub id: String,
    pub email: String,
    pub password: String,
}

/// `Debug` redacts `password` so it never ends up in logs/panics.
impl std::fmt::Debug for NewAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewAccount")
            .field("id", &self.id)
            .field("email", &self.email)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Seller profile linked to an account.
#[derive(Debug, Clone)]
pub struct Seller {
    pub account_id: String,
    pub display_name: String,
    pub split_receiver_id: Option<String>,
    pub verified_at: Option<i64>,
    pub created_at: i64,
}

/// Input for registering a seller profile (account must already exist).
#[derive(Debug, Clone)]
pub struct NewSeller {
    pub account_id: String,
    pub display_name: String,
}
