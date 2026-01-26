//! Database repository for simulation CRUD operations

use std::sync::Arc;
use rusqlite::{params, Connection};
use tokio::sync::Mutex;

use crate::models::{FullSimulation, StoredLog, StoredSimulation, StoredTrace};

/// Repository for simulation database operations
pub struct SimulationRepository {
    conn: Arc<Mutex<Connection>>,
}

impl SimulationRepository {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Store a new simulation
    pub async fn create_simulation(&self, sim: &StoredSimulation) -> eyre::Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            r#"
            INSERT INTO simulations (
                id, created_at, chain_id, block_number, from_address, to_address,
                value, gas_limit, gas_used, success, exit_reason, return_data, input_data, tx_hash
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
            "#,
            params![
                sim.id,
                sim.created_at,
                sim.chain_id as i64,
                sim.block_number as i64,
                sim.from_address,
                sim.to_address,
                sim.value,
                sim.gas_limit,
                sim.gas_used as i64,
                sim.success as i32,
                sim.exit_reason,
                sim.return_data,
                sim.input_data,
                sim.tx_hash,
            ],
        )?;
        Ok(())
    }

    /// Store trace data for a simulation
    pub async fn create_trace(&self, trace: &StoredTrace) -> eyre::Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            r#"
            INSERT INTO traces (simulation_id, trace_json, formatted_trace)
            VALUES (?1, ?2, ?3)
            "#,
            params![trace.simulation_id, trace.trace_json, trace.formatted_trace],
        )?;
        Ok(())
    }

    /// Store logs for a simulation
    pub async fn create_logs(&self, logs: &[StoredLog]) -> eyre::Result<()> {
        let conn = self.conn.lock().await;
        for log in logs {
            conn.execute(
                r#"
                INSERT INTO logs (simulation_id, address, topics, data, log_index)
                VALUES (?1, ?2, ?3, ?4, ?5)
                "#,
                params![
                    log.simulation_id,
                    log.address,
                    log.topics,
                    log.data,
                    log.log_index as i32,
                ],
            )?;
        }
        Ok(())
    }

    /// Get a simulation by ID
    pub async fn get_simulation(&self, id: &str) -> eyre::Result<Option<StoredSimulation>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, created_at, chain_id, block_number, from_address, to_address,
                   value, gas_limit, gas_used, success, exit_reason, return_data, input_data, tx_hash
            FROM simulations WHERE id = ?1
            "#,
        )?;

        let result = stmt.query_row(params![id], |row| {
            Ok(StoredSimulation {
                id: row.get(0)?,
                created_at: row.get(1)?,
                chain_id: row.get::<_, i64>(2)? as u64,
                block_number: row.get::<_, i64>(3)? as u64,
                from_address: row.get(4)?,
                to_address: row.get(5)?,
                value: row.get(6)?,
                gas_limit: row.get(7)?,
                gas_used: row.get::<_, i64>(8)? as u64,
                success: row.get::<_, i32>(9)? != 0,
                exit_reason: row.get(10)?,
                return_data: row.get(11)?,
                input_data: row.get(12)?,
                tx_hash: row.get(13)?,
            })
        });

        match result {
            Ok(sim) => Ok(Some(sim)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get trace for a simulation
    pub async fn get_trace(&self, simulation_id: &str) -> eyre::Result<Option<StoredTrace>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            r#"
            SELECT simulation_id, trace_json, formatted_trace
            FROM traces WHERE simulation_id = ?1
            "#,
        )?;

        let result = stmt.query_row(params![simulation_id], |row| {
            Ok(StoredTrace {
                simulation_id: row.get(0)?,
                trace_json: row.get(1)?,
                formatted_trace: row.get(2)?,
            })
        });

        match result {
            Ok(trace) => Ok(Some(trace)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get logs for a simulation
    pub async fn get_logs(&self, simulation_id: &str) -> eyre::Result<Vec<StoredLog>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare(
            r#"
            SELECT id, simulation_id, address, topics, data, log_index
            FROM logs WHERE simulation_id = ?1 ORDER BY log_index
            "#,
        )?;

        let logs = stmt
            .query_map(params![simulation_id], |row| {
                Ok(StoredLog {
                    id: Some(row.get(0)?),
                    simulation_id: row.get(1)?,
                    address: row.get(2)?,
                    topics: row.get(3)?,
                    data: row.get(4)?,
                    log_index: row.get::<_, i32>(5)? as u32,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(logs)
    }

    /// Get full simulation with trace and logs
    pub async fn get_full_simulation(&self, id: &str) -> eyre::Result<Option<FullSimulation>> {
        let simulation = self.get_simulation(id).await?;

        match simulation {
            Some(sim) => {
                let trace = self.get_trace(id).await?;
                let logs = self.get_logs(id).await?;
                Ok(Some(FullSimulation {
                    simulation: sim,
                    trace,
                    logs,
                }))
            }
            None => Ok(None),
        }
    }

    /// Delete old simulations (for retention policy)
    pub async fn delete_older_than(&self, timestamp: i64) -> eyre::Result<u64> {
        let conn = self.conn.lock().await;
        let deleted = conn.execute(
            "DELETE FROM simulations WHERE created_at < ?1",
            params![timestamp],
        )?;
        Ok(deleted as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    async fn setup_db() -> (Database, SimulationRepository) {
        let db = Database::open_in_memory().unwrap();
        db.init().await.unwrap();
        let repo = SimulationRepository::new(db.connection());
        (db, repo)
    }

    #[tokio::test]
    async fn test_create_and_get_simulation() {
        let (_db, repo) = setup_db().await;

        let sim = StoredSimulation {
            id: "test123".to_string(),
            created_at: 1234567890,
            chain_id: 1,
            block_number: 12345678,
            from_address: "0x1234".to_string(),
            to_address: Some("0x5678".to_string()),
            value: "0x0".to_string(),
            gas_limit: "0x100000".to_string(),
            gas_used: 21000,
            success: true,
            exit_reason: "Stop".to_string(),
            return_data: "0x".to_string(),
            input_data: Some("0x12345678".to_string()),
            tx_hash: None,
        };

        repo.create_simulation(&sim).await.unwrap();

        let retrieved = repo.get_simulation("test123").await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.id, "test123");
        assert_eq!(retrieved.chain_id, 1);
        assert!(retrieved.success);
    }

    #[tokio::test]
    async fn test_get_nonexistent_simulation() {
        let (_db, repo) = setup_db().await;
        let result = repo.get_simulation("nonexistent").await.unwrap();
        assert!(result.is_none());
    }
}
