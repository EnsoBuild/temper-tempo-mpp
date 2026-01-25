//! Standard foundry EVM backend for Ethereum and EVM-compatible chains
//! Uses latest foundry with alloy types

use std::collections::HashMap;

use foundry_config_ethereum::{Chain, GasLimit};
use foundry_evm_ethereum::backend::Backend;
use foundry_evm_ethereum::executors::Executor;
use foundry_evm_ethereum::executors::ExecutorBuilder;
use foundry_evm_ethereum::fork::CreateFork;
use foundry_evm_ethereum::opts::EvmOpts;
use foundry_evm_ethereum::Env;

use foundry_evm_ethereum::traces::identifier::{ExternalIdentifier, SignaturesIdentifier};
use foundry_evm_ethereum::traces::{
    CallTraceArena, CallTraceDecoder, CallTraceDecoderBuilder, CallTraceNode, SparsedTraceArena,
    TraceMode,
};

// Use revm types from foundry's re-export
use foundry_evm_ethereum::revm::database::{DatabaseCommit, DatabaseRef};
use foundry_evm_ethereum::revm::interpreter::InstructionResult;
use foundry_evm_ethereum::revm::primitives::Log;
use alloy_primitives::{Address, Bytes, U256};
use foundry_evm_ethereum::revm::state::{Account, AccountStatus, EvmStorageSlot};
use foundry_evm_ethereum::revm::bytecode::Bytecode;

use revm_inspectors::tracing::TraceWriter;

use crate::errors::{EvmError, OverrideError};
use crate::simulation::CallTrace;

pub type AccessList = Vec<(Address, Vec<U256>)>;

#[derive(Debug, Clone)]
pub struct CallRawRequest {
    pub from: Address,
    pub to: Address,
    pub value: Option<U256>,
    pub data: Option<Bytes>,
    pub access_list: Option<AccessList>,
    pub format_trace: bool,
}

#[derive(Debug, Clone)]
pub struct CallRawResult {
    pub gas_used: u64,
    pub block_number: u64,
    pub success: bool,
    pub trace: Option<SparsedTraceArena>,
    pub logs: Vec<Log>,
    pub exit_reason: InstructionResult,
    pub return_data: Bytes,
    pub formatted_trace: Option<String>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct StorageOverride {
    pub slots: HashMap<U256, U256>,
    pub diff: bool,
}

pub struct Evm {
    executor: Executor,
    decoder: CallTraceDecoder,
    etherscan_identifier: Option<ExternalIdentifier>,
}

impl Evm {
    pub async fn new(
        env: Option<Env>,
        fork_url: String,
        fork_block_number: Option<u64>,
        gas_limit: U256,
        etherscan_key: Option<String>,
    ) -> Self {
        let evm_opts = EvmOpts {
            fork_url: Some(fork_url.clone()),
            fork_block_number,
            env: foundry_evm_ethereum::opts::Env {
                chain_id: None,
                code_size_limit: None,
                gas_price: Some(0),
                gas_limit: GasLimit(u64::MAX),
                ..Default::default()
            },
            memory_limit: foundry_config_ethereum::Config::default().memory_limit,
            ..Default::default()
        };

        let fork_opts = CreateFork {
            url: fork_url,
            enable_caching: true,
            env: evm_opts.evm_env().await.unwrap(),
            evm_opts,
        };

        let db = Backend::spawn(Some(fork_opts.clone())).expect("Failed to spawn backend");

        // Convert U256 gas_limit to u64 for the executor
        let gas_limit_u64: u64 = gas_limit.try_into().unwrap_or(u64::MAX);
        let builder = ExecutorBuilder::default().gas_limit(gas_limit_u64);

        let mut executor = builder.build(env.unwrap_or(fork_opts.env.clone()), db);
        executor.set_tracing(TraceMode::Call);

        let foundry_config = foundry_config_ethereum::Config {
            etherscan_api_key: etherscan_key,
            ..Default::default()
        };

        let chain: Chain = executor.env().evm_env.cfg_env.chain_id.into();
        let etherscan_identifier = ExternalIdentifier::new(&foundry_config, Some(chain)).ok().flatten();
        let decoder = CallTraceDecoderBuilder::new().with_verbosity(5);

        let decoder = if let Ok(identifier) = SignaturesIdentifier::new(false) {
            decoder.with_signature_identifier(identifier)
        } else {
            decoder
        };

        Evm {
            executor,
            decoder: decoder.build(),
            etherscan_identifier,
        }
    }

    pub async fn call_raw(&mut self, call: CallRawRequest) -> Result<CallRawResult, EvmError> {
        self.set_access_list(call.access_list)?;
        let res = self
            .executor
            .call_raw(
                call.from,
                call.to,
                call.data.unwrap_or_default(),
                call.value.unwrap_or_default(),
            )
            .map_err(|err| {
                dbg!(&err);
                EvmError(err)
            })?;

        let formatted_trace = if call.format_trace {
            self.format_traces(&res.traces)
        } else {
            None
        };

        Ok(CallRawResult {
            gas_used: res.gas_used,
            block_number: res.env.evm_env.block_env.number.to(),
            success: !res.reverted,
            trace: res.traces,
            logs: res.logs,
            exit_reason: res.exit_reason.unwrap_or(InstructionResult::Stop),
            return_data: res.result,
            formatted_trace,
        })
    }

