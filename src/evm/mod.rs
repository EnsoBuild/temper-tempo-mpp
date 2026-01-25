//! EVM execution backends
//!
//! This module provides two EVM backends controlled by feature flags:
//! - `ethereum` (default): Uses latest foundry for Ethereum and EVM-compatible chains
//! - `tempo`: Uses tempo-foundry for Tempo chain with TIP-20 precompile support
//!
//! Both backends use alloy types for consistency.

#[cfg(feature = "ethereum")]
mod ethereum;

#[cfg(feature = "tempo")]
mod tempo;

// Re-export the appropriate backend based on feature
#[cfg(feature = "ethereum")]
pub use ethereum::*;

#[cfg(feature = "tempo")]
pub use tempo::*;

// Re-export common types - both features use alloy now
#[cfg(feature = "tempo")]
pub use foundry_evm_tempo::traces::{CallKind, CallTraceArena, CallTraceNode};

#[cfg(feature = "ethereum")]
pub use foundry_evm_ethereum::traces::{CallKind, CallTraceArena, CallTraceNode};

// Common types from alloy (both features)
pub use alloy_primitives::{Address, Bytes, U256};

#[cfg(feature = "tempo")]
pub use foundry_evm_tempo::revm::interpreter::InstructionResult;
#[cfg(feature = "tempo")]
pub use foundry_evm_tempo::revm::primitives::Log;

#[cfg(feature = "ethereum")]
pub use foundry_evm_ethereum::revm::interpreter::InstructionResult;
#[cfg(feature = "ethereum")]
pub use foundry_evm_ethereum::revm::primitives::Log;

// Ensure exactly one feature is enabled
#[cfg(all(feature = "ethereum", feature = "tempo"))]
compile_error!("Features 'ethereum' and 'tempo' are mutually exclusive. Please enable only one.");

#[cfg(not(any(feature = "ethereum", feature = "tempo")))]
compile_error!("Either 'ethereum' or 'tempo' feature must be enabled.");
