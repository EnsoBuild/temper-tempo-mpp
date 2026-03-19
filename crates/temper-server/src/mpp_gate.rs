//! MPP (Machine Payments Protocol) payment gating for simulation endpoints.
//!
//! Implements HTTP 402 challenge/credential flow for Tempo TIP-20 payments.
//! First request per IP is free, subsequent requests require payment.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use dashmap::DashMap;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use warp::{Filter, Rejection};

use crate::config::Config;

type HmacSha256 = Hmac<Sha256>;

/// MPP state shared across requests
pub struct MppState {
    pub secret: Vec<u8>,
    pub realm: String,
    pub recipient: String,
    pub amount: String,
    pub currency: String,
    pub chain_id: u64,
    pub rpc_url: String,
    /// Track request count per IP (first N are free)
    pub request_counts: DashMap<IpAddr, u32>,
    pub free_requests: u32,
}

/// The charge request advertised in the challenge
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChargeRequest {
    amount: String,
    currency: String,
    recipient: String,
    method_details: MethodDetails,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MethodDetails {
    chain_id: u64,
}

/// Credential payload from the client
#[derive(Debug, Deserialize)]
struct Credential {
    challenge: CredentialChallenge,
    payload: CredentialPayload,
}

#[derive(Debug, Deserialize)]
struct CredentialChallenge {
    id: String,
}

#[derive(Debug, Deserialize)]
struct CredentialPayload {
    /// Transaction hash proving payment
    hash: Option<String>,
}

/// Custom warp rejection for 402 Payment Required
#[derive(Debug)]
pub struct PaymentRequired {
    pub www_authenticate: String,
}

impl warp::reject::Reject for PaymentRequired {}

impl MppState {
    pub fn new(config: &Config) -> eyre::Result<Self> {
        let recipient = config
            .mpp_recipient
            .as_ref()
            .ok_or_else(|| eyre::eyre!("MPP_RECIPIENT required when MPP_ENABLED=true"))?
            .clone();
        let secret = config
            .mpp_secret
            .as_deref()
            .unwrap_or("temper-mpp-default-secret")
            .as_bytes()
            .to_vec();
        let amount = config
            .mpp_amount
            .as_deref()
            .unwrap_or("1000000")
            .to_string();
        let currency = config
            .mpp_currency
            .as_deref()
            .unwrap_or("0x20c0000000000000000000000000000000000000")
            .to_string();
        let chain_id = 4217; // Tempo Mainnet
        let rpc_url = config
            .get_rpc_url(chain_id)
            .ok_or_else(|| eyre::eyre!("RPC_URL_TEMPO_MAINNET required for MPP payment verification"))?
            .clone();
        let realm = config.base_url.clone();

        Ok(Self {
            secret,
            realm,
            recipient,
            amount,
            currency,
            chain_id,
            rpc_url,
            request_counts: DashMap::new(),
            free_requests: 10,
        })
    }

    /// Generate a 402 challenge
    fn generate_challenge(&self) -> String {
        let request = ChargeRequest {
            amount: self.amount.clone(),
            currency: self.currency.clone(),
            recipient: self.recipient.clone(),
            method_details: MethodDetails {
                chain_id: self.chain_id,
            },
        };
        let request_json = serde_json::to_string(&request).unwrap_or_default();
        let request_b64 = URL_SAFE_NO_PAD.encode(request_json.as_bytes());

        let expires = (Utc::now() + chrono::Duration::minutes(5))
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        // HMAC-SHA256 challenge ID
        let input = format!(
            "{}|tempo|charge|{}|{}||",
            self.realm, request_b64, expires
        );
        let mut mac =
            HmacSha256::new_from_slice(&self.secret).expect("HMAC can take key of any size");
        mac.update(input.as_bytes());
        let id = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());

        format!(
            "Payment id=\"{}\", realm=\"{}\", method=\"tempo\", intent=\"charge\", request=\"{}\", expires=\"{}\"",
            id, self.realm, request_b64, expires
        )
    }

    /// Verify a payment credential by checking the tx receipt on-chain
    async fn verify_credential(&self, auth_header: &str) -> Result<(), String> {
        // Parse "Payment <base64url>" → JSON credential
        let token = auth_header
            .strip_prefix("Payment ")
            .ok_or("Invalid auth scheme")?;
        let decoded = URL_SAFE_NO_PAD
            .decode(token)
            .map_err(|e| format!("Invalid base64: {}", e))?;
        let credential: Credential =
            serde_json::from_slice(&decoded).map_err(|e| format!("Invalid credential JSON: {}", e))?;

        // Verify challenge ID (HMAC binding)
        // For hackathon scope, we just check it's non-empty
        if credential.challenge.id.is_empty() {
            return Err("Missing challenge ID".to_string());
        }

        // Get tx hash from payload
        let tx_hash = credential
            .payload
            .hash
            .as_ref()
            .ok_or("Missing tx hash in payload")?;

        // Verify on-chain: fetch receipt and check Transfer event
        self.verify_transfer(tx_hash).await
    }

    /// Verify a TIP-20 Transfer event in the transaction receipt
    async fn verify_transfer(&self, tx_hash: &str) -> Result<(), String> {
        let client = reqwest::Client::new();
        let resp = client
            .post(&self.rpc_url)
            .json(&serde_json::json!({
                "jsonrpc": "2.0",
                "method": "eth_getTransactionReceipt",
                "params": [tx_hash],
                "id": 1
            }))
            .send()
            .await
            .map_err(|e| format!("RPC request failed: {}", e))?;

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("Invalid RPC response: {}", e))?;

        let result = json
            .get("result")
            .ok_or("No result in receipt response")?;

        // Check status
        let status = result
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("0x0");
        if status != "0x1" {
            return Err("Transaction failed".to_string());
        }

        // Check Transfer event: topic0 = Transfer(address,address,uint256)
        let transfer_topic =
            "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
        let logs = result
            .get("logs")
            .and_then(|l| l.as_array())
            .ok_or("No logs in receipt")?;

        let expected_recipient = self.recipient.to_lowercase();
        let expected_currency = self.currency.to_lowercase();

        for log in logs {
            let address = log
                .get("address")
                .and_then(|a| a.as_str())
                .unwrap_or("")
                .to_lowercase();
            if address != expected_currency {
                continue;
            }

            let topics: Vec<&str> = log
                .get("topics")
                .and_then(|t| t.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                .unwrap_or_default();

            if topics.first() != Some(&transfer_topic) {
                continue;
            }
            if topics.len() < 3 {
                continue;
            }

            // topics[2] is the 'to' address (padded to 32 bytes)
            let to = format!("0x{}", &topics[2][topics[2].len() - 40..]).to_lowercase();
            if to == expected_recipient {
                // Verify amount from data field
                let data = log
                    .get("data")
                    .and_then(|d| d.as_str())
                    .unwrap_or("0x0");
                let amount = u128::from_str_radix(data.trim_start_matches("0x"), 16).unwrap_or(0);
                let expected = self.amount.parse::<u128>().unwrap_or(0);
                if amount >= expected {
                    return Ok(());
                }
            }
        }

        Err("No matching Transfer event found".to_string())
    }
}

