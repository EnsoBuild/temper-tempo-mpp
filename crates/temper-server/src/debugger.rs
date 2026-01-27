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

/// Block explorer APIs by chain ID
fn get_explorer_api(chain_id: u64) -> Option<&'static str> {
    match chain_id {
        1 => Some("https://api.etherscan.io/api"),
        137 => Some("https://api.polygonscan.com/api"),
        56 => Some("https://api.bscscan.com/api"),
        42161 => Some("https://api.arbiscan.io/api"),
        10 => Some("https://api-optimistic.etherscan.io/api"),
        8453 => Some("https://api.basescan.org/api"),
        999 => Some("https://www.hyperscan.com/api"),
        _ => None,
    }
}

/// Fetch contract source from Sourcify or block explorer
pub async fn fetch_contract_source(
    address: &str,
    chain_id: u64,
    etherscan_key: Option<&str>,
) -> Result<Option<ContractSource>, String> {
    let client = http_client();

    // Try block explorer first for chains like HyperEVM that may not be on Sourcify
    if let Some(api_url) = get_explorer_api(chain_id) {
        let mut url = format!(
            "{}?module=contract&action=getsourcecode&address={}",
            api_url, address
        );

        // Add API key if available (required for Etherscan V2)
        if let Some(key) = etherscan_key {
            url.push_str("&apikey=");
            url.push_str(key);
        }

        if let Ok(response) = client
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
        {
            if response.status().is_success() {
                if let Ok(data) = response.json::<serde_json::Value>().await {
                    if data.get("status").and_then(|s| s.as_str()) == Some("1") {
                        if let Some(result) = data.get("result").and_then(|r| r.as_array()).and_then(|a| a.first()) {
                            let contract_name = result.get("ContractName")
                                .and_then(|n| n.as_str())
                                .unwrap_or("")
                                .to_string();

                            let source_code = result.get("SourceCode")
                                .and_then(|s| s.as_str())
                                .unwrap_or("");

                            if !source_code.is_empty() && source_code != "Contract source code not verified" {
                                let abi_str = result.get("ABI")
                                    .and_then(|a| a.as_str())
                                    .unwrap_or("[]");

                                let abi: serde_json::Value = serde_json::from_str(abi_str)
                                    .unwrap_or(serde_json::Value::Array(vec![]));

                                let mut sources = HashMap::new();
                                let file_name = format!("{}.sol", contract_name);
                                sources.insert(file_name.clone(), source_code.to_string());

                                return Ok(Some(ContractSource {
                                    name: contract_name,
                                    sources,
                                    source_list: vec![file_name],
                                    abi,
                                    source_map: String::new(),
                                    source_locations: vec![],
                                }));
                            }
                        }
                    }
                }
            }
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
