//! Temper Core - Core simulation logic for EVM transaction simulation

pub mod evm;
pub mod simulation;

pub mod tip20;

// Re-export EVM types
pub use evm::{
    AccessList, Address, Bytes, CallKind, CallRawRequest, CallRawResult, CallTraceArena,
    CallTraceNode, Evm, EvmError, InstructionResult, Log, OverrideError, StorageOverride, U256,
};

// Re-export simulation types
pub use simulation::{
    CallResult, CallTrace, EnhancedCallTrace, SimulationRequest, SimulationResponse, State,
    StateOverride, StatefulSimulationRequest, StatefulSimulationResponse,
    StatefulSimulationEndResponse, TempoCall,
};
