//! Fetch transaction traces from RPC using debug_traceTransaction

use serde::{Deserialize, Serialize};
use temper_core::simulation::EnhancedCallTrace;

/// RPC trace response from debug_traceTransaction with callTracer
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcCallTrace {
    #[serde(rename = "type")]
    pub call_type: String,
    pub from: String,
    pub to: Option<String>,
    pub value: Option<String>,
    pub gas: Option<String>,
    pub gas_used: Option<String>,
    pub input: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
    pub revert_reason: Option<String>,
    #[serde(default)]
    pub calls: Vec<RpcCallTrace>,
}

/// Transaction receipt for gas and success info
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionReceipt {
    pub status: Option<String>,
    pub gas_used: Option<String>,
    pub block_number: Option<String>,
}

/// Fetch the trace of a historical transaction using debug_traceTransaction
pub async fn fetch_transaction_trace(
    rpc_url: &str,
    tx_hash: &str,
) -> Result<(EnhancedCallTrace, TransactionReceipt), String> {
    let client = reqwest::Client::new();

    // Fetch trace using debug_traceTransaction
    let trace_request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "debug_traceTransaction",
        "params": [tx_hash, {"tracer": "callTracer"}],
        "id": 1
    });

    let trace_response = client
        .post(rpc_url)
        .json(&trace_request)
        .send()
        .await
        .map_err(|e| format!("RPC request failed: {}", e))?;

    let trace_json: serde_json::Value = trace_response
        .json()
        .await
        .map_err(|e| format!("Failed to parse trace response: {}", e))?;

    if let Some(error) = trace_json.get("error") {
        return Err(format!("RPC error: {}", error));
    }

    let rpc_trace: RpcCallTrace = serde_json::from_value(
        trace_json.get("result").cloned().ok_or("No result in trace response")?,
    )
    .map_err(|e| format!("Failed to parse trace: {}", e))?;

    // Fetch receipt for additional info
    let receipt_request = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_getTransactionReceipt",
        "params": [tx_hash],
        "id": 2
    });

    let receipt_response = client
        .post(rpc_url)
        .json(&receipt_request)
        .send()
        .await
        .map_err(|e| format!("Receipt request failed: {}", e))?;

    let receipt_json: serde_json::Value = receipt_response
        .json()
        .await
        .map_err(|e| format!("Failed to parse receipt response: {}", e))?;

    let receipt: TransactionReceipt = serde_json::from_value(
        receipt_json.get("result").cloned().ok_or("No result in receipt response")?,
    )
    .map_err(|e| format!("Failed to parse receipt: {}", e))?;

    // Convert RPC trace to our EnhancedCallTrace format
    let enhanced_trace = convert_rpc_trace(rpc_trace);

    Ok((enhanced_trace, receipt))
}

/// Convert RPC call trace to our EnhancedCallTrace format
fn convert_rpc_trace(rpc: RpcCallTrace) -> EnhancedCallTrace {
    let calls = rpc.calls.into_iter().map(convert_rpc_trace).collect();

    EnhancedCallTrace {
        call_type: rpc.call_type.to_uppercase(),
        from: rpc.from.parse().unwrap_or_default(),
        to: rpc.to.and_then(|s| s.parse().ok()).unwrap_or_default(),
        value: rpc.value.unwrap_or_else(|| "0x0".to_string()),
        input: rpc.input.unwrap_or_else(|| "0x".to_string()),
        output: rpc.output.unwrap_or_else(|| "0x".to_string()),
        gas_used: rpc.gas_used.unwrap_or_else(|| "0x0".to_string()),
        error: rpc.error,
        revert_reason: rpc.revert_reason,
        calls,
    }
}
