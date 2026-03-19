//! Debug trace fetcher - gets step-by-step execution trace for debugging

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

/// Shared HTTP client for connection reuse
fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .pool_max_idle_per_host(10)
            .build()
            .expect("Failed to create HTTP client")
    })
}

/// A single execution step from structLogger
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugStep {
    pub pc: u64,
    pub op: String,
    pub gas: u64,
    pub gas_cost: u64,
    pub depth: u64,
    #[serde(default)]
    pub stack: Vec<String>,
    #[serde(default)]
    pub memory: Vec<String>,
    #[serde(default)]
    pub storage: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// A call frame from callTracer - contains the call tree with input/output data
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallFrame {
    #[serde(rename = "type")]
    pub call_type: String,
    pub from: String,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub gas: Option<String>,
    #[serde(default)]
    pub gas_used: Option<String>,
    #[serde(default)]
    pub input: Option<String>,
    #[serde(default)]
    pub output: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub revert_reason: Option<String>,
    #[serde(default)]
    pub calls: Vec<CallFrame>,
}

/// Full debug trace for a transaction
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugTrace {
    #[serde(default)]
    pub gas: u64,
    #[serde(default)]
    pub failed: bool,
    #[serde(default)]
    pub return_value: String,
    #[serde(default)]
    pub struct_logs: Vec<DebugStep>,
    /// Transaction input data (added separately)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_input: Option<String>,
    /// Transaction target address
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_to: Option<String>,
    /// Transaction sender
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tx_from: Option<String>,
    /// Call tree from callTracer (contains input data for all calls)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_tree: Option<CallFrame>,
}

/// Contract source info from Sourcify
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractSource {
    pub name: String,
    /// Map of filename -> source code
    pub sources: HashMap<String, String>,
    /// List of source file names in order (for file index mapping)
    pub source_list: Vec<String>,
    pub abi: serde_json::Value,
    /// Deployed bytecode source map
    #[serde(default)]
    pub source_map: String,
    /// Parsed source locations indexed by bytecode offset
    #[serde(default)]
    pub source_locations: Vec<SourceLocation>,
}

/// Debug session combining trace + source
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugSession {
    pub tx_hash: String,
    pub chain_id: u64,
    pub trace: DebugTrace,
    pub contracts: HashMap<String, ContractSource>,
    /// Maps PC to source location for each contract
    pub source_maps: HashMap<String, Vec<SourceLocation>>,
}

/// Source code location
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceLocation {
    pub start: u32,
    pub length: u32,
    pub file_index: i32,
    pub jump: char,
    pub modifier_depth: u32,
}

/// Call parameters for debug_traceCall
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DebugCallParams {
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas_price: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

/// Fetch debug trace for an existing transaction
/// Tries callTracer first (widely supported), falls back to structLogger if needed
pub async fn fetch_debug_trace(
    rpc_url: &str,
    tx_hash: &str,
) -> Result<DebugTrace, String> {
    let client = http_client();

    // Prepare requests
    let tx_request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_getTransactionByHash",
        "params": [tx_hash],
        "id": 1
    });

    let call_tracer_request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "debug_traceTransaction",
        "params": [tx_hash, {
            "tracer": "callTracer",
            "tracerConfig": {
                "withLog": true
            }
        }],
        "id": 2
    });

    // Execute tx info and callTracer in parallel (callTracer is more widely supported)
    let (tx_result, call_tracer_result) = tokio::join!(
        client.post(rpc_url).json(&tx_request).send(),
        client.post(rpc_url).json(&call_tracer_request).send()
    );

    // Parse call tracer response (primary source)
    let call_response = call_tracer_result.map_err(|e| format!("RPC request failed: {}", e))?;
    let call_json: serde_json::Value = call_response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if let Some(error) = call_json.get("error") {
        return Err(format!("RPC error: {}", error));
    }

    let call_result = call_json.get("result").ok_or("No result in response")?;
    let call_tree: CallFrame = serde_json::from_value(call_result.clone())
        .map_err(|e| format!("Failed to parse call trace: {}", e))?;

    // Build trace from call tree
    let mut trace = DebugTrace {
        gas: call_tree.gas.as_ref()
            .and_then(|g| u64::from_str_radix(g.trim_start_matches("0x"), 16).ok())
            .unwrap_or(0),
        failed: call_tree.error.is_some(),
        return_value: call_tree.output.clone().unwrap_or_default(),
        struct_logs: vec![], // Not available with callTracer
        tx_input: call_tree.input.clone(),
        tx_to: call_tree.to.clone(),
        tx_from: Some(call_tree.from.clone()),
        call_tree: Some(call_tree),
    };

    // Parse transaction details for additional info
    if let Ok(tx_response) = tx_result {
        if let Ok(tx_json) = tx_response.json::<serde_json::Value>().await {
            if let Some(tx_result) = tx_json.get("result") {
                if trace.tx_input.is_none() {
                    trace.tx_input = tx_result
                        .get("input")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
                if trace.tx_to.is_none() {
                    trace.tx_to = tx_result
                        .get("to")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
                if trace.tx_from.is_none() {
                    trace.tx_from = tx_result
                        .get("from")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                }
            }
        }
    }

    Ok(trace)
}

/// Fetch debug trace for a simulated call using debug_traceCall
pub async fn fetch_debug_trace_call(
    rpc_url: &str,
    call_params: DebugCallParams,
    block_number: Option<u64>,
) -> Result<DebugTrace, String> {
    let client = http_client();

    let block = block_number
        .map(|b| format!("0x{:x}", b))
        .unwrap_or_else(|| "latest".to_string());

    let request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "debug_traceCall",
        "params": [call_params, block, {
            "enableMemory": true,
            "enableStack": true,
            "enableStorage": true
        }],
        "id": 1
    });

    let response = client
        .post(rpc_url)
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("RPC request failed: {}", e))?;

    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {}", e))?;

    if let Some(error) = json.get("error") {
        return Err(format!("RPC error: {}", error));
    }

    let result = json.get("result").ok_or("No result in response")?;

    let trace: DebugTrace = serde_json::from_value(result.clone())
        .map_err(|e| format!("Failed to parse trace: {}", e))?;

    Ok(trace)
}