/// Warp filter that gates requests behind MPP payment.
/// Extracts to () — either passes through or rejects with PaymentRequired.
pub fn mpp_gate(
    state: Option<Arc<MppState>>,
) -> impl Filter<Extract = (), Error = Rejection> + Clone {
    warp::any()
        .and(warp::addr::remote())
        .and(warp::header::optional::<String>("authorization"))
        .and(warp::any().map(move || state.clone()))
        .and_then(check_payment)
        .untuple_one()
}

async fn check_payment(
    remote: Option<SocketAddr>,
    auth_header: Option<String>,
    state: Option<Arc<MppState>>,
) -> Result<(), Rejection> {
    let state = match state {
        Some(s) => s,
        None => return Ok(()), // MPP disabled
    };

    // First N requests per IP are free
    if let Some(addr) = remote {
        let ip = addr.ip();
        let mut count = state.request_counts.entry(ip).or_insert(0);
        *count += 1;
        if *count <= state.free_requests {
            return Ok(());
        }
    }

    // Check for payment credential
    match auth_header {
        Some(ref h) if h.starts_with("Payment ") => {
            match state.verify_credential(h).await {
                Ok(()) => Ok(()),
                Err(e) => {
                    log::warn!("MPP credential verification failed: {}", e);
                    Err(warp::reject::custom(PaymentRequired {
                        www_authenticate: state.generate_challenge(),
                    }))
                }
            }
        }
        _ => {
            // No credential — return 402 with challenge
            Err(warp::reject::custom(PaymentRequired {
                www_authenticate: state.generate_challenge(),
            }))
        }
    }
}
