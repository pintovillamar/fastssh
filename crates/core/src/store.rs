//! SQLite storage: users, sessions, saved connections and trusted host keys.
//!
//! Secrets are stored only as blobs encrypted by the owner's vault key (see
//! [`crate::vault`]); this module never sees them in the clear.

use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("not found")]
    NotFound,
    #[error("{0}")]
    Invalid(&'static str),
    #[error("an account with this email already exists")]
    EmailTaken,
    #[error("sign-ups are closed on this server")]
    SignupClosed,
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}

type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, Clone)]
pub struct User {
    pub id: i64,
    pub email: String,
    /// The first account created on a server is its admin.
    pub is_admin: bool,
    pub vault: Option<VaultRecord>,
}

/// What the database keeps about a user's vault passphrase.
#[derive(Debug, Clone)]
pub struct VaultRecord {
    pub salt: Vec<u8>,
    /// Present when the passphrase is also the login password. Accounts that
    /// only sign in with Google have a vault passphrase but no verifier.
    pub verifier: Option<Vec<u8>>,
    pub wrapped_key: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    Password,
    Key,
}

/// The editable, non-secret part of a saved connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionDetails {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthKind,
}

#[derive(Debug, Clone)]
pub struct SavedConnection {
    pub id: i64,
    pub details: ConnectionDetails,
    /// Encrypted [`crate::Secrets`], if any were saved.
    pub secrets: Option<Vec<u8>>,
}

impl ConnectionDetails {
    fn normalized(mut self) -> Result<Self> {
        self.name = self.name.trim().to_owned();
        self.host = self.host.trim().to_owned();
        self.username = self.username.trim().to_owned();
        if self.host.is_empty() {
            return Err(StoreError::Invalid("host is required"));
        }
        if self.username.is_empty() {
            return Err(StoreError::Invalid("username is required"));
        }
        if self.port == 0 {
            return Err(StoreError::Invalid("port must be between 1 and 65535"));
        }
        if self.name.is_empty() {
            self.name = format!("{}@{}", self.username, self.host);
        }
        Ok(self)
    }
}

pub fn normalize_email(email: &str) -> Result<String> {
    let email = email.trim().to_lowercase();
    match email.split_once('@') {
        Some((local, domain)) if !local.is_empty() && domain.contains('.') => Ok(email),
        _ => Err(StoreError::Invalid("enter a valid email address")),
    }
}

/// Cheap to clone; all clones share one database connection.
///
/// Queries here are tiny and local, so they run directly on async threads
/// rather than through `spawn_blocking`.
#[derive(Clone)]
pub struct Store {
    db: Arc<Mutex<Connection>>,
}

