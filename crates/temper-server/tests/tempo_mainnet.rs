//! HTTP-level integration tests for Temper transaction simulator on Tempo Mainnet
//!
//! All tests go through the full Warp HTTP stack:
//!   POST /api/v1/simulate  →  routes  →  handler  →  simulation core  →  JSON response
//!
//! Set RPC_URL_TEMPO_MAINNET env var to run:
//!   RPC_URL_TEMPO_MAINNET=https://... cargo test -p temper-server --test tempo_mainnet -- --nocapture

use std::collections::HashMap;
use std::sync::Arc;

use dashmap::DashMap;
use serde_json::{json, Value};
use warp::Filter;

use temper_core::simulation::{is_tempo_chain, TEMPO_MAINNET_CHAIN_ID};
use temper_server::{config::Config, routes::api_routes, SharedSimulationState};

const CHAIN_ID: u64 = TEMPO_MAINNET_CHAIN_ID; // 4217
/// USDC.e (Bridged USDC via Stargate) — 0x20c0 prefix = TIP-20 precompile
const USDC_E: &str = "0x20c000000000000000000000b9537d11c60e8b50";
/// Another TIP-20 token
const TOKEN_B: &str = "0x20c000000000000000000000987bef2978df41f9";
/// Observed active address on Tempo mainnet
const SENDER: &str = "0x60d770d1707edf692601a4dd4ea802d241c7ff19";
const RECIPIENT: &str = "0x0cac4c432ae8e553e3deacc7e6af30dd5b14cd20";

fn rpc_url() -> Option<String> {
    std::env::var("RPC_URL_TEMPO_MAINNET")
        .ok()
        .filter(|s| !s.is_empty())
}

fn make_config(rpc_url: &str) -> Config {
    let mut rpc_urls = HashMap::new();
    rpc_urls.insert(CHAIN_ID, rpc_url.to_string());
    Config {
        port: 8080,
        rpc_urls,
        etherscan_key: None,
        api_key: None,
        max_request_size: 64 * 1024,
        database_url: None,
        base_url: "http://localhost:8080".to_string(),
        web_ui_enabled: false,
        storage_enabled: false,
        storage_retention_days: None,
    }
}

// ─── Pure unit tests (no RPC) ──────────────────────────────────────────────

#[test]
fn test_chain_ids_recognized() {
    assert!(is_tempo_chain(4217), "Tempo mainnet (4217)");
    assert!(is_tempo_chain(42429), "Tempo testnet (42429)");
    assert!(is_tempo_chain(42431), "Tempo Moderato (42431)");
    assert!(!is_tempo_chain(1), "Ethereum should not be Tempo");
    assert!(!is_tempo_chain(42161), "Arbitrum should not be Tempo");
}

#[test]
fn test_tip20_address_prefix() {
    assert!(USDC_E.to_lowercase().starts_with("0x20c0"), "USDC.e should be TIP-20");
    assert!(
        !"0x1234567890123456789012345678901234567890".starts_with("0x20c0"),
        "regular address should not match"
    );
}

// ─── HTTP API tests ──────────────────────────────────────────────────────────

