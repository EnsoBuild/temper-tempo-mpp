use std::collections::HashMap;
use std::sync::Arc;

use dashmap::mapref::one::RefMut;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use uuid::Uuid;
use warp::reply::Json;
use warp::Rejection;

use crate::errors::{
    IncorrectChainIdError, InvalidBlockNumbersError, MultipleChainIdsError, NoURLForChainIdError,
    StateNotFound,
};
// Import types from the feature-gated evm module
use crate::evm::{
    AccessList, Address, Bytes, CallKind, InstructionResult, Log, StorageOverride, U256,
};
#[cfg(feature = "tempo")]
use crate::tip20;
use crate::SharedSimulationState;

use super::config::Config;
use super::evm::{CallRawRequest, Evm};

/// Tempo chain ID for testnet
#[cfg(feature = "tempo")]
const TEMPO_TESTNET_CHAIN_ID: u64 = 42429;

/// TIP-20 bytecode with REAL storage access for balanceOf
///
/// This bytecode reads balances from TIP-20's storage slot 9:
/// 1. Load address from calldata[4:36]
/// 2. Compute storage slot: keccak256(abi.encode(address, 9))
/// 3. SLOAD and return the balance
///
/// TIP-20 Storage layout (from tempo-foundry):
/// - Slot 8: uint256 _totalSupply
/// - Slot 9: mapping(address => uint256) _balances
/// - Slot 10: mapping(address => mapping(address => uint256)) _allowances
///
/// Note: Uses NOT NOT workaround after CALLDATALOAD due to revm execution quirk.
/// For functions other than balanceOf, returns uint256(1) which represents:
/// - true for transfer/approve/transferFrom
/// - 1 for other numeric returns
///
/// Full TIP-20 precompile logic is available in the tip20 module for reference.
#[cfg(feature = "tempo")]
const TIP20_BYTECODE: &str = "6004351919600052600960205260406000205460005260206000f3";

/// Get the TIP-20 bytecode as Bytes
#[cfg(feature = "tempo")]
fn get_tip20_bytecode() -> Bytes {
    hex::decode(TIP20_BYTECODE)
        .expect("Invalid TIP-20 bytecode hex")
        .into()
}

/// Calculate access list intrinsic gas cost per EIP-2930
/// - 2400 gas per address
/// - 1900 gas per storage key
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
    /// When provided, `to`, `data`, and `value` are ignored
    pub calls: Option<Vec<TempoCall>>,
    /// Fee payer address for sponsored transactions (Tempo 0x76)
    /// The fee payer's balance will be used for gas costs
    pub fee_payer: Option<Address>,
    /// Fee token address for paying gas in ERC20 (Tempo 0x76)
    /// When set, gas is conceptually paid in this token instead of native
    pub fee_token: Option<Address>,
    pub access_list: Option<AccessList>,
    pub block_number: Option<u64>,
    pub block_timestamp: Option<U256>,
    pub state_overrides: Option<HashMap<Address, StateOverride>>,
    pub format_trace: Option<bool>,
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
    pub simulation_id: u64,
    pub gas_used: u64,
    pub block_number: u64,
    pub success: bool,
    pub trace: Vec<CallTrace>,
    pub formatted_trace: Option<String>,
    pub logs: Vec<Log>,
    pub exit_reason: InstructionResult,
    pub return_data: Bytes,
    /// Results for each call in a batch transaction (Tempo 0x76)
    /// Only present when `calls` was provided in the request
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
    pub stateful_simulation_id: Uuid,
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

