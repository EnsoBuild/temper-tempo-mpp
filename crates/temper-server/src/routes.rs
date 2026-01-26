//! HTTP route definitions

use std::sync::Arc;

use serde::de::DeserializeOwned;
use uuid::Uuid;
use warp::{Filter, Rejection, Reply};

use temper_core::{SimulationRequest, StatefulSimulationRequest};

use crate::config::Config;
use crate::handlers;
use crate::storage::StorageService;
use crate::SharedSimulationState;

/// Create all API routes
pub fn api_routes(
    config: Config,
    state: Arc<SharedSimulationState>,
    storage: Option<Arc<StorageService>>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    simulate(config.clone(), storage.clone())
        .or(simulate_bundle(config.clone(), storage.clone()))
        .or(simulate_stateful_new(config.clone(), state.clone()))
        .or(simulate_stateful_end(state.clone()))
        .or(simulate_stateful(config.clone(), state, storage.clone()))
        .or(get_simulation(storage.clone()))
        .or(get_simulation_trace(storage.clone()))
        .or(get_simulation_debug(config.clone(), storage))
        .or(debug_trace(config.clone()))
        .or(contract_source())
}

/// POST /simulate
fn simulate(
    config: Config,
    storage: Option<Arc<StorageService>>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    warp::path!("simulate")
        .and(warp::post())
        .and(json_body::<SimulationRequest>(&config))
        .and(with_config(config))
        .and(with_storage(storage))
        .and_then(handlers::simulate)
}

/// POST /simulate-bundle
fn simulate_bundle(
    config: Config,
    storage: Option<Arc<StorageService>>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    warp::path!("simulate-bundle")
        .and(warp::post())
        .and(json_body::<Vec<SimulationRequest>>(&config))
        .and(with_config(config))
        .and(with_storage(storage))
        .and_then(handlers::simulate_bundle)
}

/// POST /simulate-stateful (create new)
fn simulate_stateful_new(
    config: Config,
    state: Arc<SharedSimulationState>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    warp::path!("simulate-stateful")
        .and(warp::post())
        .and(json_body::<StatefulSimulationRequest>(&config))
        .and(with_config(config))
        .and(with_state(state))
        .and_then(handlers::simulate_stateful_new)
}

/// DELETE /simulate-stateful/{id}
fn simulate_stateful_end(
    state: Arc<SharedSimulationState>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    warp::path!("simulate-stateful" / Uuid)
        .and(warp::delete())
        .and(with_state(state))
        .and_then(handlers::simulate_stateful_end)
}

/// POST /simulate-stateful/{id}
fn simulate_stateful(
    config: Config,
    state: Arc<SharedSimulationState>,
    storage: Option<Arc<StorageService>>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    warp::path!("simulate-stateful" / Uuid)
        .and(warp::post())
        .and(json_body::<Vec<SimulationRequest>>(&config))
        .and(with_state(state))
        .and(with_storage(storage))
        .and_then(handlers::simulate_stateful)
}

/// GET /simulation/{id}
fn get_simulation(
    storage: Option<Arc<StorageService>>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    warp::path!("simulation" / String)
        .and(warp::get())
        .and(with_required_storage(storage))
        .and_then(handlers::get_simulation)
}

/// GET /simulation/{id}/trace
fn get_simulation_trace(
    storage: Option<Arc<StorageService>>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    warp::path!("simulation" / String / "trace")
        .and(warp::get())
        .and(with_required_storage(storage))
        .and_then(handlers::get_simulation_trace)
}

/// GET /simulation/{id}/debug - Get debug trace for a simulation
fn get_simulation_debug(
    config: Config,
    storage: Option<Arc<StorageService>>,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    warp::path!("simulation" / String / "debug")
        .and(warp::get())
        .and(with_config(config))
        .and(with_required_storage(storage))
        .and_then(handlers::get_simulation_debug)
}

/// POST /debug - Get step-by-step debug trace
fn debug_trace(
    config: Config,
) -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    warp::path!("debug")
        .and(warp::post())
        .and(warp::body::json())
        .and(with_config(config))
        .and_then(handlers::get_debug_trace)
}

/// GET /contract/{chainId}/{address}/source
fn contract_source() -> impl Filter<Extract = (impl Reply,), Error = Rejection> + Clone {
    warp::path!("contract" / u64 / String / "source")
        .and(warp::get())
        .and_then(handlers::get_contract_source)
}

// Filter helpers

fn with_config(
    config: Config,
) -> impl Filter<Extract = (Config,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || config.clone())
}

fn with_state(
    state: Arc<SharedSimulationState>,
) -> impl Filter<Extract = (Arc<SharedSimulationState>,), Error = std::convert::Infallible> + Clone
{
    warp::any().map(move || state.clone())
}

fn with_storage(
    storage: Option<Arc<StorageService>>,
) -> impl Filter<Extract = (Option<Arc<StorageService>>,), Error = std::convert::Infallible> + Clone
{
    warp::any().map(move || storage.clone())
}

fn with_required_storage(
    storage: Option<Arc<StorageService>>,
) -> impl Filter<Extract = (Arc<StorageService>,), Error = Rejection> + Clone {
    warp::any()
        .map(move || storage.clone())
        .and_then(|storage: Option<Arc<StorageService>>| async move {
            storage.ok_or_else(warp::reject::not_found)
        })
}

fn json_body<T: DeserializeOwned + Send>(
    config: &Config,
) -> impl Filter<Extract = (T,), Error = Rejection> + Clone {
    warp::body::content_length_limit(config.max_request_size).and(warp::body::json())
}
