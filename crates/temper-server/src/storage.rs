//! Storage integration - converts between core types and database models

use std::time::{SystemTime, UNIX_EPOCH};

use temper_core::{
    simulation::{EnhancedCallTrace, SimulationRequest, SimulationResponse},
    Log,
};
use temper_db::{
    generate_id, Database, SimulationRepository, StoredLog, StoredSimulation, StoredTrace,
};

use crate::Config;

/// Storage service for persisting simulations
pub struct StorageService {
    repo: SimulationRepository,
    config: Config,
}

impl StorageService {
    pub fn new(db: &Database, config: Config) -> Self {
        Self {
            repo: SimulationRepository::new(db.connection()),
            config,
        }
    }

    /// Store a simulation and return the ID and URL
    pub async fn store_simulation(
        &self,
        request: &SimulationRequest,
        response: &mut SimulationResponse,
    ) -> eyre::Result<String> {
        let id = generate_id();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Create simulation record
        let sim = StoredSimulation {
            id: id.clone(),
            created_at: now,
            chain_id: request.chain_id,
            block_number: response.block_number,
            from_address: format!("{:?}", request.from),
            to_address: request.to.map(|a| format!("{:?}", a)),
            value: request.value.map(|v| format!("0x{:x}", v)).unwrap_or_else(|| "0x0".to_string()),
            gas_limit: format!("0x{:x}", request.gas_limit),
            gas_used: response.gas_used,
            success: response.success,
            exit_reason: format!("{:?}", response.exit_reason),
            return_data: format!("0x{}", hex::encode(&response.return_data)),
            input_data: request.data.as_ref().map(|d| format!("0x{}", hex::encode(d))),
            tx_hash: request.tx_hash.clone(),
        };

        self.repo.create_simulation(&sim).await?;

        // Store trace if present
        if let Some(ref enhanced_trace) = response.enhanced_trace {
            let trace = StoredTrace {
                simulation_id: id.clone(),
                trace_json: serde_json::to_string(enhanced_trace)?,
                formatted_trace: response.formatted_trace.clone(),
            };
            self.repo.create_trace(&trace).await?;
        }

        // Store logs
        let logs: Vec<StoredLog> = response
            .logs
            .iter()
            .enumerate()
            .map(|(idx, log)| log_to_stored(&id, log, idx as u32))
            .collect();

        if !logs.is_empty() {
            self.repo.create_logs(&logs).await?;
        }

        // Update response with ID and URL
        let url = self.config.simulation_url(&id);
        response.id = Some(id.clone());
        response.simulation_url = Some(url);

        Ok(id)
    }

    /// Get a simulation by ID
    pub async fn get_simulation(&self, id: &str) -> eyre::Result<Option<SimulationResponse>> {
        let full = self.repo.get_full_simulation(id).await?;

        match full {
            Some(full) => {
                let sim = full.simulation;

                // Parse enhanced trace from JSON
                let enhanced_trace: Option<EnhancedCallTrace> = full
                    .trace
                    .as_ref()
                    .and_then(|t| serde_json::from_str(&t.trace_json).ok());

                // Convert logs back
                let logs: Vec<Log> = full
                    .logs
                    .iter()
                    .filter_map(|l| stored_to_log(l).ok())
                    .collect();

                // Parse exit reason (simplified - just use the string)
                let exit_reason = temper_core::InstructionResult::Stop; // Default, actual parsing would need more work

                // Parse return data
                let return_data = hex::decode(sim.return_data.trim_start_matches("0x"))
                    .unwrap_or_default()
                    .into();

                Ok(Some(SimulationResponse {
                    simulation_id: 1,
                    id: Some(sim.id.clone()),
                    simulation_url: Some(self.config.simulation_url(&sim.id)),
                    chain_id: Some(sim.chain_id),
                    tx_hash: sim.tx_hash,
                    gas_used: sim.gas_used,
                    block_number: sim.block_number,
                    success: sim.success,
                    trace: vec![], // Legacy trace not stored
                    enhanced_trace,
                    formatted_trace: full.trace.and_then(|t| t.formatted_trace),
                    logs,
                    exit_reason,
                    return_data,
                    call_results: None,
                    is_tempo_tx: None,
                    fee_token: None,
                    transaction_type: None,
                    tx_from: None,
                    tx_to: None,
                }))
            }
            None => Ok(None),
        }
    }

    /// Get trace for viewer
    pub async fn get_trace(&self, id: &str) -> eyre::Result<Option<EnhancedCallTrace>> {
        let trace = self.repo.get_trace(id).await?;
        match trace {
            Some(t) => Ok(serde_json::from_str(&t.trace_json).ok()),
            None => Ok(None),
        }
    }
}

fn log_to_stored(sim_id: &str, log: &Log, idx: u32) -> StoredLog {
    let topics: Vec<String> = log
        .topics()
        .iter()
        .map(|t| format!("0x{}", hex::encode(t)))
        .collect();

    StoredLog {
        id: None,
        simulation_id: sim_id.to_string(),
        address: format!("{:?}", log.address),
        topics: serde_json::to_string(&topics).unwrap_or_else(|_| "[]".to_string()),
        data: format!("0x{}", hex::encode(log.data.data.as_ref())),
        log_index: idx,
    }
}

fn stored_to_log(stored: &StoredLog) -> eyre::Result<Log> {
    use temper_core::{Address, Bytes};

    // Parse address
    let addr_str = stored.address.trim_start_matches("0x");
    let addr_bytes = hex::decode(addr_str)?;
    let address = Address::from_slice(&addr_bytes);

    // Parse topics
    let topic_strs: Vec<String> = serde_json::from_str(&stored.topics)?;
    let mut topics = Vec::new();
    for t in topic_strs {
        let t_str = t.trim_start_matches("0x");
        let t_bytes = hex::decode(t_str)?;
        if t_bytes.len() == 32 {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&t_bytes);
            topics.push(alloy_primitives::B256::from(arr));
        }
    }

    // Parse data
    let data_str = stored.data.trim_start_matches("0x");
    let data = Bytes::from(hex::decode(data_str)?);

    // Construct Log using the builder pattern or direct construction
    // The exact API depends on the revm version
    Ok(Log::new(address, topics, data).expect("valid log"))
}
