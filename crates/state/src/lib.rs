use rusqlite::{Connection, OptionalExtension};
use std::path::Path;
use thiserror::Error;

const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Error)]
pub enum StateError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("database schema version {0} is newer than this build")]
    FutureSchema(i64),
}

pub struct StateStore {
    conn: Connection,
}

impl StateStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StateError> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;

        let mut store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    pub fn open_memory() -> Result<Self, StateError> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let mut store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<(), StateError> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
             );",
        )?;

        let current: Option<i64> = self
            .conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |row| {
                row.get(0)
            })
            .optional()?
            .flatten();

        let current = current.unwrap_or(0);
        if current > SCHEMA_VERSION {
            return Err(StateError::FutureSchema(current));
        }

        if current < 1 {
            let tx = self.conn.transaction()?;
            tx.execute_batch(
                "CREATE TABLE providers (
                    id TEXT PRIMARY KEY,
                    enabled INTEGER NOT NULL CHECK(enabled IN (0,1)),
                    credential_ref TEXT
                 );
                 CREATE TABLE models (
                    slug TEXT PRIMARY KEY,
                    provider_id TEXT NOT NULL REFERENCES providers(id),
                    display_name TEXT NOT NULL,
                    visible INTEGER NOT NULL CHECK(visible IN (0,1))
                 );
                 CREATE TABLE browser_sessions (
                    id INTEGER PRIMARY KEY CHECK(id = 1),
                    state TEXT NOT NULL,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                 );
                 INSERT INTO schema_migrations(version) VALUES (1);",
            )?;
            tx.commit()?;
        }

        Ok(())
    }

    pub fn schema_version(&self) -> Result<i64, StateError> {
        Ok(self.conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_current_schema() {
        let store = StateStore::open_memory().expect("open memory state");
        assert_eq!(
            store.schema_version().expect("schema version"),
            SCHEMA_VERSION
        );
    }
}