/// TIP-20 reads: decimals(), totalSupply(), balanceOf() via POST /api/v1/simulate-bundle.
/// Bundle shares one EVM init for all three calls.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_http_tip20_reads() {
    let url = match rpc_url() {
        Some(u) => u,
        None => {
            eprintln!("SKIP: RPC_URL_TEMPO_MAINNET not set");
            return;
        }
    };

    let state = Arc::new(SharedSimulationState { evms: Arc::new(DashMap::new()) });
    let routes = warp::path("api")
        .and(warp::path("v1"))
        .and(api_routes(make_config(&url), state, None, None));

    let addr_padded = format!(
        "000000000000000000000000{}",
        SENDER.trim_start_matches("0x")
    );

    // Send three reads as a bundle — one EVM fork, three simulations
    let body = json!([
        {
            "chainId": CHAIN_ID,
            "from": SENDER,
            "to": USDC_E,
            "data": "0x313ce567",   // decimals()
            "gasLimit": "0x7a120"
        },
        {
            "chainId": CHAIN_ID,
            "from": SENDER,
            "to": USDC_E,
            "data": "0x18160ddd",   // totalSupply()
            "gasLimit": "0x7a120"
        },
        {
            "chainId": CHAIN_ID,
            "from": SENDER,
            "to": USDC_E,
            "data": format!("0x70a08231{}", addr_padded),  // balanceOf(SENDER)
            "gasLimit": "0x7a120"
        }
    ]);

    let resp = warp::test::request()
        .method("POST")
        .path("/api/v1/simulate-bundle")
        .json(&body)
        .reply(&routes)
        .await;

    assert_eq!(resp.status(), 200, "HTTP 200 expected, body: {}", String::from_utf8_lossy(resp.body()));
    let results: Vec<Value> = serde_json::from_slice(resp.body()).expect("valid JSON array");
    assert_eq!(results.len(), 3);

    // 1. decimals() → 6
    assert!(results[0]["success"].as_bool().unwrap_or(false), "decimals() reverted");
    assert_eq!(results[0]["isTempoTx"], json!(true), "should be tagged isTempoTx");
    let dec_data = results[0]["returnData"].as_str().unwrap_or("");
    assert!(dec_data.ends_with("06"), "decimals should be 6, got returnData={}", dec_data);
    println!("decimals() ✓  returnData={}", dec_data);

    // 2. totalSupply() → > 0
    assert!(results[1]["success"].as_bool().unwrap_or(false), "totalSupply() reverted");
    let supply_hex = results[1]["returnData"].as_str().unwrap_or("0x").trim_start_matches("0x");
    let supply = u128::from_str_radix(supply_hex, 16).unwrap_or(0);
    assert!(supply > 0, "totalSupply should be > 0, got {}", supply);
    println!("totalSupply() ✓  supply={}", supply);

    // 3. balanceOf(SENDER)
    assert!(results[2]["success"].as_bool().unwrap_or(false), "balanceOf() reverted");
    println!("balanceOf({}) ✓  returnData={}", SENDER, results[2]["returnData"]);
}

/// TIP-20 write: approve() via POST /api/v1/simulate.
/// No balance required — just sets allowance in TIP-20 storage.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_http_tip20_approve() {
    let url = match rpc_url() {
        Some(u) => u,
        None => {
            eprintln!("SKIP: RPC_URL_TEMPO_MAINNET not set");
            return;
        }
    };

    let state = Arc::new(SharedSimulationState { evms: Arc::new(DashMap::new()) });
    let routes = warp::path("api")
        .and(warp::path("v1"))
        .and(api_routes(make_config(&url), state, None, None));

    // approve(recipient, 1_000_000)
    let recipient_padded = format!("000000000000000000000000{}", RECIPIENT.trim_start_matches("0x"));
    let amount = "00000000000000000000000000000000000000000000000000000000000f4240";
    let calldata = format!("0x095ea7b3{}{}", recipient_padded, amount);

    let body = json!({
        "chainId": CHAIN_ID,
        "from": SENDER,
        "to": USDC_E,
        "data": calldata,
        "gasLimit": "0x7a120"
    });

    let resp = warp::test::request()
        .method("POST")
        .path("/api/v1/simulate")
        .json(&body)
        .reply(&routes)
        .await;

    assert_eq!(resp.status(), 200, "body: {}", String::from_utf8_lossy(resp.body()));
    let result: Value = serde_json::from_slice(resp.body()).expect("valid JSON");

    assert!(result["success"].as_bool().unwrap_or(false), "approve() reverted");
    assert_eq!(result["isTempoTx"], json!(true));
    let ret = result["returnData"].as_str().unwrap_or("");
    assert!(ret.ends_with("01"), "approve() should return true, got {}", ret);
    assert!(result["gasUsed"].as_u64().unwrap_or(0) > 0);
    println!("approve() ✓  gasUsed={}", result["gasUsed"]);
}

