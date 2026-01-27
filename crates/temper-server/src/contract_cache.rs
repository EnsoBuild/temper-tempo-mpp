//! Contract cache service - caches contract metadata from external sources

use std::time::{SystemTime, UNIX_EPOCH};

use temper_db::{CachedContract, ContractCacheRepository, Database};

use crate::debugger::{fetch_contract_source, ContractSource};

/// Maximum cache size in bytes (500MB)
const MAX_CACHE_SIZE: u64 = 500 * 1024 * 1024;

/// Contract cache service
pub struct ContractCacheService {
    repo: ContractCacheRepository,
    etherscan_key: Option<String>,
}

impl ContractCacheService {
    pub fn new(db: &Database, etherscan_key: Option<String>) -> Self {
        Self {
            repo: ContractCacheRepository::new(db.connection()),
            etherscan_key,
        }
    }

    /// Get contract source, checking cache first
    pub async fn get_contract_source(
        &self,
        chain_id: u64,
        address: &str,
    ) -> Result<Option<ContractSource>, String> {
        let address = address.to_lowercase();

        // Check cache first
        if let Ok(Some(cached)) = self.repo.get_contract(chain_id, &address).await {
            log::debug!("Cache hit for contract {} on chain {}", address, chain_id);

            // Update last accessed time (fire and forget)
            let _ = self.repo.update_last_accessed(chain_id, &address).await;

            // Convert cached contract to ContractSource
            return Ok(cached_to_source(cached));
        }

        log::debug!("Cache miss for contract {} on chain {}, fetching...", address, chain_id);

        // Fetch from external sources
        let source = fetch_contract_source(&address, chain_id, self.etherscan_key.as_deref()).await?;

        // Cache the result if we got a source
        if let Some(ref src) = source {
            if let Err(e) = self.cache_source(chain_id, &address, src).await {
                log::warn!("Failed to cache contract source: {}", e);
            }
        }

        Ok(source)
    }

    /// Cache a contract source
    async fn cache_source(
        &self,
        chain_id: u64,
        address: &str,
        source: &ContractSource,
    ) -> eyre::Result<()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        // Serialize selectors from ABI
        let selectors = extract_selectors(&source.abi);

        // Combine all sources into one string for storage
        let source_code = if source.sources.len() == 1 {
            source.sources.values().next().cloned()
        } else {
            // For multi-file contracts, store as JSON
            Some(serde_json::to_string(&source.sources).unwrap_or_default())
        };

        let cached = CachedContract {
            chain_id,
            address: address.to_lowercase(),
            name: Some(source.name.clone()),
            abi: Some(serde_json::to_string(&source.abi).unwrap_or_default()),
            selectors: Some(serde_json::to_string(&selectors).unwrap_or_default()),
            source_code,
            cached_at: now,
            last_accessed: now,
        };

        self.repo.cache_contract(&cached).await?;

        // Check if we need to evict old entries
        self.maybe_evict().await;

        Ok(())
    }

    /// Evict old entries if cache is too large
    async fn maybe_evict(&self) {
        if let Ok(size) = self.repo.get_cache_size().await {
            if size > MAX_CACHE_SIZE {
                log::info!("Contract cache size {}MB exceeds limit, evicting...", size / 1024 / 1024);
                if let Ok(evicted) = self.repo.evict_lru(MAX_CACHE_SIZE * 80 / 100).await {
                    log::info!("Evicted {} contracts from cache", evicted);
                }
            }
        }
    }

    /// Get cache statistics
    pub async fn get_stats(&self) -> eyre::Result<CacheStats> {
        let count = self.repo.get_cache_count().await?;
        let size = self.repo.get_cache_size().await?;
        Ok(CacheStats { count, size_bytes: size })
    }
}

/// Cache statistics
#[derive(Debug, Clone, serde::Serialize)]
pub struct CacheStats {
    pub count: u64,
    pub size_bytes: u64,
}

/// Convert cached contract back to ContractSource
fn cached_to_source(cached: CachedContract) -> Option<ContractSource> {
    let name = cached.name.unwrap_or_default();
    if name.is_empty() {
        return None;
    }

    // Parse ABI
    let abi: serde_json::Value = cached
        .abi
        .as_ref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::Value::Array(vec![]));

    // Parse source code
    let sources = if let Some(ref src) = cached.source_code {
        // Try to parse as JSON map first (multi-file)
        if let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, String>>(src) {
            map
        } else {
            // Single file
            let mut m = std::collections::HashMap::new();
            m.insert(format!("{}.sol", name), src.clone());
            m
        }
    } else {
        std::collections::HashMap::new()
    };

    let source_list: Vec<String> = sources.keys().cloned().collect();

    Some(ContractSource {
        name,
        sources,
        source_list,
        abi,
        source_map: String::new(),
        source_locations: vec![],
    })
}

/// Extract function selectors from ABI
fn extract_selectors(abi: &serde_json::Value) -> std::collections::HashMap<String, String> {
    use sha3::{Digest, Keccak256};

    let mut selectors = std::collections::HashMap::new();

    if let Some(items) = abi.as_array() {
        for item in items {
            if item.get("type").and_then(|t| t.as_str()) == Some("function") {
                if let Some(name) = item.get("name").and_then(|n| n.as_str()) {
                    // Build signature
                    let inputs = item
                        .get("inputs")
                        .and_then(|i| i.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|p| p.get("type").and_then(|t| t.as_str()))
                                .collect::<Vec<_>>()
                                .join(",")
                        })
                        .unwrap_or_default();

                    let signature = format!("{}({})", name, inputs);

                    // Calculate selector
                    let mut hasher = Keccak256::new();
                    hasher.update(signature.as_bytes());
                    let result = hasher.finalize();
                    let selector = format!("0x{}", hex::encode(&result[..4]));

                    selectors.insert(selector, signature);
                }
            }
        }
    }

    selectors
}