const CONNECTION_COLUMNS: &str = "id, name, host, port, username, auth_kind, secrets";

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(db: Connection) -> Result<Self> {
        db.pragma_update(None, "journal_mode", "WAL")?;
        db.pragma_update(None, "foreign_keys", true)?;
        let version: i64 = db.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version < 1 {
            db.execute_batch(
                "BEGIN;
                 CREATE TABLE connections (
                     id        INTEGER PRIMARY KEY,
                     name      TEXT NOT NULL,
                     host      TEXT NOT NULL,
                     port      INTEGER NOT NULL,
                     username  TEXT NOT NULL,
                     auth_kind TEXT NOT NULL,
                     key_path  TEXT
                 );
                 CREATE TABLE known_hosts (
                     host       TEXT NOT NULL,
                     port       INTEGER NOT NULL,
                     public_key TEXT NOT NULL,
                     PRIMARY KEY (host, port)
                 );
                 PRAGMA user_version = 1;
                 COMMIT;",
            )?;
        }
        if version < 2 {
            // Accounts. Connections saved before accounts existed have no
            // owner yet; the first account created claims them. Trusted host
            // keys become per-user and are asked for again.
            db.execute_batch(
                "BEGIN;
                 CREATE TABLE users (
                     id                INTEGER PRIMARY KEY,
                     email             TEXT NOT NULL UNIQUE,
                     is_admin          INTEGER NOT NULL,
                     kdf_salt          BLOB,
                     password_verifier BLOB,
                     wrapped_key       BLOB,
                     created_at        INTEGER NOT NULL
                 );
                 CREATE TABLE sessions (
                     token_hash BLOB PRIMARY KEY,
                     user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                     expires_at INTEGER NOT NULL
                 );
                 ALTER TABLE connections ADD COLUMN user_id INTEGER REFERENCES users(id) ON DELETE CASCADE;
                 ALTER TABLE connections ADD COLUMN secrets BLOB;
                 DROP TABLE known_hosts;
                 CREATE TABLE known_hosts (
                     user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
                     host       TEXT NOT NULL,
                     port       INTEGER NOT NULL,
                     public_key TEXT NOT NULL,
                     PRIMARY KEY (user_id, host, port)
                 );
                 PRAGMA user_version = 2;
                 COMMIT;",
            )?;
        }
        Ok(Self {
            db: Arc::new(Mutex::new(db)),
        })
    }

    fn db(&self) -> MutexGuard<'_, Connection> {
        // A poisoned lock only means another thread panicked mid-query; the
        // connection itself is still usable.
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    // --- users ---

    pub fn has_users(&self) -> Result<bool> {
        Ok(self
            .db()
            .query_row("SELECT EXISTS (SELECT 1 FROM users)", [], |row| row.get(0))?)
    }

    /// Creates an account. The first one is always allowed and becomes the
    /// admin; later ones need `signup_open`.
    pub fn create_user(
        &self,
        email: &str,
        vault: Option<VaultRecord>,
        signup_open: bool,
        now: i64,
    ) -> Result<User> {
        let email = normalize_email(email)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let first: bool = tx.query_row("SELECT NOT EXISTS (SELECT 1 FROM users)", [], |r| r.get(0))?;
        if !first && !signup_open {
            return Err(StoreError::SignupClosed);
        }
        let taken: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM users WHERE email = ?1)",
            [&email],
            |r| r.get(0),
        )?;
        if taken {
            return Err(StoreError::EmailTaken);
        }
        let (salt, verifier, wrapped) = match &vault {
            Some(v) => (Some(&v.salt), v.verifier.as_ref(), Some(&v.wrapped_key)),
            None => (None, None, None),
        };
        tx.execute(
            "INSERT INTO users (email, is_admin, kdf_salt, password_verifier, wrapped_key, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![email, first, salt, verifier, wrapped, now],
        )?;
        let id = tx.last_insert_rowid();
        if first {
            tx.execute("UPDATE connections SET user_id = ?1 WHERE user_id IS NULL", [id])?;
        }
        tx.commit()?;
        Ok(User {
            id,
            email,
            is_admin: first,
            vault,
        })
    }

    pub fn user_by_email(&self, email: &str) -> Result<Option<User>> {
        let Ok(email) = normalize_email(email) else {
            return Ok(None);
        };
        Ok(self
            .db()
            .query_row(
                "SELECT id, email, is_admin, kdf_salt, password_verifier, wrapped_key
                 FROM users WHERE email = ?1",
                [email],
                read_user,
            )
            .optional()?)
    }

    /// Sets the vault of an account that does not have one yet.
    pub fn create_vault(&self, user_id: i64, vault: &VaultRecord) -> Result<()> {
        let changed = self.db().execute(
            "UPDATE users SET kdf_salt = ?1, password_verifier = ?2, wrapped_key = ?3
             WHERE id = ?4 AND wrapped_key IS NULL",
            params![vault.salt, vault.verifier, vault.wrapped_key, user_id],
        )?;
        match changed {
            0 => Err(StoreError::Invalid("this account already has a vault")),
            _ => Ok(()),
        }
    }

    // --- sessions ---

    pub fn create_session(&self, token_hash: &[u8; 32], user_id: i64, now: i64, expires_at: i64) -> Result<()> {
        let db = self.db();
        db.execute("DELETE FROM sessions WHERE expires_at <= ?1", [now])?;
        db.execute(
            "INSERT INTO sessions (token_hash, user_id, expires_at) VALUES (?1, ?2, ?3)",
            params![token_hash, user_id, expires_at],
        )?;
        Ok(())
    }

    pub fn session_user(&self, token_hash: &[u8; 32], now: i64) -> Result<Option<User>> {
        Ok(self
            .db()
            .query_row(
                "SELECT u.id, u.email, u.is_admin, u.kdf_salt, u.password_verifier, u.wrapped_key
                 FROM sessions s JOIN users u ON u.id = s.user_id
                 WHERE s.token_hash = ?1 AND s.expires_at > ?2",
                params![token_hash, now],
                read_user,
            )
            .optional()?)
    }

    pub fn delete_session(&self, token_hash: &[u8; 32]) -> Result<()> {
        self.db()
            .execute("DELETE FROM sessions WHERE token_hash = ?1", [token_hash])?;
        Ok(())
    }

    // --- connections (always scoped to their owner) ---

    pub fn connections(&self, user_id: i64) -> Result<Vec<SavedConnection>> {
        let db = self.db();
        let mut stmt = db.prepare(&format!(
            "SELECT {CONNECTION_COLUMNS} FROM connections
             WHERE user_id = ?1 ORDER BY name COLLATE NOCASE"
        ))?;
        let rows = stmt.query_map([user_id], read_connection)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn connection(&self, user_id: i64, id: i64) -> Result<SavedConnection> {
        self.db()
            .query_row(
                &format!("SELECT {CONNECTION_COLUMNS} FROM connections WHERE id = ?1 AND user_id = ?2"),
                [id, user_id],
                read_connection,
            )
            .optional()?
            .ok_or(StoreError::NotFound)
    }

    pub fn add_connection(
        &self,
        user_id: i64,
        details: ConnectionDetails,
        secrets: Option<Vec<u8>>,
    ) -> Result<SavedConnection> {
        let details = details.normalized()?;
        let db = self.db();
        db.execute(
            "INSERT INTO connections (user_id, name, host, port, username, auth_kind, secrets)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                user_id,
                details.name,
                details.host,
                details.port,
                details.username,
                auth_column(details.auth),
                secrets
            ],
        )?;
        Ok(SavedConnection {
            id: db.last_insert_rowid(),
            details,
            secrets,
        })
    }

    pub fn update_connection(
        &self,
        user_id: i64,
        id: i64,
        details: ConnectionDetails,
        secrets: Option<Vec<u8>>,
    ) -> Result<SavedConnection> {
        let details = details.normalized()?;
        let changed = self.db().execute(
            "UPDATE connections
             SET name = ?1, host = ?2, port = ?3, username = ?4, auth_kind = ?5, secrets = ?6
             WHERE id = ?7 AND user_id = ?8",
            params![
                details.name,
                details.host,
                details.port,
                details.username,
                auth_column(details.auth),
                secrets,
                id,
                user_id
            ],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(SavedConnection { id, details, secrets })
    }

    pub fn delete_connection(&self, user_id: i64, id: i64) -> Result<()> {
        let changed = self.db().execute(
            "DELETE FROM connections WHERE id = ?1 AND user_id = ?2",
            [id, user_id],
        )?;
        match changed {
            0 => Err(StoreError::NotFound),
            _ => Ok(()),
        }
    }

    // --- trusted host keys (per user, so nobody can pre-trust a key for others) ---

    /// The host key this user trusted earlier for a server, in OpenSSH format.
    pub fn known_host(&self, user_id: i64, host: &str, port: u16) -> Result<Option<String>> {
        Ok(self
            .db()
            .query_row(
                "SELECT public_key FROM known_hosts WHERE user_id = ?1 AND host = ?2 AND port = ?3",
                params![user_id, host, port],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn trust_host(&self, user_id: i64, host: &str, port: u16, public_key: &str) -> Result<()> {
        self.db().execute(
            "INSERT INTO known_hosts (user_id, host, port, public_key) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (user_id, host, port) DO UPDATE SET public_key = excluded.public_key",
            params![user_id, host, port, public_key],
        )?;
        Ok(())
    }

    pub fn forget_host(&self, user_id: i64, host: &str, port: u16) -> Result<()> {
        self.db().execute(
            "DELETE FROM known_hosts WHERE user_id = ?1 AND host = ?2 AND port = ?3",
            params![user_id, host, port],
        )?;
        Ok(())
    }
}

fn auth_column(auth: AuthKind) -> &'static str {
    match auth {
        AuthKind::Password => "password",
        AuthKind::Key => "key",
    }
}

fn read_user(row: &rusqlite::Row<'_>) -> rusqlite::Result<User> {
    let salt: Option<Vec<u8>> = row.get(3)?;
    let wrapped_key: Option<Vec<u8>> = row.get(5)?;
    Ok(User {
        id: row.get(0)?,
        email: row.get(1)?,
        is_admin: row.get(2)?,
        vault: match (salt, wrapped_key) {
            (Some(salt), Some(wrapped_key)) => Some(VaultRecord {
                salt,
                verifier: row.get(4)?,
                wrapped_key,
            }),
            _ => None,
        },
    })
}

fn read_connection(row: &rusqlite::Row<'_>) -> rusqlite::Result<SavedConnection> {
    let kind: String = row.get(5)?;
    Ok(SavedConnection {
        id: row.get(0)?,
        details: ConnectionDetails {
            name: row.get(1)?,
            host: row.get(2)?,
            port: row.get(3)?,
            username: row.get(4)?,
            auth: match kind.as_str() {
                "password" => AuthKind::Password,
                // Includes "key_file" rows from before the vault existed.
                _ => AuthKind::Key,
            },
        },
        secrets: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn details(name: &str) -> ConnectionDetails {
        ConnectionDetails {
            name: name.into(),
            host: " example.com ".into(),
            port: 22,
            username: "root".into(),
            auth: AuthKind::Key,
        }
    }

    fn store_with_user() -> (Store, User) {
        let store = Store::in_memory().unwrap();
        let user = store.create_user("Admin@Example.com", None, false, 0).unwrap();
        (store, user)
    }

    #[test]
    fn first_user_is_admin_and_later_ones_need_open_signup() {
        let (store, admin) = store_with_user();
        assert!(admin.is_admin);
        assert_eq!(admin.email, "admin@example.com");

        assert!(matches!(
            store.create_user("b@example.com", None, false, 0),
            Err(StoreError::SignupClosed)
        ));
        let second = store.create_user("b@example.com", None, true, 0).unwrap();
        assert!(!second.is_admin);
        assert!(matches!(
            store.create_user(" B@example.com", None, true, 0),
            Err(StoreError::EmailTaken)
        ));
        assert!(matches!(
            store.create_user("not-an-email", None, true, 0),
            Err(StoreError::Invalid(_))
        ));
    }

    #[test]
    fn vault_can_be_created_once() {
        let (store, user) = store_with_user();
        assert!(user.vault.is_none());
        let vault = VaultRecord {
            salt: vec![1; 16],
            verifier: None,
            wrapped_key: vec![2; 72],
        };
        store.create_vault(user.id, &vault).unwrap();
        let loaded = store.user_by_email("admin@example.com").unwrap().unwrap();
        assert_eq!(loaded.vault.unwrap().wrapped_key, vec![2; 72]);
        assert!(store.create_vault(user.id, &vault).is_err());
    }

    #[test]
    fn sessions_expire_and_can_be_deleted() {
        let (store, user) = store_with_user();
        let token = [7u8; 32];
        store.create_session(&token, user.id, 100, 200).unwrap();
        assert_eq!(store.session_user(&token, 150).unwrap().unwrap().id, user.id);
        assert!(store.session_user(&token, 200).unwrap().is_none());
        assert!(store.session_user(&[8u8; 32], 150).unwrap().is_none());
        store.delete_session(&token).unwrap();
        assert!(store.session_user(&token, 150).unwrap().is_none());
    }

    #[test]
    fn connections_round_trip() {
        let (store, user) = store_with_user();
        let saved = store.add_connection(user.id, details(""), Some(vec![9, 9])).unwrap();
        assert_eq!(saved.details.name, "root@example.com");
        assert_eq!(saved.details.host, "example.com");

        let mut edited = details("prod");
        edited.auth = AuthKind::Password;
        store.update_connection(user.id, saved.id, edited, None).unwrap();
        let loaded = store.connection(user.id, saved.id).unwrap();
        assert_eq!(loaded.details.name, "prod");
        assert_eq!(loaded.details.auth, AuthKind::Password);
        assert_eq!(loaded.secrets, None);

        store.delete_connection(user.id, saved.id).unwrap();
        assert!(matches!(store.connection(user.id, saved.id), Err(StoreError::NotFound)));
        assert!(store.connections(user.id).unwrap().is_empty());
    }

    #[test]
    fn users_cannot_touch_each_others_data() {
        let (store, alice) = store_with_user();
        let bob = store.create_user("bob@example.com", None, true, 0).unwrap();
        let saved = store.add_connection(alice.id, details("a"), None).unwrap();

        assert!(store.connections(bob.id).unwrap().is_empty());
        assert!(matches!(store.connection(bob.id, saved.id), Err(StoreError::NotFound)));
        assert!(matches!(
            store.update_connection(bob.id, saved.id, details("x"), None),
            Err(StoreError::NotFound)
        ));
        assert!(matches!(store.delete_connection(bob.id, saved.id), Err(StoreError::NotFound)));

        store.trust_host(alice.id, "h", 22, "ssh-ed25519 AAAA").unwrap();
        assert_eq!(store.known_host(bob.id, "h", 22).unwrap(), None);
    }

    #[test]
    fn rejects_incomplete_connections() {
        let (store, user) = store_with_user();
        let mut bad = details("x");
        bad.host = "  ".into();
        assert!(matches!(
            store.add_connection(user.id, bad, None),
            Err(StoreError::Invalid(_))
        ));
    }

    #[test]
    fn known_hosts_round_trip() {
        let (store, user) = store_with_user();
        assert_eq!(store.known_host(user.id, "h", 22).unwrap(), None);
        store.trust_host(user.id, "h", 22, "ssh-ed25519 AAAA").unwrap();
        assert_eq!(
            store.known_host(user.id, "h", 22).unwrap().as_deref(),
            Some("ssh-ed25519 AAAA")
        );
        assert_eq!(store.known_host(user.id, "h", 2222).unwrap(), None);
        store.forget_host(user.id, "h", 22).unwrap();
        assert_eq!(store.known_host(user.id, "h", 22).unwrap(), None);
    }
}