/// Hardcoded ContractSource for TIP-20 native token precompiles (0x20c0... addresses).
/// These are Tempo-native precompiles — no bytecode is deployed, so Sourcify will
/// never find them. We return the standard TIP-20 / ERC-20 interface directly.
fn tip20_contract_source(address: &str) -> ContractSource {
    let source = r#"// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

/// @title TIP-20 Native Token Precompile
/// @notice Tempo-native token at address with 0x20c0 prefix.
///         Storage layout: slot 8 = totalSupply, slot 9 = balances, slot 10 = allowances.
///         All TIP-20 tokens have 6 decimals.
interface ITIP20 {
    function name() external view returns (string memory);
    function symbol() external view returns (string memory);
    function decimals() external view returns (uint8);       // always 6
    function totalSupply() external view returns (uint256);
    function balanceOf(address account) external view returns (uint256);
    function transfer(address to, uint256 amount) external returns (bool);
    function allowance(address owner, address spender) external view returns (uint256);
    function approve(address spender, uint256 amount) external returns (bool);
    function transferFrom(address from, address to, uint256 amount) external returns (bool);

    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);
}
"#;
    let file_name = "TIP20.sol".to_string();
    let mut sources = HashMap::new();
    sources.insert(file_name.clone(), source.to_string());

    let abi = serde_json::json!([
        {"type":"function","name":"name","inputs":[],"outputs":[{"type":"string"}],"stateMutability":"view"},
        {"type":"function","name":"symbol","inputs":[],"outputs":[{"type":"string"}],"stateMutability":"view"},
        {"type":"function","name":"decimals","inputs":[],"outputs":[{"type":"uint8"}],"stateMutability":"view"},
        {"type":"function","name":"totalSupply","inputs":[],"outputs":[{"type":"uint256"}],"stateMutability":"view"},
        {"type":"function","name":"balanceOf","inputs":[{"name":"account","type":"address"}],"outputs":[{"type":"uint256"}],"stateMutability":"view"},
        {"type":"function","name":"transfer","inputs":[{"name":"to","type":"address"},{"name":"amount","type":"uint256"}],"outputs":[{"type":"bool"}],"stateMutability":"nonpayable"},
        {"type":"function","name":"allowance","inputs":[{"name":"owner","type":"address"},{"name":"spender","type":"address"}],"outputs":[{"type":"uint256"}],"stateMutability":"view"},
        {"type":"function","name":"approve","inputs":[{"name":"spender","type":"address"},{"name":"amount","type":"uint256"}],"outputs":[{"type":"bool"}],"stateMutability":"nonpayable"},
        {"type":"function","name":"transferFrom","inputs":[{"name":"from","type":"address"},{"name":"to","type":"address"},{"name":"amount","type":"uint256"}],"outputs":[{"type":"bool"}],"stateMutability":"nonpayable"},
        {"type":"event","name":"Transfer","inputs":[{"name":"from","type":"address","indexed":true},{"name":"to","type":"address","indexed":true},{"name":"value","type":"uint256","indexed":false}]},
        {"type":"event","name":"Approval","inputs":[{"name":"owner","type":"address","indexed":true},{"name":"spender","type":"address","indexed":true},{"name":"value","type":"uint256","indexed":false}]}
    ]);

    ContractSource {
        name: format!("TIP-20 Native Token ({}...{})", &address[..6], &address[address.len()-4..]),
        sources,
        source_list: vec![file_name],
        abi,
        source_map: String::new(),
        source_locations: vec![],
    }
}

