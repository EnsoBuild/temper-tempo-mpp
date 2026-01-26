//! Temper Database - SQLite storage for simulation results

mod models;
mod repository;

pub use models::*;
pub use repository::*;

use std::path::Path;
use std::sync::Arc;
use rusqlite::Connection;
use tokio::sync::Mutex;

/// Database connection wrapper for async operations
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    /// Open or create a database at the given path
    pub fn open(path: impl AsRef<Path>) -> eyre::Result<Self> {
        let conn = Connection::open(path)?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        Ok(db)
    }

    /// Open an in-memory database (useful for testing)
    pub fn open_in_memory() -> eyre::Result<Self> {
        let conn = Connection::open_in_memory()?;
        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        Ok(db)
    }

    /// Initialize the database schema
    pub async fn init(&self) -> eyre::Result<()> {
        let conn = self.conn.lock().await;
        conn.execute_batch(SCHEMA)?;
        Ok(())
    }

    /// Get a reference to the connection for repository operations
    pub fn connection(&self) -> Arc<Mutex<Connection>> {
        self.conn.clone()
    }
}

/// Database schema
const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS simulations (
    id TEXT PRIMARY KEY,              -- nanoid (21 chars, URL-safe)
    created_at INTEGER NOT NULL,
    chain_id INTEGER NOT NULL,
    block_number INTEGER NOT NULL,
    from_address TEXT NOT NULL,
    to_address TEXT,
    value TEXT NOT NULL,
    gas_limit TEXT NOT NULL,
    gas_used INTEGER NOT NULL,
    success INTEGER NOT NULL,
    exit_reason TEXT NOT NULL,
    return_data TEXT NOT NULL,
    input_data TEXT,
    tx_hash TEXT                      -- Original transaction hash (optional)
);

CREATE TABLE IF NOT EXISTS traces (
    simulation_id TEXT PRIMARY KEY,
    trace_json TEXT NOT NULL,         -- Enhanced trace tree as JSON
    formatted_trace TEXT,
    FOREIGN KEY (simulation_id) REFERENCES simulations(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    simulation_id TEXT NOT NULL,
    address TEXT NOT NULL,
    topics TEXT NOT NULL,             -- JSON array of topic hex strings
    data TEXT NOT NULL,
    log_index INTEGER NOT NULL,
    FOREIGN KEY (simulation_id) REFERENCES simulations(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_simulations_created_at ON simulations(created_at);
CREATE INDEX IF NOT EXISTS idx_logs_simulation_id ON logs(simulation_id);
"#;

/// Generate a new simulation ID using nanoid
pub fn generate_id() -> String {
    nanoid::nanoid!(21)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_database_init() {
        let db = Database::open_in_memory().unwrap();
        db.init().await.unwrap();
    }

    #[test]
    fn test_generate_id() {
        let id = generate_id();
        assert_eq!(id.len(), 21);
        // Check URL-safe characters
        assert!(id.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-'));
    }
}
