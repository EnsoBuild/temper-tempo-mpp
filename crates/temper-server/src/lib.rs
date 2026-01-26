//! Temper Server - HTTP server for simulation API

pub mod config;
pub mod debugger;
pub mod errors;
pub mod handlers;
pub mod routes;
pub mod storage;
pub mod trace_fetcher;

use std::sync::Arc;

use dashmap::DashMap;
use temper_core::Evm;
use tokio::sync::Mutex;
use uuid::Uuid;

pub use config::Config;

/// Shared state for stateful simulations
pub struct SharedSimulationState {
    pub evms: Arc<DashMap<Uuid, Arc<Mutex<Evm>>>>,
}