    fn format_traces(&mut self, traces: &Option<SparsedTraceArena>) -> Option<String> {
        if let Some(trace) = traces {
            let mut trace_writer = TraceWriter::new(Vec::<u8>::new());
            // Get a reference to the CallTraceArena inside SparsedTraceArena
            let arena: &CallTraceArena = trace;
            if let Some(identifier) = &mut self.etherscan_identifier {
                // Clone to get a mutable version for identify
                let arena_clone = arena.clone();
                self.decoder.identify(&arena_clone, identifier);
            }
            trace_writer
                .write_arena(arena)
                .expect("trace writer failure");
            Some(
                String::from_utf8(trace_writer.into_writer())
                    .expect("trace writer wrote invalid UTF-8"),
            )
        } else {
            None
        }
    }

    pub fn override_account(
        &mut self,
        address: Address,
        balance: Option<U256>,
        nonce: Option<u64>,
        code: Option<Bytes>,
        storage: Option<StorageOverride>,
    ) -> Result<(), OverrideError> {
        let mut account = Account {
            info: self
                .executor
                .backend()
                .basic_ref(address)
                .map_err(|_| OverrideError)?
                .unwrap_or_default(),
            ..Account::new_not_existing(0)
        };

        if let Some(balance) = balance {
            account.info.balance = balance;
        }
        if let Some(nonce) = nonce {
            account.info.nonce = nonce;
        }
        if let Some(code) = code {
            account.info.code = Some(Bytecode::new_raw(code.to_vec().into()));
        }
        if let Some(storage) = storage {
            // If we do a "full storage override", mark as Created so the EVM
            // doesn't fetch storage from the forked node for missing slots.
            if !storage.diff {
                account.storage.clear();
                account.status = AccountStatus::Created | AccountStatus::Touched;
            } else {
                // For diff overrides, only mark as touched
                account.status = AccountStatus::Touched;
            }
            account.storage.extend(
                storage
                    .slots
                    .into_iter()
                    .map(|(key, value)| (key, EvmStorageSlot::new(value, 0))),
            );
        } else {
            // Mark account as touched so commit() doesn't skip it
            account.mark_touch();
        }

        self.executor
            .backend_mut()
            .commit([(address, account)].into_iter().collect());

        Ok(())
    }

    pub async fn call_raw_committing(
        &mut self,
        call: CallRawRequest,
        gas_limit: U256,
    ) -> Result<CallRawResult, EvmError> {
        let gas_limit_u64: u64 = gas_limit.try_into().unwrap_or(u64::MAX);
        self.executor.set_gas_limit(gas_limit_u64);
        self.set_access_list(call.access_list)?;

        // Use transact_raw for committing calls
        let res = self
            .executor
            .transact_raw(
                call.from,
                call.to,
                call.data.unwrap_or_default(),
                call.value.unwrap_or_default(),
            )
            .map_err(|err| {
                dbg!(&err);
                EvmError(err)
            })?;

        let formatted_trace = if call.format_trace {
            self.format_traces(&res.traces)
        } else {
            None
        };

        Ok(CallRawResult {
            gas_used: res.gas_used,
            block_number: res.env.evm_env.block_env.number.to(),
            success: !res.reverted,
            trace: res.traces,
            logs: res.logs,
            exit_reason: res.exit_reason.unwrap_or(InstructionResult::Stop),
            return_data: res.result,
            formatted_trace,
        })
    }

    pub async fn set_block(&mut self, number: U256) -> Result<(), EvmError> {
        self.executor.env_mut().evm_env.block_env.number = number;
        Ok(())
    }

    pub fn get_block(&self) -> U256 {
        self.executor.env().evm_env.block_env.number
    }

    pub async fn set_block_timestamp(&mut self, timestamp: U256) -> Result<(), EvmError> {
        self.executor.env_mut().evm_env.block_env.timestamp = timestamp;
        Ok(())
    }

    pub fn get_block_timestamp(&self) -> U256 {
        self.executor.env().evm_env.block_env.timestamp
    }

    pub fn get_chain_id(&self) -> u64 {
        self.executor.env().evm_env.cfg_env.chain_id
    }

    fn set_access_list(&mut self, access_list: Option<AccessList>) -> Result<(), EvmError> {
        if let Some(access_list) = access_list {
            // Convert to alloy AccessList format
            let alloy_access_list: Vec<alloy_eips::eip2930::AccessListItem> = access_list
                .into_iter()
                .map(|(address, storage_keys): (Address, Vec<U256>)| {
                    alloy_eips::eip2930::AccessListItem {
                        address,
                        storage_keys: storage_keys
                            .into_iter()
                            .map(|k| alloy_primitives::B256::from(k))
                            .collect(),
                    }
                })
                .collect();
            self.executor.env_mut().tx.access_list = alloy_access_list.into();
        }

        Ok(())
    }
}