/// Hardcoded source for the Tempo protocol batch dispatcher (zero address).
/// In type 0x76 transactions the root call has from=0x0 and to=0x0 —
/// this is the protocol-level batch dispatch, not a deployed contract.
fn tempo_batch_dispatcher_source() -> ContractSource {
    let source = r#"// Tempo Protocol — Batch Transaction Dispatcher
//
// The zero address (0x0000...0000) is the entry point for Tempo's
// type 0x76 batch transactions. It is not a deployed contract —
// it is handled natively by the Tempo execution layer.
//
// A type 0x76 transaction contains:
//   - calls[]     : array of { to, value, input } sub-calls
//   - feeToken    : TIP-20 token used to pay gas fees
//   - feePayer    : address sponsoring the transaction fees
//
// Each sub-call is executed sequentially and commits state independently.
"#;
    let file_name = "TempoBatchDispatcher.txt".to_string();
    let mut sources = HashMap::new();
    sources.insert(file_name.clone(), source.to_string());

    ContractSource {
        name: "Tempo Batch Dispatcher (0x76)".to_string(),
        sources,
        source_list: vec![file_name],
        abi: serde_json::Value::Array(vec![]),
        source_map: String::new(),
        source_locations: vec![],
    }
}

/// Try Sourcify v2 API (used by contracts.tempo.xyz and sourcify.dev/server)
async fn try_sourcify_v2(client: &reqwest::Client, base_url: &str, chain_id: u64, address: &str) -> Option<ContractSource> {
    let url = format!("{}/v2/contract/{}/{}?fields=all", base_url, chain_id, address);

    let response = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    let data: serde_json::Value = response.json().await.ok()?;

    let contract_name = data.get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();

    let abi = data.get("abi").cloned().unwrap_or(serde_json::Value::Array(vec![]));

    // Sources are a map of { "filename": { "content": "..." } }
    let mut sources: HashMap<String, String> = HashMap::new();
    let mut source_list: Vec<String> = Vec::new();

    if let Some(srcs) = data.get("sources").and_then(|s| s.as_object()) {
        for (filename, file_obj) in srcs {
            if let Some(content) = file_obj.get("content").and_then(|c| c.as_str()) {
                sources.insert(filename.clone(), content.to_string());
                source_list.push(filename.clone());
            }
        }
    }

    if sources.is_empty() {
        return None;
    }

    Some(ContractSource {
        name: contract_name,
        sources,
        source_list,
        abi,
        source_map: String::new(),
        source_locations: vec![],
    })
}

/// Check if chain is supported by Etherscan V2 API
fn is_etherscan_supported(chain_id: u64) -> bool {
    matches!(chain_id, 1 | 137 | 56 | 42161 | 10 | 8453 | 11155111 | 17000)
}

