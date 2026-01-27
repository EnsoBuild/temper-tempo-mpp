//! HTTP request handlers

use std::sync::Arc;

use dashmap::mapref::one::RefMut;
use tokio::sync::Mutex;
use uuid::Uuid;
use warp::reply::Json;
use warp::Rejection;

use temper_core::{
    simulation::{run_simulation, SimulationError, SimulationResponse},
    Evm, SimulationRequest, StatefulSimulationEndResponse,
    StatefulSimulationRequest, StatefulSimulationResponse, U256, InstructionResult,
};

use crate::config::Config;
use crate::contract_cache::ContractCacheService;
use crate::errors::{
    IncorrectChainIdError, InvalidBlockNumbersError, MultipleChainIdsError, NoURLForChainIdError,
    SimulationNotFound, StateNotFound,
};
use crate::storage::StorageService;
use crate::trace_fetcher;
use crate::SharedSimulationState;

fn get_rpc_url_for_chain(config: &Config, chain_id: u64) -> Result<String, Rejection> {
    config
        .get_rpc_url(chain_id)
        .cloned()
        .ok_or_else(|| warp::reject::custom(NoURLForChainIdError))
}

/// POST /simulate
pub async fn simulate(
    transaction: SimulationRequest,
    config: Config,
    storage: Option<Arc<StorageService>>,
) -> Result<Json, Rejection> {
    log::info!("Starting simulation for chain {}", transaction.chain_id);
    let fork_url = get_rpc_url_for_chain(&config, transaction.chain_id)?;
    log::info!("Using RPC URL: {}", fork_url);

    // If tx_hash is provided, fetch the actual trace from the RPC instead of simulating
    if let Some(ref tx_hash) = transaction.tx_hash {
        log::info!("Fetching trace for tx_hash: {}", tx_hash);

        let (trace, receipt) = trace_fetcher::fetch_transaction_trace(&fork_url, tx_hash)
            .await
            .map_err(|e| {
                log::error!("Failed to fetch trace: {}", e);
                warp::reject::custom(crate::errors::TraceFetchError(e))
            })?;

        // Parse receipt data
        let gas_used = receipt.gas_used
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);
        let block_number = receipt.block_number
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0);
        let success = receipt.status
            .map(|s| s == "0x1")
            .unwrap_or(true);

        let mut response = SimulationResponse {
            simulation_id: 1,
            id: None,
            simulation_url: None,
            chain_id: Some(transaction.chain_id),
            tx_hash: Some(tx_hash.clone()),
            gas_used,
            block_number,
            success,
            trace: vec![],
            enhanced_trace: Some(trace),
            formatted_trace: None,
            logs: vec![],
            exit_reason: if success { InstructionResult::Stop } else { InstructionResult::Revert },
            return_data: Default::default(),
            call_results: None,
        };

        // Store simulation if storage is enabled
        if let Some(storage) = storage {
            if let Err(e) = storage.store_simulation(&transaction, &mut response).await {
                log::warn!("Failed to store simulation: {}", e);
            }
        }

        return Ok(warp::reply::json(&response));
    }

    log::info!("Creating EVM...");
    let mut evm = Evm::new(
        None,
        fork_url,
        transaction.block_number,
        transaction.gas_limit,
        config.etherscan_key.clone(),
    )
    .await;
    log::info!("EVM created successfully");

    if evm.get_chain_id() != transaction.chain_id {
        return Err(warp::reject::custom(IncorrectChainIdError()));
    }

    if let Some(timestamp) = transaction.block_timestamp {
        evm.set_block_timestamp(U256::from(timestamp))
            .await
            .expect("failed to set block timestamp");
    }

    log::info!("Running simulation...");
    let mut response = run_simulation(&mut evm, transaction.clone(), false)
        .await
        .map_err(crate::errors::simulation_error_to_rejection)?;
    log::info!("Simulation completed successfully");

    // Store simulation if storage is enabled
    if let Some(storage) = storage {
        if let Err(e) = storage.store_simulation(&transaction, &mut response).await {
            log::warn!("Failed to store simulation: {}", e);
        }
    }

    Ok(warp::reply::json(&response))
}

