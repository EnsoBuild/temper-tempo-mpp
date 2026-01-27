//! Database models for stored simulations

use serde::{Deserialize, Serialize};

/// Stored simulation record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredSimulation {
    pub id: String,
    pub created_at: i64,
    pub chain_id: u64,
    pub block_number: u64,
    pub from_address: String,
    pub to_address: Option<String>,
    pub value: String,
    pub gas_limit: String,
    pub gas_used: u64,
    pub success: bool,
    pub exit_reason: String,
    pub return_data: String,
    pub input_data: Option<String>,
    pub tx_hash: Option<String>,
}

/// Stored trace record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTrace {
    pub simulation_id: String,
    pub trace_json: String,
    pub formatted_trace: Option<String>,
}

/// Stored log record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredLog {
    pub id: Option<i64>,
    pub simulation_id: String,
    pub address: String,
    pub topics: String,  // JSON array
    pub data: String,
    pub log_index: u32,
}

/// Complete simulation data for retrieval
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullSimulation {
    pub simulation: StoredSimulation,
    pub trace: Option<StoredTrace>,
    pub logs: Vec<StoredLog>,
}

/// Cached contract metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedContract {
    pub chain_id: u64,
    pub address: String,
    pub name: Option<String>,
    pub abi: Option<String>,        // JSON ABI
    pub selectors: Option<String>,  // JSON map: selector -> function signature
    pub source_code: Option<String>,
    pub cached_at: i64,
    pub last_accessed: i64,
}
