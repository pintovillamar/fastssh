//! SQLite storage: saved connections and trusted host keys.
//!
//! No secrets live here. Passwords and passphrases are asked for on every
//! connect until the encrypted vault exists.

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
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
}

type Result<T> = std::result::Result<T, StoreError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthMethod {
    /// Ask for the password on every connect.
    Password,
    /// A private key file on the machine FastSSH runs on.
    KeyFile { path: String },
}

/// The editable part of a saved connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionDetails {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthMethod,
}

#[derive(Debug, Clone, Serialize)]
pub struct SavedConnection {
    pub id: i64,
    #[serde(flatten)]
    pub details: ConnectionDetails,
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
        if let AuthMethod::KeyFile { path } = &mut self.auth {
            *path = path.trim().to_owned();
            if path.is_empty() {
                return Err(StoreError::Invalid("key file path is required"));
            }
        }
        if self.name.is_empty() {
            self.name = format!("{}@{}", self.username, self.host);
        }
        Ok(self)
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

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::init(Connection::open(path)?)
    }

    pub fn in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(db: Connection) -> Result<Self> {
        db.pragma_update(None, "journal_mode", "WAL")?;
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
        Ok(Self {
            db: Arc::new(Mutex::new(db)),
        })
    }

    fn db(&self) -> MutexGuard<'_, Connection> {
        // A poisoned lock only means another thread panicked mid-query; the
        // connection itself is still usable.
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn connections(&self) -> Result<Vec<SavedConnection>> {
        let db = self.db();
        let mut stmt = db.prepare(
            "SELECT id, name, host, port, username, auth_kind, key_path
             FROM connections ORDER BY name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], read_connection)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn connection(&self, id: i64) -> Result<SavedConnection> {
        self.db()
            .query_row(
                "SELECT id, name, host, port, username, auth_kind, key_path
                 FROM connections WHERE id = ?1",
                [id],
                read_connection,
            )
            .optional()?
            .ok_or(StoreError::NotFound)
    }

    pub fn add_connection(&self, details: ConnectionDetails) -> Result<SavedConnection> {
        let details = details.normalized()?;
        let (kind, key_path) = auth_columns(&details.auth);
        let db = self.db();
        db.execute(
            "INSERT INTO connections (name, host, port, username, auth_kind, key_path)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![details.name, details.host, details.port, details.username, kind, key_path],
        )?;
        Ok(SavedConnection {
            id: db.last_insert_rowid(),
            details,
        })
    }

    pub fn update_connection(&self, id: i64, details: ConnectionDetails) -> Result<SavedConnection> {
        let details = details.normalized()?;
        let (kind, key_path) = auth_columns(&details.auth);
        let changed = self.db().execute(
            "UPDATE connections
             SET name = ?1, host = ?2, port = ?3, username = ?4, auth_kind = ?5, key_path = ?6
             WHERE id = ?7",
            params![details.name, details.host, details.port, details.username, kind, key_path, id],
        )?;
        if changed == 0 {
            return Err(StoreError::NotFound);
        }
        Ok(SavedConnection { id, details })
    }

    pub fn delete_connection(&self, id: i64) -> Result<()> {
        match self.db().execute("DELETE FROM connections WHERE id = ?1", [id])? {
            0 => Err(StoreError::NotFound),
            _ => Ok(()),
        }
    }

    /// The host key we trusted earlier for this server, in OpenSSH format.
    pub fn known_host(&self, host: &str, port: u16) -> Result<Option<String>> {
        Ok(self
            .db()
            .query_row(
                "SELECT public_key FROM known_hosts WHERE host = ?1 AND port = ?2",
                params![host, port],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn trust_host(&self, host: &str, port: u16, public_key: &str) -> Result<()> {
        self.db().execute(
            "INSERT INTO known_hosts (host, port, public_key) VALUES (?1, ?2, ?3)
             ON CONFLICT (host, port) DO UPDATE SET public_key = excluded.public_key",
            params![host, port, public_key],
        )?;
        Ok(())
    }

    pub fn forget_host(&self, host: &str, port: u16) -> Result<()> {
        self.db().execute(
            "DELETE FROM known_hosts WHERE host = ?1 AND port = ?2",
            params![host, port],
        )?;
        Ok(())
    }
}

fn auth_columns(auth: &AuthMethod) -> (&'static str, Option<&str>) {
    match auth {
        AuthMethod::Password => ("password", None),
        AuthMethod::KeyFile { path } => ("key_file", Some(path)),
    }
}

fn read_connection(row: &rusqlite::Row<'_>) -> rusqlite::Result<SavedConnection> {
    let kind: String = row.get(5)?;
    let auth = match (kind.as_str(), row.get::<_, Option<String>>(6)?) {
        ("key_file", Some(path)) => AuthMethod::KeyFile { path },
        _ => AuthMethod::Password,
    };
    Ok(SavedConnection {
        id: row.get(0)?,
        details: ConnectionDetails {
            name: row.get(1)?,
            host: row.get(2)?,
            port: row.get(3)?,
            username: row.get(4)?,
            auth,
        },
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
            auth: AuthMethod::KeyFile {
                path: "~/.ssh/id_ed25519".into(),
            },
        }
    }

    #[test]
    fn connections_round_trip() {
        let store = Store::in_memory().unwrap();
        let saved = store.add_connection(details("")).unwrap();
        assert_eq!(saved.details.name, "root@example.com");
        assert_eq!(saved.details.host, "example.com");

        let mut edited = details("prod");
        edited.auth = AuthMethod::Password;
        store.update_connection(saved.id, edited).unwrap();
        let loaded = store.connection(saved.id).unwrap();
        assert_eq!(loaded.details.name, "prod");
        assert!(matches!(loaded.details.auth, AuthMethod::Password));

        store.delete_connection(saved.id).unwrap();
        assert!(matches!(store.connection(saved.id), Err(StoreError::NotFound)));
        assert!(store.connections().unwrap().is_empty());
    }

    #[test]
    fn rejects_incomplete_connections() {
        let store = Store::in_memory().unwrap();
        let mut bad = details("x");
        bad.host = "  ".into();
        assert!(matches!(store.add_connection(bad), Err(StoreError::Invalid(_))));
    }

    #[test]
    fn known_hosts_round_trip() {
        let store = Store::in_memory().unwrap();
        assert_eq!(store.known_host("h", 22).unwrap(), None);
        store.trust_host("h", 22, "ssh-ed25519 AAAA").unwrap();
        assert_eq!(store.known_host("h", 22).unwrap().as_deref(), Some("ssh-ed25519 AAAA"));
        assert_eq!(store.known_host("h", 2222).unwrap(), None);
        store.forget_host("h", 22).unwrap();
        assert_eq!(store.known_host("h", 22).unwrap(), None);
    }
}
