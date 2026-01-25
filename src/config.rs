use dotenvy::dotenv;
use std::collections::HashMap;

/// Mapping of network names to chain IDs
const NETWORK_CHAIN_IDS: &[(&str, u64)] = &[
    // Mainnets
    ("ETHEREUM", 1),
    ("POLYGON", 137),
    ("AVALANCHE", 43114),
    ("FANTOM", 250),
    ("GNOSIS", 100),
    ("BSC", 56),
    ("ARBITRUM", 42161),
    ("OPTIMISM", 10),
    ("BASE", 8453),
    ("LINEA", 59144),
    ("BERACHAIN", 80094),
    ("SONIC", 146),
    ("PLUME", 98866),
    ("HYPER", 999),
    ("PLASMA", 1012),
    // Testnets
    ("GOERLI", 5),
    ("SEPOLIA", 11155111),
    ("MUMBAI", 80001),
    ("AVALANCHE_FUJI", 43113),
    ("FANTOM_TESTNET", 4002),
    ("BSC_TESTNET", 97),
    ("ARBITRUM_GOERLI", 421613),
    ("OPTIMISM_GOERLI", 420),
    ("BASE_GOERLI", 84531),
    ("TEMPO_TESTNET", 42429),
];

#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    pub rpc_urls: HashMap<u64, String>,
    pub etherscan_key: Option<String>,
    pub api_key: Option<String>,
    pub max_request_size: u64,
}

impl Config {
    /// Get RPC URL for a given chain ID
    pub fn get_rpc_url(&self, chain_id: u64) -> Option<&String> {
        self.rpc_urls.get(&chain_id)
    }
}

pub fn config() -> Config {
    dotenv().ok();

    load_config()
}

fn load_config() -> Config {
    let port = std::env::var("PORT")
        .unwrap_or("8080".to_string())
        .parse::<u16>()
        .expect("PORT must be a valid u16.");

    // Load RPC URLs from environment variables
    let mut rpc_urls = HashMap::new();
    for (network_name, chain_id) in NETWORK_CHAIN_IDS {
        let env_var = format!("RPC_URL_{}", network_name);
        if let Ok(url) = std::env::var(&env_var) {
            if !url.is_empty() {
                rpc_urls.insert(*chain_id, url);
            }
        }
    }

    let etherscan_key = std::env::var("ETHERSCAN_KEY")
        .ok()
        .filter(|k| !k.is_empty());
    let api_key = std::env::var("API_KEY").ok().filter(|k| !k.is_empty());
    let max_request_size = std::env::var("MAX_REQUEST_SIZE")
        .unwrap_or("16".to_string())
        .parse::<u64>()
        .expect("MAX_REQUEST_SIZE must be a valid u64")
        * 1024;

    Config {
        port,
        rpc_urls,
        etherscan_key,
        api_key,
        max_request_size,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[should_panic(expected = "PORT must be a valid u16.")]
    fn test_config_port_number() {
        temp_env::with_var("PORT", Some("not a number"), || {
            super::load_config();
        });
    }

    #[test]
    fn test_config_rpc_urls() {
        temp_env::with_vars(
            [
                ("RPC_URL_ETHEREUM", Some("https://eth.example.com")),
                ("RPC_URL_POLYGON", Some("https://polygon.example.com")),
            ],
            || {
                let config = super::load_config();
                assert_eq!(
                    config.get_rpc_url(1),
                    Some(&"https://eth.example.com".to_string())
                );
                assert_eq!(
                    config.get_rpc_url(137),
                    Some(&"https://polygon.example.com".to_string())
                );
                assert_eq!(config.get_rpc_url(42161), None); // Arbitrum not set
            },
        );
    }

    #[test]
    fn test_config_rpc_urls_empty() {
        temp_env::with_vars([("RPC_URL_ETHEREUM", Some(""))], || {
            let config = super::load_config();
            assert_eq!(config.get_rpc_url(1), None);
        });
    }

    #[test]
    fn test_config_etherscan_key() {
        temp_env::with_vars([("ETHERSCAN_KEY", Some("a"))], || {
            let config = super::load_config();
            assert_eq!(config.etherscan_key, Some("a".to_string()));
        });

        temp_env::with_vars([("ETHERSCAN_KEY", Some(""))], || {
            let config = super::load_config();
            assert_eq!(config.etherscan_key, None);
        });

        temp_env::with_vars_unset([("ETHERSCAN_KEY")], || {
            let config = super::load_config();
            assert_eq!(config.etherscan_key, None);
        });
    }

    #[test]
    fn test_config_api_key() {
        temp_env::with_vars([("API_KEY", Some("a"))], || {
            let config = super::load_config();
            assert_eq!(config.api_key, Some("a".to_string()));
        });

        temp_env::with_vars([("API_KEY", Some(""))], || {
            let config = super::load_config();
            assert_eq!(config.api_key, None);
        });

        temp_env::with_vars_unset([("API_KEY")], || {
            let config = super::load_config();
            assert_eq!(config.api_key, None);
        });
    }
}
