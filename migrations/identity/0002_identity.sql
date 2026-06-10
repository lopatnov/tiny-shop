-- Identity context: accounts, multi-role, sessions, sellers (T1a-2).
-- Security decisions: Argon2id PHC hash in pass_hash; BLAKE3 of raw token in sessions.

CREATE TABLE accounts (
    id                TEXT    PRIMARY KEY,
    email             TEXT    NOT NULL UNIQUE,
    pass_hash         TEXT    NOT NULL,
    email_verified_at INTEGER,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);

CREATE TABLE account_roles (
    account_id TEXT    NOT NULL REFERENCES accounts(id),
    role       TEXT    NOT NULL CHECK (role IN ('customer', 'seller', 'admin')),
    granted_at INTEGER NOT NULL,
    PRIMARY KEY (account_id, role)
);

CREATE TABLE sessions (
    token_hash TEXT    PRIMARY KEY,
    account_id TEXT    NOT NULL REFERENCES accounts(id),
    expires_at INTEGER NOT NULL,
    user_agent TEXT,
    ip_addr    TEXT,
    created_at INTEGER NOT NULL
);
CREATE INDEX sessions_account ON sessions(account_id);

CREATE TABLE sellers (
    account_id        TEXT    PRIMARY KEY REFERENCES accounts(id),
    display_name      TEXT    NOT NULL,
    split_receiver_id TEXT,
    verified_at       INTEGER,
    created_at        INTEGER NOT NULL
);
