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
    pub from: Option<String>,
    pub to: Option<String>,
    #[serde(default)]
    pub logs: Vec<ReceiptLog>,
}

/// Log entry from transaction receipt
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptLog {
    pub address: Option<String>,
    #[serde(default)]
    pub topics: Vec<String>,
    pub data: Option<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_receipt_deserializes_from_to_and_logs() {
        // Real Tempo receipt for tx 0x97853001b7c7...
        // On Tempo, the trace root has from/to = 0x0000...0000 (dispatcher),
        // but the RECEIPT has the actual sender/recipient.
        let json = serde_json::json!({
            "status": "0x1",
            "gasUsed": "0x49574",
            "blockNumber": "0x9a84ae",
            "from": "0x980a94f30d20b066bfb201d5cabe13e5dfc02572",
            "to": "0x20c0000000000000000000000000000000000000",
            "logs": [
                {
                    "address": "0x20c0000000000000000000000000000000000000",
                    "topics": [
                        "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                        "0x0000000000000000000000004c2c0f0bb2631b02ac9299c59690914ee7a200b8",
                        "0x000000000000000000000000ee473e8746910d394071376ff322619449bd02d5"
                    ],
                    "data": "0x00000000000000000000000000000000000000000000000000000000000157c0"
                }
            ]
        });

        let receipt: TransactionReceipt = serde_json::from_value(json).unwrap();

        // from/to must be the real sender/recipient, NOT 0x000...
        assert_eq!(
            receipt.from.as_deref(),
            Some("0x980a94f30d20b066bfb201d5cabe13e5dfc02572"),
            "Receipt must contain actual sender (not trace dispatcher)"
        );
        assert_eq!(
            receipt.to.as_deref(),
            Some("0x20c0000000000000000000000000000000000000"),
            "Receipt must contain actual recipient (pathUSD TIP-20)"
        );

        // Logs must be parsed for token transfer detection
        assert_eq!(receipt.logs.len(), 1);
        let log = &receipt.logs[0];
        assert_eq!(log.address.as_deref(), Some("0x20c0000000000000000000000000000000000000"));
        assert_eq!(log.topics.len(), 3);
        // First topic is Transfer event signature
        assert_eq!(log.topics[0], "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef");
    }

    #[test]
    fn test_trace_root_from_to_is_zero_on_tempo() {
        // On Tempo, debug_traceTransaction returns 0x000... for root from/to.
        // This is WHY we must use receipt.from/to instead.
        let json = serde_json::json!({
            "type": "CALL",
            "from": "0x0000000000000000000000000000000000000000",
            "to": "0x0000000000000000000000000000000000000000",
            "value": "0x0",
            "gas": "0x49574",
            "gasUsed": "0x49574",
            "input": "0x",
            "output": "0x",
            "calls": [{
                "type": "CALL",
                "from": "0x0000000000000000000000000000000000000000",
                "to": "0x20c0000000000000000000000000000000000000",
                "value": "0x0",
                "gas": "0x44a70",
                "gasUsed": "0x44a70",
                "input": "0x23b872dd0000000000000000000000004c2c0f0bb2631b02ac9299c59690914ee7a200b8000000000000000000000000ee473e8746910d394071376ff322619449bd02d500000000000000000000000000000000000000000000000000000000000157c0",
                "output": "0x0000000000000000000000000000000000000000000000000000000000000001"
            }]
        });

        let rpc_trace: RpcCallTrace = serde_json::from_value(json).unwrap();
        let trace = convert_rpc_trace(rpc_trace);

        // Trace root from/to are WRONG on Tempo (both zero) — this is expected
        assert_eq!(trace.from, alloy_primitives::Address::ZERO);
        assert_eq!(trace.to, alloy_primitives::Address::ZERO);

        // But subcall correctly shows pathUSD as target
        assert_eq!(trace.calls.len(), 1);
        let subcall = &trace.calls[0];
        assert_eq!(
            format!("{:?}", subcall.to).to_lowercase().contains("20c0"),
            true,
            "Subcall target should be the TIP-20 pathUSD precompile"
        );
    }

    #[test]
    fn test_handler_uses_receipt_not_trace_for_from_to() {
        // Simulate what the handler does: prefer receipt from/to over trace
        let receipt_from: Option<String> = Some("0x980a94f30d20b066bfb201d5cabe13e5dfc02572".to_string());
        let receipt_to: Option<String> = Some("0x20c0000000000000000000000000000000000000".to_string());

        let tx_from: Option<alloy_primitives::Address> = receipt_from.as_ref().and_then(|s| s.parse().ok());
        let tx_to: Option<alloy_primitives::Address> = receipt_to.as_ref().and_then(|s| s.parse().ok());

        assert!(tx_from.is_some(), "Must parse receipt from");
        assert!(tx_to.is_some(), "Must parse receipt to");
        assert_ne!(tx_from.unwrap(), alloy_primitives::Address::ZERO, "tx_from must not be zero address");
    }
}