/// POST /simulate-bundle
pub async fn simulate_bundle(
    transactions: Vec<SimulationRequest>,
    config: Config,
    storage: Option<Arc<StorageService>>,
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
        config.etherscan_key.clone(),
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

    let mut responses = Vec::with_capacity(transactions.len());
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
            evm.set_block_timestamp(evm.get_block_timestamp() + U256::from(12))
                .await
                .expect("Failed to set block timestamp");
        }

        let mut response = run_simulation(&mut evm, transaction.clone(), true)
            .await
            .map_err(crate::errors::simulation_error_to_rejection)?;

        // Store each simulation if storage is enabled
        if let Some(ref storage) = storage {
            if let Err(e) = storage.store_simulation(&transaction, &mut response).await {
                log::warn!("Failed to store simulation: {}", e);
            }
        }

        responses.push(response);
    }

    Ok(warp::reply::json(&responses))
}

/// POST /simulate-stateful (create new)
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
        config.etherscan_key.clone(),
    )
    .await;

    if let Some(timestamp) = stateful_simulation_request.block_timestamp {
        evm.set_block_timestamp(timestamp)
            .await
            .map_err(|_| SimulationError::OverrideError)
            .map_err(crate::errors::simulation_error_to_rejection)?;
    }

    let new_id = Uuid::new_v4();
    state.evms.insert(new_id, Arc::new(Mutex::new(evm)));

    let response = StatefulSimulationResponse {
        stateful_simulation_id: new_id,
    };

    Ok(warp::reply::json(&response))
}

/// DELETE /simulate-stateful/{id}
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

/// POST /simulate-stateful/{id}
pub async fn simulate_stateful(
    param: Uuid,
    transactions: Vec<SimulationRequest>,
    state: Arc<SharedSimulationState>,
    storage: Option<Arc<StorageService>>,
) -> Result<Json, Rejection> {
    let first_chain_id = transactions[0].chain_id;
    let first_block_number = transactions[0].block_number;

    let mut responses = Vec::with_capacity(transactions.len());

    let evm_ref_mut: RefMut<'_, Uuid, Arc<Mutex<Evm>>> = state
        .evms
        .get_mut(&param)
        .ok_or_else(warp::reject::not_found)?;

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

        let mut response = run_simulation(&mut evm, transaction.clone(), true)
            .await
            .map_err(crate::errors::simulation_error_to_rejection)?;

        if let Some(ref storage) = storage {
            if let Err(e) = storage.store_simulation(&transaction, &mut response).await {
                log::warn!("Failed to store simulation: {}", e);
            }
        }

        responses.push(response);
    }

    Ok(warp::reply::json(&responses))
}

/// GET /simulation/{id} - Get stored simulation
pub async fn get_simulation(
    id: String,
    storage: Arc<StorageService>,
) -> Result<Json, Rejection> {
    match storage.get_simulation(&id).await {
        Ok(Some(response)) => Ok(warp::reply::json(&response)),
        Ok(None) => Err(warp::reject::custom(SimulationNotFound)),
        Err(e) => {
            log::error!("Database error: {}", e);
            Err(warp::reject::custom(crate::errors::DatabaseError(
                e.to_string(),
            )))
        }
    }
}

/// GET /simulation/{id}/trace - Get trace in viewer format
pub async fn get_simulation_trace(
    id: String,
    storage: Arc<StorageService>,
) -> Result<Json, Rejection> {
    match storage.get_trace(&id).await {
        Ok(Some(trace)) => Ok(warp::reply::json(&trace)),
        Ok(None) => Err(warp::reject::custom(SimulationNotFound)),
        Err(e) => {
            log::error!("Database error: {}", e);
            Err(warp::reject::custom(crate::errors::DatabaseError(
                e.to_string(),
            )))
        }
    }
}

