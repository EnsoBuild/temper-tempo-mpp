//! Database repository for simulation CRUD operations

use std::sync::Arc;
use rusqlite::{params, Connection};
use tokio::sync::Mutex;

use crate::models::{CachedContract, FullSimulation, StoredLog, StoredSimulation, StoredTrace};

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

/// Repository for contract cache operations
pub struct ContractCacheRepository {
    conn: Arc<Mutex<Connection>>,
}

impl ContractCacheRepository {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Get a cached contract by chain_id and address
    pub async fn get_contract(&self, chain_id: u64, address: &str) -> eyre::Result<Option<CachedContract>> {
        let address = address.to_lowercase();
        let conn = self.conn.lock().await;

        let mut stmt = conn.prepare(
            r#"
            SELECT chain_id, address, name, abi, selectors, source_code, cached_at, last_accessed
            FROM contract_cache WHERE chain_id = ?1 AND address = ?2
            "#,
        )?;

        let result = stmt.query_row(params![chain_id as i64, address], |row| {
            Ok(CachedContract {
                chain_id: row.get::<_, i64>(0)? as u64,
                address: row.get(1)?,
                name: row.get(2)?,
                abi: row.get(3)?,
                selectors: row.get(4)?,
                source_code: row.get(5)?,
                cached_at: row.get(6)?,
                last_accessed: row.get(7)?,
            })
        });

        match result {
            Ok(contract) => Ok(Some(contract)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Cache a contract (insert or replace)
    pub async fn cache_contract(&self, contract: &CachedContract) -> eyre::Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            r#"
            INSERT OR REPLACE INTO contract_cache (
                chain_id, address, name, abi, selectors, source_code, cached_at, last_accessed
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            params![
                contract.chain_id as i64,
                contract.address.to_lowercase(),
                contract.name,
                contract.abi,
                contract.selectors,
                contract.source_code,
                contract.cached_at,
                contract.last_accessed,
            ],
        )?;
        Ok(())
    }

    /// Update last_accessed timestamp for a contract
    pub async fn update_last_accessed(&self, chain_id: u64, address: &str) -> eyre::Result<()> {
        let address = address.to_lowercase();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE contract_cache SET last_accessed = ?1 WHERE chain_id = ?2 AND address = ?3",
            params![now, chain_id as i64, address],
        )?;
        Ok(())
    }

    /// Get total cache size in bytes (approximate based on TEXT column lengths)
    pub async fn get_cache_size(&self) -> eyre::Result<u64> {
        let conn = self.conn.lock().await;
        let size: i64 = conn.query_row(
            r#"
            SELECT COALESCE(SUM(
                LENGTH(address) +
                COALESCE(LENGTH(name), 0) +
                COALESCE(LENGTH(abi), 0) +
                COALESCE(LENGTH(selectors), 0) +
                COALESCE(LENGTH(source_code), 0)
            ), 0) FROM contract_cache
            "#,
            [],
            |row| row.get(0),
        )?;
        Ok(size as u64)
    }

    /// Get count of cached contracts
    pub async fn get_cache_count(&self) -> eyre::Result<u64> {
        let conn = self.conn.lock().await;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM contract_cache",
            [],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// Evict least recently used contracts until cache is below target size
    /// Returns number of contracts evicted
    pub async fn evict_lru(&self, max_size_bytes: u64) -> eyre::Result<u64> {
        let current_size = self.get_cache_size().await?;
        if current_size <= max_size_bytes {
            return Ok(0);
        }

        let conn = self.conn.lock().await;
        let mut evicted = 0u64;

        // Get contracts ordered by last_accessed (oldest first)
        let mut stmt = conn.prepare(
            r#"
            SELECT chain_id, address,
                   LENGTH(address) + COALESCE(LENGTH(name), 0) + COALESCE(LENGTH(abi), 0) +
                   COALESCE(LENGTH(selectors), 0) + COALESCE(LENGTH(source_code), 0) as size
            FROM contract_cache
            ORDER BY last_accessed ASC
            "#,
        )?;

        let contracts: Vec<(i64, String, i64)> = stmt
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?
            .collect::<Result<Vec<_>, _>>()?;

        drop(stmt);

        let mut freed = 0u64;
        let target_free = current_size - max_size_bytes;

        for (chain_id, address, size) in contracts {
            if freed >= target_free {
                break;
            }
            conn.execute(
                "DELETE FROM contract_cache WHERE chain_id = ?1 AND address = ?2",
                params![chain_id, address],
            )?;
            freed += size as u64;
            evicted += 1;
        }

        Ok(evicted)
    }

    /// Delete contracts not accessed since the given timestamp
    pub async fn delete_stale(&self, older_than: i64) -> eyre::Result<u64> {
        let conn = self.conn.lock().await;
        let deleted = conn.execute(
            "DELETE FROM contract_cache WHERE last_accessed < ?1",
            params![older_than],
        )?;
        Ok(deleted as u64)
    }
}

#[cfg(test)]
mod contract_cache_tests {
    use super::*;
    use crate::Database;

    async fn setup_db() -> (Database, ContractCacheRepository) {
        let db = Database::open_in_memory().unwrap();
        db.init().await.unwrap();
        let repo = ContractCacheRepository::new(db.connection());
        (db, repo)
    }

    #[tokio::test]
    async fn test_cache_and_get_contract() {
        let (_db, repo) = setup_db().await;

        let contract = CachedContract {
            chain_id: 1,
            address: "0x1234567890123456789012345678901234567890".to_string(),
            name: Some("TestToken".to_string()),
            abi: Some(r#"[{"type":"function","name":"transfer"}]"#.to_string()),
            selectors: Some(r#"{"a9059cbb":"transfer(address,uint256)"}"#.to_string()),
            source_code: Some("contract TestToken {}".to_string()),
            cached_at: 1234567890,
            last_accessed: 1234567890,
        };

        repo.cache_contract(&contract).await.unwrap();

        let retrieved = repo.get_contract(1, "0x1234567890123456789012345678901234567890").await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.name, Some("TestToken".to_string()));
    }

    #[tokio::test]
    async fn test_get_nonexistent_contract() {
        let (_db, repo) = setup_db().await;
        let result = repo.get_contract(1, "0xnonexistent").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_cache_size() {
        let (_db, repo) = setup_db().await;

        let contract = CachedContract {
            chain_id: 1,
            address: "0x1234".to_string(),
            name: Some("Test".to_string()),
            abi: Some("[]".to_string()),
            selectors: None,
            source_code: Some("x".repeat(1000)),
            cached_at: 1234567890,
            last_accessed: 1234567890,
        };

        repo.cache_contract(&contract).await.unwrap();

        let size = repo.get_cache_size().await.unwrap();
        assert!(size > 1000); // At least the source code
    }

    #[tokio::test]
    async fn test_lru_eviction() {
        let (_db, repo) = setup_db().await;

        // Add several contracts with different access times
        for i in 0..5 {
            let contract = CachedContract {
                chain_id: 1,
                address: format!("0x{:040x}", i),
                name: Some(format!("Contract{}", i)),
                abi: None,
                selectors: None,
                source_code: Some("x".repeat(100)),
                cached_at: 1000 + i,
                last_accessed: 1000 + i, // Older contracts have older access times
            };
            repo.cache_contract(&contract).await.unwrap();
        }

        let count_before = repo.get_cache_count().await.unwrap();
        assert_eq!(count_before, 5);

        // Evict to a small size (should remove oldest contracts)
        let evicted = repo.evict_lru(200).await.unwrap();
        assert!(evicted > 0);

        let count_after = repo.get_cache_count().await.unwrap();
        assert!(count_after < count_before);
    }
}