/// Get legacy block explorer API for chains not on Etherscan V2
fn get_legacy_explorer_api(chain_id: u64) -> Option<&'static str> {
    match chain_id {
        999 => Some("https://www.hyperscan.com/api"),
        _ => None,
    }
}

/// Try to fetch contract source from an Etherscan-compatible API
async fn try_etherscan_api(client: &reqwest::Client, url: &str) -> Option<ContractSource> {
    let response = client
        .get(url)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;

    if !response.status().is_success() {
        return None;
    }

    let data: serde_json::Value = response.json().await.ok()?;

    if data.get("status").and_then(|s| s.as_str()) != Some("1") {
        return None;
    }

    let result = data.get("result")?.as_array()?.first()?;

    let contract_name = result.get("ContractName")
        .and_then(|n| n.as_str())
        .unwrap_or("")
        .to_string();

    let source_code = result.get("SourceCode")
        .and_then(|s| s.as_str())
        .unwrap_or("");

    if source_code.is_empty() || source_code == "Contract source code not verified" {
        return None;
    }

    let abi_str = result.get("ABI")
        .and_then(|a| a.as_str())
        .unwrap_or("[]");

    let abi: serde_json::Value = serde_json::from_str(abi_str)
        .unwrap_or(serde_json::Value::Array(vec![]));

    let mut sources = HashMap::new();
    let file_name = format!("{}.sol", contract_name);
    sources.insert(file_name.clone(), source_code.to_string());

    Some(ContractSource {
        name: contract_name,
        sources,
        source_list: vec![file_name],
        abi,
        source_map: String::new(),
        source_locations: vec![],
    })
}