// Both features use alloy types now, so this conversion is the same
impl From<State> for StorageOverride {
    fn from(value: State) -> Self {
        let (slots, diff) = match value {
            State::Full { state } => (state, false),
            State::Diff { state_diff } => (state_diff, true),
        };

        StorageOverride { slots, diff }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CallTrace {
    pub call_type: CallKind,
    pub from: Address,
    pub to: Address,
    pub value: U256,
}

fn get_rpc_url_for_chain(config: &Config, chain_id: u64) -> Result<String, Rejection> {
    config
        .get_rpc_url(chain_id)
        .cloned()
        .ok_or_else(|| NoURLForChainIdError.into())
}

async fn run(
    evm: &mut Evm,
    transaction: SimulationRequest,
    commit: bool,
) -> Result<SimulationResponse, Rejection> {
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
        // Give the fee payer a large balance to cover gas costs
        // In a real Tempo transaction, this would be handled by the protocol
        evm.override_account(
            fee_payer,
            Some(U256::from(1_000_000_000_000_000_000_000u128)), // 1000 ETH equivalent
            None,
            None,
            None,
        )?;
    }

    // For Tempo chain, inject mock TIP-20 bytecode at TIP-20 addresses
    // TIP-20 tokens are precompiles on Tempo but need bytecode for simulation
    #[cfg(feature = "tempo")]
    if transaction.chain_id == TEMPO_TESTNET_CHAIN_ID {
        // Collect all TIP-20 addresses that will be called
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

        // Inject mock TIP-20 bytecode for each address
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
        // Calculate access list gas once for the transaction
        let access_list_gas = calculate_access_list_gas(&transaction.access_list);
        // Execute batch calls
        let mut total_gas_used = access_list_gas;
        let mut all_traces = Vec::new();
        let mut all_logs = Vec::new();
        let mut call_results = Vec::new();
        let mut all_success = true;
        let mut last_exit_reason = InstructionResult::Stop;
        let mut last_return_data = Bytes::default();
        let mut block_number = 0u64;
        let mut formatted_traces = Vec::new();

        for tempo_call in calls {
            let call = CallRawRequest {
                from: transaction.from,
                to: tempo_call.to,
                value: tempo_call.value.map(U256::from),
                data: tempo_call.data,
                access_list: transaction.access_list.clone(),
                format_trace,
            };

            // Always commit for batch calls to maintain state between calls
            let result = evm.call_raw_committing(call, transaction.gas_limit).await?;

            total_gas_used += result.gas_used;
            block_number = result.block_number;

            if let Some(ref trace) = result.trace {
                // Clone the CallTraceArena from SparsedTraceArena to get ownership
                let arena: &crate::evm::CallTraceArena = trace;
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

        return Ok(SimulationResponse {
            simulation_id: 1,
            gas_used: total_gas_used,
            block_number,
            success: all_success,
            trace: all_traces,
            formatted_trace,
            logs: all_logs,
            exit_reason: last_exit_reason,
            return_data: last_return_data,
            call_results: Some(call_results),
        });
    }

    // Single call (backward compatible)
    let to = transaction.to.expect("'to' field is required for single-call transactions");
    // Calculate access list gas before moving the access_list
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

    Ok(SimulationResponse {
        simulation_id: 1,
        gas_used: result.gas_used + access_list_gas,
        block_number: result.block_number,
        success: result.success,
        trace: result
            .trace
            .as_ref()
            .map(|t| {
                let arena: &crate::evm::CallTraceArena = t;
                arena.clone().into_nodes().into_iter().map(CallTrace::from).collect()
            })
            .unwrap_or_default(),
        logs: result.logs,
        exit_reason: result.exit_reason,
        formatted_trace: result.formatted_trace,
        return_data: result.return_data,
        call_results: None,
    })
}

pub async fn simulate(transaction: SimulationRequest, config: Config) -> Result<Json, Rejection> {
    let fork_url = get_rpc_url_for_chain(&config, transaction.chain_id)?;
    let mut evm = Evm::new(
        None,
        fork_url,
        transaction.block_number,
        transaction.gas_limit,
        config.etherscan_key,
    )
    .await;

    if evm.get_chain_id() != transaction.chain_id {
        return Err(warp::reject::custom(IncorrectChainIdError()));
    }

    if let Some(timestamp) = transaction.block_timestamp {
        evm.set_block_timestamp(U256::from(timestamp))
            .await
            .expect("failed to set block timestamp");
    }

    let response = run(&mut evm, transaction, false).await?;

    Ok(warp::reply::json(&response))
}

pub async fn simulate_bundle(
    transactions: Vec<SimulationRequest>,
    config: Config,
) -> Result<Json, Rejection> {
    let first_chain_id = transactions[0].chain_id;
    let first_block_number = transactions[0].block_number;
    let first_block_timestamp = transactions[0].block_timestamp;

    let fork_url = get_rpc_url_for_chain(&config, first_chain_id)?;
    let mut evm = Evm::new(
        None,
        fork_url,
        first_block_number,
        transactions[0].gas_limit,
        config.etherscan_key,
    )
    .await;

    if evm.get_chain_id() != first_chain_id {
        return Err(warp::reject::custom(IncorrectChainIdError()));
    }

    if let Some(timestamp) = first_block_timestamp {
        evm.set_block_timestamp(timestamp)
            .await
            .expect("failed to set block timestamp");
    }

    let mut response = Vec::with_capacity(transactions.len());
    for transaction in transactions {
        if transaction.chain_id != first_chain_id {
            return Err(warp::reject::custom(MultipleChainIdsError()));
        }
        if transaction.block_number != first_block_number {
            let tx_block = U256::from(
                transaction
                    .block_number
                    .expect("Transaction has no block number"),
            );
            if transaction.block_number < first_block_number || tx_block < evm.get_block() {
                return Err(warp::reject::custom(InvalidBlockNumbersError()));
            }
            evm.set_block(tx_block)
                .await
                .expect("Failed to set block number");
            evm.set_block_timestamp(evm.get_block_timestamp() + U256::from(12)) // TOOD: make block time configurable
                .await
                .expect("Failed to set block timestamp");
        }
        response.push(run(&mut evm, transaction, true).await?);
    }

    Ok(warp::reply::json(&response))
}

pub async fn simulate_stateful_new(
    stateful_simulation_request: StatefulSimulationRequest,
    config: Config,
    state: Arc<SharedSimulationState>,
) -> Result<Json, Rejection> {
    let fork_url = get_rpc_url_for_chain(&config, stateful_simulation_request.chain_id)?;
    let mut evm = Evm::new(
        None,
        fork_url,
        stateful_simulation_request.block_number,
        stateful_simulation_request.gas_limit,
        config.etherscan_key,
    )
    .await;

    if let Some(timestamp) = stateful_simulation_request.block_timestamp {
        evm.set_block_timestamp(timestamp).await?;
    }

    let new_id = Uuid::new_v4();
    state.evms.insert(new_id, Arc::new(Mutex::new(evm)));

    let response = StatefulSimulationResponse {
        stateful_simulation_id: new_id,
    };

    Ok(warp::reply::json(&response))
}

pub async fn simulate_stateful_end(
    param: Uuid,
    state: Arc<SharedSimulationState>,
) -> Result<Json, Rejection> {
    if state.evms.contains_key(&param) {
        state.evms.remove(&param);
        let response = StatefulSimulationEndResponse { success: true };
        Ok(warp::reply::json(&response))
    } else {
        Err(warp::reject::custom(StateNotFound()))
    }
}

pub async fn simulate_stateful(
    param: Uuid,
    transactions: Vec<SimulationRequest>,
    state: Arc<SharedSimulationState>,
) -> Result<Json, Rejection> {
    let first_chain_id = transactions[0].chain_id;
    let first_block_number = transactions[0].block_number;

    let mut response = Vec::with_capacity(transactions.len());

    // Get a mutable reference to the EVM here.
    let evm_ref_mut: RefMut<'_, Uuid, Arc<Mutex<Evm>>> = state
        .evms
        .get_mut(&param)
        .ok_or_else(warp::reject::not_found)?;

    // Dereference to obtain the EVM.
    let evm = evm_ref_mut.value();
    let mut evm = evm.lock().await;

    if evm.get_chain_id() != first_chain_id {
        return Err(warp::reject::custom(IncorrectChainIdError()));
    }

    for transaction in transactions {
        if transaction.chain_id != first_chain_id {
            return Err(warp::reject::custom(MultipleChainIdsError()));
        }
        if transaction.block_number != first_block_number
            || U256::from(transaction.block_number.unwrap_or_default()) != evm.get_block()
        {
            let tx_block = U256::from(
                transaction
                    .block_number
                    .expect("Transaction has no block number"),
            );
            if transaction.block_number < first_block_number || tx_block < evm.get_block() {
                return Err(warp::reject::custom(InvalidBlockNumbersError()));
            }
            evm.set_block(tx_block)
                .await
                .expect("Failed to set block number");
            let block_timestamp = evm.get_block_timestamp();
            evm.set_block_timestamp(block_timestamp + U256::from(12))
                .await
                .expect("Failed to set block timestamp");
        }
        response.push(run(&mut evm, transaction, true).await?);
    }

    Ok(warp::reply::json(&response))
}