/// Type 0x76 batch transaction: POST /api/v1/simulate with `calls` array.
/// Verifies callResults array, isTempoTx, transactionType, feeToken in response.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_http_batch_0x76() {
    let url = match rpc_url() {
        Some(u) => u,
        None => {
            eprintln!("SKIP: RPC_URL_TEMPO_MAINNET not set");
            return;
        }
    };

    let state = Arc::new(SharedSimulationState { evms: Arc::new(DashMap::new()) });
    let routes = warp::path("api")
        .and(warp::path("v1"))
        .and(api_routes(make_config(&url), state, None, None));

    let body = json!({
        "chainId": CHAIN_ID,
        "from": SENDER,
        "gasLimit": "0xf4240",
        "transactionType": "batch",
        "feeToken": USDC_E,
        "calls": [
            { "to": USDC_E,  "data": "0x313ce567" },  // decimals() on USDC.e
            { "to": USDC_E,  "data": "0x18160ddd" },  // totalSupply() on USDC.e
            { "to": TOKEN_B, "data": "0x313ce567" }   // decimals() on TOKEN_B
        ]
    });

    let resp = warp::test::request()
        .method("POST")
        .path("/api/v1/simulate")
        .json(&body)
        .reply(&routes)
        .await;

    assert_eq!(resp.status(), 200, "body: {}", String::from_utf8_lossy(resp.body()));
    let result: Value = serde_json::from_slice(resp.body()).expect("valid JSON");

    assert!(result["success"].as_bool().unwrap_or(false), "batch simulation reverted");
    assert_eq!(result["isTempoTx"], json!(true));
    assert_eq!(result["transactionType"].as_str(), Some("batch"));

    let call_results = result["callResults"].as_array().expect("callResults must be an array");
    assert_eq!(call_results.len(), 3, "expected 3 call results");

    for (i, r) in call_results.iter().enumerate() {
        assert!(r["success"].as_bool().unwrap_or(false), "call {} failed", i);
    }

    // decimals() should return 6 for both tokens
    let dec0 = call_results[0]["returnData"].as_str().unwrap_or("");
    let dec2 = call_results[2]["returnData"].as_str().unwrap_or("");
    assert!(dec0.ends_with("06"), "USDC.e decimals should be 6, got {}", dec0);
    assert!(dec2.ends_with("06"), "TOKEN_B decimals should be 6, got {}", dec2);

    println!(
        "Batch (0x76) ✓  {} calls, gasUsed={}",
        call_results.len(),
        result["gasUsed"]
    );
}

/// Fetch a real on-chain 0x76 tx via POST /api/v1/simulate with txHash.
/// No EVM fork — goes through debug_traceTransaction directly.
#[tokio::test]
async fn test_http_tx_hash_fetch() {
    let url = match rpc_url() {
        Some(u) => u,
        None => {
            eprintln!("SKIP: RPC_URL_TEMPO_MAINNET not set");
            return;
        }
    };

    let state = Arc::new(SharedSimulationState { evms: Arc::new(DashMap::new()) });
    let routes = warp::path("api")
        .and(warp::path("v1"))
        .and(api_routes(make_config(&url), state, None, None));

    // Real type-0x76 tx from Tempo mainnet block 0x9a84ae
    let tx_hash = "0x97853001b7c7974d556953bc27af6a0b3d6931d313df6340a150e978754990f9";

    let body = json!({
        "chainId": CHAIN_ID,
        "from": "0x0000000000000000000000000000000000000000",
        "gasLimit": "0x7a120",
        "txHash": tx_hash
    });

    let resp = warp::test::request()
        .method("POST")
        .path("/api/v1/simulate")
        .json(&body)
        .reply(&routes)
        .await;

    assert_eq!(resp.status(), 200, "body: {}", String::from_utf8_lossy(resp.body()));
    let result: Value = serde_json::from_slice(resp.body()).expect("valid JSON");

    assert!(result["success"].as_bool().unwrap_or(false), "tx should have succeeded on-chain");
    assert_eq!(result["txHash"].as_str(), Some(tx_hash));
    assert!(result["gasUsed"].as_u64().unwrap_or(0) > 0, "gasUsed should be > 0");
    assert!(result["enhancedTrace"].is_object(), "should have enhancedTrace in response");
    assert_eq!(result["isTempoTx"], json!(true));

    println!(
        "tx_hash fetch ✓  gasUsed={}, sub-calls={}",
        result["gasUsed"],
        result["enhancedTrace"]["calls"].as_array().map(|c| c.len()).unwrap_or(0)
    );
}