/// Request body for debug endpoint
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugRequest {
    pub tx_hash: String,
    pub chain_id: u64,
}

/// POST /debug - Get step-by-step debug trace for a transaction
pub async fn get_debug_trace(
    request: DebugRequest,
    config: Config,
) -> Result<Json, Rejection> {
    let fork_url = config
        .get_rpc_url(request.chain_id)
        .cloned()
        .ok_or_else(|| warp::reject::custom(NoURLForChainIdError))?;

    log::info!("Fetching debug trace for tx: {}", request.tx_hash);

    // Fetch the step-by-step trace
    let trace = crate::debugger::fetch_debug_trace(&fork_url, &request.tx_hash)
        .await
        .map_err(|e| {
            log::error!("Failed to fetch debug trace: {}", e);
            warp::reject::custom(crate::errors::TraceFetchError(e))
        })?;

    log::info!("Got {} execution steps", trace.struct_logs.len());

    Ok(warp::reply::json(&trace))
}

/// GET /contract/{chainId}/{address}/source - Get contract source from cache or external sources
pub async fn get_contract_source(
    chain_id: u64,
    address: String,
    config: Config,
    contract_cache: Option<Arc<ContractCacheService>>,
) -> Result<Json, Rejection> {
    log::info!("Fetching source for {} on chain {}", address, chain_id);

    // Try cache first if available
    if let Some(cache) = contract_cache {
        match cache.get_contract_source(chain_id, &address).await {
            Ok(Some(source)) => return Ok(warp::reply::json(&source)),
            Ok(None) => {
                log::info!("No verified source found for {}", address);
                return Err(warp::reject::not_found());
            }
            Err(e) => {
                log::error!("Failed to fetch source: {}", e);
                return Err(warp::reject::custom(crate::errors::TraceFetchError(e)));
            }
        }
    }

    // Fallback to direct fetch without caching
    match crate::debugger::fetch_contract_source(&address, chain_id, config.etherscan_key.as_deref()).await {
        Ok(Some(source)) => Ok(warp::reply::json(&source)),
        Ok(None) => {
            log::info!("No verified source found for {}", address);
            Err(warp::reject::not_found())
        }
        Err(e) => {
            log::error!("Failed to fetch source: {}", e);
            Err(warp::reject::custom(crate::errors::TraceFetchError(e)))
        }
    }
}

/// GET /simulation/{id}/debug - Get debug trace for a stored simulation
pub async fn get_simulation_debug(
    id: String,
    config: Config,
    storage: Arc<StorageService>,
) -> Result<Json, Rejection> {
    // Get the stored simulation
    let sim = storage
        .get_simulation(&id)
        .await
        .map_err(|e| warp::reject::custom(crate::errors::DatabaseError(e.to_string())))?
        .ok_or_else(|| warp::reject::custom(SimulationNotFound))?;

    let chain_id = sim.chain_id.unwrap_or(1);

    // If it has a tx_hash, use debug_traceTransaction
    if let Some(ref tx_hash) = sim.tx_hash {
        let fork_url = config
            .get_rpc_url(chain_id)
            .cloned()
            .ok_or_else(|| warp::reject::custom(NoURLForChainIdError))?;

        log::info!("Fetching debug trace for tx: {}", tx_hash);

        let trace = crate::debugger::fetch_debug_trace(&fork_url, tx_hash)
            .await
            .map_err(|e| {
                log::error!("Failed to fetch debug trace: {}", e);
                warp::reject::custom(crate::errors::TraceFetchError(e))
            })?;

        return Ok(warp::reply::json(&trace));
    }

    // Otherwise, we need to re-trace using debug_traceCall
    // This requires the original transaction parameters which we need to fetch from storage
    Err(warp::reject::custom(crate::errors::TraceFetchError(
        "Simulation debug trace not available - no tx_hash stored".to_string(),
    )))
}
