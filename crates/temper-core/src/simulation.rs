//! Core simulation types and execution logic

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::evm::{
    AccessList, Address, Bytes, CallKind, CallRawRequest, CallTraceArena, CallTraceNode,
    Evm, InstructionResult, Log, StorageOverride, U256,
};
#[cfg(feature = "tempo")]
use crate::tip20;

/// Tempo chain ID for testnet
#[cfg(feature = "tempo")]
const TEMPO_TESTNET_CHAIN_ID: u64 = 42429;

/// TIP-20 bytecode with REAL storage access for balanceOf
#[cfg(feature = "tempo")]
const TIP20_BYTECODE: &str = "6004351919600052600960205260406000205460005260206000f3";

/// Get the TIP-20 bytecode as Bytes
#[cfg(feature = "tempo")]
fn get_tip20_bytecode() -> Bytes {
    hex::decode(TIP20_BYTECODE)
        .expect("Invalid TIP-20 bytecode hex")
        .into()
}

/// Errors that can occur during simulation
#[derive(Debug)]
pub enum SimulationError {
    NoRpcUrlForChain(u64),
    IncorrectChainId { expected: u64, actual: u64 },
    MultipleChainIds,
    InvalidBlockNumbers,
    OverrideError,
    EvmError(eyre::Report),
    MissingToAddress,
    StateNotFound,
}

impl std::fmt::Display for SimulationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoRpcUrlForChain(id) => write!(f, "No RPC URL configured for chain {}", id),
            Self::IncorrectChainId { expected, actual } => {
                write!(f, "Chain ID mismatch: expected {}, got {}", expected, actual)
            }
            Self::MultipleChainIds => write!(f, "Multiple chain IDs in bundle"),
            Self::InvalidBlockNumbers => write!(f, "Invalid block numbers in bundle"),
            Self::OverrideError => write!(f, "Failed to apply state override"),
            Self::EvmError(e) => write!(f, "EVM error: {}", e),
            Self::MissingToAddress => write!(f, "'to' field is required for single-call transactions"),
            Self::StateNotFound => write!(f, "Stateful simulation not found"),
        }
    }
}

impl std::error::Error for SimulationError {}

impl From<crate::evm::OverrideError> for SimulationError {
    fn from(_: crate::evm::OverrideError) -> Self {
        SimulationError::OverrideError
    }
}

impl From<crate::evm::EvmError> for SimulationError {
    fn from(e: crate::evm::EvmError) -> Self {
        SimulationError::EvmError(e.0)
    }
}

// Also implement for eyre::Report to make ? work with EVM errors
impl From<eyre::Report> for SimulationError {
    fn from(e: eyre::Report) -> Self {
        SimulationError::EvmError(e)
    }
}

/// Calculate access list intrinsic gas cost per EIP-2930
fn calculate_access_list_gas(access_list: &Option<AccessList>) -> u64 {
    const ACCESS_LIST_ADDRESS_COST: u64 = 2400;
    const ACCESS_LIST_STORAGE_KEY_COST: u64 = 1900;

    match access_list {
        Some(list) => {
            list.iter()
                .map(|(_, storage_keys)| {
                    ACCESS_LIST_ADDRESS_COST
                        + (storage_keys.len() as u64) * ACCESS_LIST_STORAGE_KEY_COST
                })
                .sum()
        }
        None => 0,
    }
}