/// Fetch contract source from Sourcify or block explorer
pub async fn fetch_contract_source(
    address: &str,
    chain_id: u64,
    etherscan_key: Option<&str>,
) -> Result<Option<ContractSource>, String> {
    let client = http_client();

    let addr_lower = address.to_lowercase();

    // TIP-20 native precompiles (0x20c0... prefix) — return hardcoded interface directly
    if addr_lower.starts_with("0x20c0") {
        return Ok(Some(tip20_contract_source(address)));
    }

    // Zero address on Tempo = protocol batch dispatcher (type 0x76 root call)
    if matches!(chain_id, 4217 | 42429 | 42431)
        && addr_lower.trim_start_matches("0x").chars().all(|c| c == '0')
    {
        return Ok(Some(tempo_batch_dispatcher_source()));
    }

    // For Tempo chains, try contracts.tempo.xyz (Sourcify v2) first
    if matches!(chain_id, 4217 | 42429 | 42431) {
        if let Some(source) = try_sourcify_v2(client, "https://contracts.tempo.xyz", chain_id, address).await {
            return Ok(Some(source));
        }
    }

    // Try Etherscan V2 API first (requires API key)
    if let Some(key) = etherscan_key {
        if is_etherscan_supported(chain_id) {
            let url = format!(
                "https://api.etherscan.io/v2/api?chainid={}&module=contract&action=getsourcecode&address={}&apikey={}",
                chain_id, address, key
            );

            if let Some(source) = try_etherscan_api(&client, &url).await {
                return Ok(Some(source));
            }
        }
    }

    // Try legacy explorer API for chains not on Etherscan V2
    if let Some(api_url) = get_legacy_explorer_api(chain_id) {
        let mut url = format!(
            "{}?module=contract&action=getsourcecode&address={}",
            api_url, address
        );

        if let Some(key) = etherscan_key {
            url.push_str("&apikey=");
            url.push_str(key);
        }

        if let Some(source) = try_etherscan_api(&client, &url).await {
            return Ok(Some(source));
        }
    }

    // Fall back to Sourcify
    // Try full match first, then partial
    for match_type in &["full_match", "partial_match", "any"] {
        let url = if *match_type == "any" {
            format!("https://sourcify.dev/server/files/any/{}/{}", chain_id, address)
        } else {
            format!("https://sourcify.dev/server/repository/contracts/{}/{}/{}", match_type, chain_id, address)
        };

        let response = client
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await;

        let response = match response {
            Ok(r) if r.status().is_success() => r,
            _ => continue,
        };

        let data: serde_json::Value = match response.json().await {
            Ok(d) => d,
            Err(_) => continue,
        };

        // Parse the Sourcify response
        let files = data.get("files").and_then(|f| f.as_array())
            .or_else(|| data.as_array());

        if let Some(files) = files {
            let mut contract_name = String::new();
            let mut sources: HashMap<String, String> = HashMap::new();
            let mut source_list: Vec<String> = Vec::new();
            let mut abi = serde_json::Value::Null;
            let source_map = String::new();

            // First pass: get metadata for source map and contract info
            for file in files {
                let name = file.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let content = file.get("content").and_then(|c| c.as_str()).unwrap_or("");

                if name == "metadata.json" {
                    if let Ok(metadata) = serde_json::from_str::<serde_json::Value>(content) {
                        // Get contract name and main source file
                        if let Some(target) = metadata.get("settings")
                            .and_then(|s| s.get("compilationTarget"))
                            .and_then(|t| t.as_object())
                        {
                            if let Some((file_path, name_val)) = target.iter().next() {
                                contract_name = name_val.as_str().unwrap_or("").to_string();
                                // The compilation target file should be first
                                if !source_list.contains(&file_path.to_string()) {
                                    source_list.insert(0, file_path.clone());
                                }
                            }
                        }

                        // Get ABI
                        if let Some(a) = metadata.get("output").and_then(|o| o.get("abi")) {
                            abi = a.clone();
                        }

                        // Get source list from metadata
                        if let Some(srcs) = metadata.get("sources").and_then(|s| s.as_object()) {
                            for src_name in srcs.keys() {
                                if !source_list.contains(src_name) {
                                    source_list.push(src_name.clone());
                                }
                            }
                        }
                    }
                }
            }

            // Second pass: collect all source files
            for file in files {
                let name = file.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let content = file.get("content").and_then(|c| c.as_str()).unwrap_or("");

                if name.ends_with(".sol") {
                    // Try to match with source list entries
                    let matched_name = source_list.iter()
                        .find(|s| s.ends_with(name) || name.ends_with(s.as_str()))
                        .cloned()
                        .unwrap_or_else(|| name.to_string());

                    sources.insert(matched_name, content.to_string());
                }
            }

            if !sources.is_empty() {
                // Now fetch the compiled output for source map
                // Try to get it from the compiler output
                let source_locations = if !source_map.is_empty() {
                    parse_source_map(&source_map)
                } else {
                    Vec::new()
                };

                return Ok(Some(ContractSource {
                    name: contract_name,
                    sources,
                    source_list,
                    abi,
                    source_map,
                    source_locations,
                }));
            }
        }
    }

    Ok(None)
}

/// Parse Solidity source map string into locations
pub fn parse_source_map(source_map: &str) -> Vec<SourceLocation> {
    let mut locations = Vec::new();
    let mut prev = SourceLocation {
        start: 0,
        length: 0,
        file_index: -1,
        jump: '-',
        modifier_depth: 0,
    };

    for entry in source_map.split(';') {
        let parts: Vec<&str> = entry.split(':').collect();

        let start = parts.get(0)
            .and_then(|s| if s.is_empty() { None } else { s.parse().ok() })
            .unwrap_or(prev.start);

        let length = parts.get(1)
            .and_then(|s| if s.is_empty() { None } else { s.parse().ok() })
            .unwrap_or(prev.length);

        let file_index = parts.get(2)
            .and_then(|s| if s.is_empty() { None } else { s.parse().ok() })
            .unwrap_or(prev.file_index);

        let jump = parts.get(3)
            .and_then(|s| s.chars().next())
            .unwrap_or(prev.jump);

        let modifier_depth = parts.get(4)
            .and_then(|s| if s.is_empty() { None } else { s.parse().ok() })
            .unwrap_or(prev.modifier_depth);

        let loc = SourceLocation {
            start,
            length,
            file_index,
            jump,
            modifier_depth,
        };

        prev = loc.clone();
        locations.push(loc);
    }

    locations
}