/// A single call in a Tempo batch transaction
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TempoCall {
    pub to: Address,
    pub data: Option<Bytes>,
    pub value: Option<U256>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SimulationRequest {
    pub chain_id: u64,
    pub from: Address,
    /// Target address for single call (use `calls` for batch transactions)
    #[serde(default)]
    pub to: Option<Address>,
    /// Calldata for single call (use `calls` for batch transactions)
    pub data: Option<Bytes>,
    pub gas_limit: U256,
    /// Value for single call (use `calls` for batch transactions)
    pub value: Option<U256>,
    /// Batch calls for Tempo transaction type (0x76)
    pub calls: Option<Vec<TempoCall>>,
    /// Fee payer address for sponsored transactions (Tempo 0x76)
    pub fee_payer: Option<Address>,
    /// Fee token address for paying gas in ERC20 (Tempo 0x76)
    pub fee_token: Option<Address>,
    pub access_list: Option<AccessList>,
    pub block_number: Option<u64>,
    pub block_timestamp: Option<U256>,
    pub state_overrides: Option<HashMap<Address, StateOverride>>,
    pub format_trace: Option<bool>,
    /// Original transaction hash (optional, for reference)
    pub tx_hash: Option<String>,
}

/// Result of a single call in a batch transaction
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CallResult {
    pub success: bool,
    pub gas_used: u64,
    pub return_data: Bytes,
    pub exit_reason: InstructionResult,
    pub logs: Vec<Log>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SimulationResponse {
    /// Numeric simulation ID (legacy, for backwards compatibility)
    pub simulation_id: u64,
    /// String simulation ID for database storage (nanoid)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// URL to view this simulation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub simulation_url: Option<String>,
    /// Chain ID for the simulation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<u64>,
    /// Original transaction hash (if provided)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    pub gas_used: u64,
    pub block_number: u64,
    pub success: bool,
    pub trace: Vec<CallTrace>,
    /// Enhanced trace tree for viewer compatibility
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enhanced_trace: Option<EnhancedCallTrace>,
    pub formatted_trace: Option<String>,
    pub logs: Vec<Log>,
    pub exit_reason: InstructionResult,
    pub return_data: Bytes,
    /// Results for each call in a batch transaction (Tempo 0x76)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_results: Option<Vec<CallResult>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatefulSimulationRequest {
    pub chain_id: u64,
    pub gas_limit: U256,
    pub block_number: Option<u64>,
    pub block_timestamp: Option<U256>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StatefulSimulationResponse {
    pub stateful_simulation_id: uuid::Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StatefulSimulationEndResponse {
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StateOverride {
    pub balance: Option<U256>,
    pub nonce: Option<u64>,
    pub code: Option<Bytes>,
    #[serde(flatten)]
    pub state: Option<State>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum State {
    Full {
        state: HashMap<U256, U256>,
    },
    #[serde(rename_all = "camelCase")]
    Diff {
        state_diff: HashMap<U256, U256>,
    },
}

impl From<State> for StorageOverride {
    fn from(value: State) -> Self {
        let (slots, diff) = match value {
            State::Full { state } => (state, false),
            State::Diff { state_diff } => (state_diff, true),
        };

        StorageOverride { slots, diff }
    }
}

/// Simple call trace (legacy format)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CallTrace {
    pub call_type: CallKind,
    pub from: Address,
    pub to: Address,
    pub value: U256,
}

impl From<CallTraceNode> for CallTrace {
    fn from(item: CallTraceNode) -> Self {
        CallTrace {
            call_type: item.trace.kind,
            from: item.trace.caller,
            to: item.trace.address,
            value: item.trace.value,
        }
    }
}

/// Enhanced call trace matching Geth debug_traceTransaction format
/// Used by evm-trace-viewer for visualization
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EnhancedCallTrace {
    /// Call type: "CALL", "DELEGATECALL", "STATICCALL", "CREATE", "CREATE2"
    #[serde(rename = "type")]
    pub call_type: String,
    pub from: Address,
    pub to: Address,
    /// Value in hex
    pub value: String,
    /// Input calldata in hex
    pub input: String,
    /// Output/return data in hex
    pub output: String,
    /// Gas used in hex
    pub gas_used: String,
    /// Error message if call failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Decoded revert reason if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revert_reason: Option<String>,
    /// Nested calls (tree structure)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calls: Vec<EnhancedCallTrace>,
}

impl EnhancedCallTrace {
    /// Convert a CallTraceArena to an EnhancedCallTrace tree
    pub fn from_arena(arena: &CallTraceArena) -> Option<Self> {
        let nodes = arena.nodes();
        if nodes.is_empty() {
            return None;
        }

        // Build tree recursively starting from root (index 0)
        Self::build_tree(nodes, 0)
    }

    fn build_tree(nodes: &[CallTraceNode], idx: usize) -> Option<Self> {
        let node = nodes.get(idx)?;
        let trace = &node.trace;

        // Convert CallKind to string
        let call_type = format!("{:?}", trace.kind).to_uppercase();

        // Build children recursively
        let calls: Vec<EnhancedCallTrace> = node
            .children
            .iter()
            .filter_map(|&child_idx| Self::build_tree(nodes, child_idx))
            .collect();

        // Determine error/revert info
        let (error, revert_reason) = if trace.success {
            (None, None)
        } else {
            let error = Some(format!("{:?}", trace.status));
            // Try to decode revert reason from output
            let revert_reason = Self::try_decode_revert(&trace.output);
            (error, revert_reason)
        };

        Some(EnhancedCallTrace {
            call_type,
            from: trace.caller,
            to: trace.address,
            value: format!("0x{:x}", trace.value),
            input: format!("0x{}", hex::encode(&trace.data)),
            output: format!("0x{}", hex::encode(&trace.output)),
            gas_used: format!("0x{:x}", trace.gas_used),
            error,
            revert_reason,
            calls,
        })
    }

    fn try_decode_revert(output: &Bytes) -> Option<String> {
        // Check for Error(string) selector: 0x08c379a0
        if output.len() >= 4 && output[..4] == [0x08, 0xc3, 0x79, 0xa0] {
            // Try to decode the string
            if output.len() >= 68 {
                let offset = 36; // 4 (selector) + 32 (offset pointer)
                let len_start = offset;
                if output.len() >= len_start + 32 {
                    let len = u64::from_be_bytes(
                        output[len_start + 24..len_start + 32]
                            .try_into()
                            .ok()?
                    ) as usize;
                    let str_start = len_start + 32;
                    if output.len() >= str_start + len {
                        return String::from_utf8(output[str_start..str_start + len].to_vec()).ok();
                    }
                }
            }
        }
        None
    }
}

/// Execute a simulation on the given EVM
pub async fn run_simulation(
    evm: &mut Evm,
    transaction: SimulationRequest,
    commit: bool,
) -> Result<SimulationResponse, SimulationError> {
    // Apply state overrides
    for (address, state_override) in transaction.state_overrides.into_iter().flatten() {
        evm.override_account(
            address,
            state_override.balance.map(U256::from),
            state_override.nonce,
            state_override.code,
            state_override.state.map(StorageOverride::from),
        )?;
    }

    // If fee_payer is set (Tempo sponsored transaction), give them balance for gas
    if let Some(fee_payer) = transaction.fee_payer {
        evm.override_account(
            fee_payer,
            Some(U256::from(1_000_000_000_000_000_000_000u128)), // 1000 ETH equivalent
            None,
            None,
            None,
        )?;
    }

    // For Tempo chain, inject mock TIP-20 bytecode at TIP-20 addresses
    #[cfg(feature = "tempo")]
    if transaction.chain_id == TEMPO_TESTNET_CHAIN_ID {
        let mut tip20_addresses: Vec<Address> = Vec::new();

        if let Some(to) = transaction.to {
            if tip20::is_tip20_address(&to) {
                tip20_addresses.push(to);
            }
        }

        if let Some(ref calls) = transaction.calls {
            for call in calls {
                if tip20::is_tip20_address(&call.to) {
                    tip20_addresses.push(call.to);
                }
            }
        }

        let tip20_bytecode = get_tip20_bytecode();
        for address in tip20_addresses {
            evm.override_account(
                address,
                None,
                None,
                Some(tip20_bytecode.clone()),
                None,
            )?;
        }
    }

    let format_trace = transaction.format_trace.unwrap_or_default();

    // Check if this is a batch transaction (Tempo 0x76 style)
    if let Some(calls) = transaction.calls {
        let access_list_gas = calculate_access_list_gas(&transaction.access_list);
        let mut total_gas_used = access_list_gas;
        let mut all_traces = Vec::new();
        let mut all_logs = Vec::new();
        let mut call_results = Vec::new();
        let mut all_success = true;
        let mut last_exit_reason = InstructionResult::Stop;
        let mut last_return_data = Bytes::default();
        let mut block_number = 0u64;
        let mut formatted_traces = Vec::new();
        let mut last_trace_arena: Option<CallTraceArena> = None;

        for tempo_call in calls {
            let call = CallRawRequest {
                from: transaction.from,
                to: tempo_call.to,
                value: tempo_call.value.map(U256::from),
                data: tempo_call.data,
                access_list: transaction.access_list.clone(),
                format_trace,
            };

            let result = evm.call_raw_committing(call, transaction.gas_limit).await?;

            total_gas_used += result.gas_used;
            block_number = result.block_number;

            if let Some(ref trace) = result.trace {
                let arena: &CallTraceArena = trace;
                last_trace_arena = Some(arena.clone());
                all_traces.extend(arena.clone().into_nodes().into_iter().map(CallTrace::from));
            }
            all_logs.extend(result.logs.clone());

            if let Some(ft) = &result.formatted_trace {
                formatted_traces.push(ft.clone());
            }

            let call_success = result.success;
            if !call_success {
                all_success = false;
            }

            call_results.push(CallResult {
                success: call_success,
                gas_used: result.gas_used,
                return_data: result.return_data.clone(),
                exit_reason: result.exit_reason,
                logs: result.logs,
            });

            last_exit_reason = result.exit_reason;
            last_return_data = result.return_data;
        }

        let formatted_trace = if formatted_traces.is_empty() {
            None
        } else {
            Some(formatted_traces.join("\n---\n"))
        };

        let enhanced_trace = last_trace_arena.as_ref().and_then(EnhancedCallTrace::from_arena);

        return Ok(SimulationResponse {
            simulation_id: 1,
            id: None,
            simulation_url: None,
            chain_id: Some(transaction.chain_id),
            tx_hash: None,
            gas_used: total_gas_used,
            block_number,
            success: all_success,
            trace: all_traces,
            enhanced_trace,
            formatted_trace,
            logs: all_logs,
            exit_reason: last_exit_reason,
            return_data: last_return_data,
            call_results: Some(call_results),
        });
    }

    // Single call (backward compatible)
    let to = transaction.to.ok_or(SimulationError::MissingToAddress)?;
    let access_list_gas = calculate_access_list_gas(&transaction.access_list);
    let call = CallRawRequest {
        from: transaction.from,
        to,
        value: transaction.value.map(U256::from),
        data: transaction.data,
        access_list: transaction.access_list,
        format_trace,
    };
    let result = if commit {
        evm.call_raw_committing(call, transaction.gas_limit).await?
    } else {
        evm.call_raw(call).await?
    };

    let trace: Vec<CallTrace> = result
        .trace
        .as_ref()
        .map(|t| {
            let arena: &CallTraceArena = t;
            arena.clone().into_nodes().into_iter().map(CallTrace::from).collect()
        })
        .unwrap_or_default();

    let enhanced_trace = result
        .trace
        .as_ref()
        .and_then(|t| EnhancedCallTrace::from_arena(t));

    Ok(SimulationResponse {
        simulation_id: 1,
        id: None,
        simulation_url: None,
        chain_id: Some(transaction.chain_id),
        tx_hash: None,
        gas_used: result.gas_used + access_list_gas,
        block_number: result.block_number,
        success: result.success,
        trace,
        enhanced_trace,
        logs: result.logs,
        exit_reason: result.exit_reason,
        formatted_trace: result.formatted_trace,
        return_data: result.return_data,
        call_results: None,
    })
}
