//! Temper Server - Main entry point

use std::sync::Arc;
use std::{env, path::PathBuf};

use dashmap::DashMap;
use warp::Filter;

use temper_db::Database;
use temper_server::{
    config::config,
    contract_cache::ContractCacheService,
    errors::handle_rejection,
    mpp_gate,
    routes::api_routes,
    storage::StorageService,
    Config, SharedSimulationState,
};

#[tokio::main]
async fn main() {
    if env::var_os("RUST_LOG").is_none() {
        env::set_var("RUST_LOG", "temper_server=info");
    }
    pretty_env_logger::init();

    let config = config();
    let port = config.port;
    let api_key = config.api_key.clone();

    // Initialize database if storage is enabled
    let (storage, contract_cache) = if config.storage_enabled {
        match init_storage(&config).await {
            Ok((storage, cache)) => {
                log::info!(target: "temper_server", "Storage enabled with contract cache");
                (Some(Arc::new(storage)), Some(Arc::new(cache)))
            }
            Err(e) => {
                log::error!(target: "temper_server", "Failed to initialize storage: {}", e);
                (None, None)
            }
        }
    } else {
        log::info!(target: "temper_server", "Storage disabled");
        (None, None)
    };

    let shared_state = Arc::new(SharedSimulationState {
        evms: Arc::new(DashMap::new()),
    });

    // Initialize MPP payment gating
    let mpp_state = if config.mpp_enabled {
        match mpp_gate::MppState::new(&config) {
            Ok(state) => {
                log::info!(target: "temper_server", "MPP payment gating enabled (first request free)");
                Some(Arc::new(state))
            }
            Err(e) => {
                log::error!(target: "temper_server", "Failed to initialize MPP: {}", e);
                None
            }
        }
    } else {
        log::info!(target: "temper_server", "MPP disabled");
        None
    };

    // Build API routes
    let api_base = warp::path("api").and(warp::path("v1"));

    let api_base = if let Some(api_key) = api_key {
        log::info!(target: "temper_server", "Running with API key protection");
        let api_key_filter = warp::header::exact("X-API-KEY", Box::leak(api_key.into_boxed_str()));
        api_base.and(api_key_filter).boxed()
    } else {
        api_base.boxed()
    };

    let api = api_base.and(api_routes(config.clone(), shared_state, storage.clone(), contract_cache, mpp_state));

    // Build routes with optional web UI
    let routes = if config.web_ui_enabled {
        log::info!(target: "temper_server", "Web UI enabled");
        api.or(web_ui_routes(storage))
            .recover(handle_rejection)
            .with(warp::log("temper_server"))
            .boxed()
    } else {
        api.recover(handle_rejection)
            .with(warp::log("temper_server"))
            .boxed()
    };

    log::info!(target: "temper_server", "Starting server on port {port}");
    warp::serve(routes).run(([0, 0, 0, 0], port)).await;
}

async fn init_storage(config: &Config) -> eyre::Result<(StorageService, ContractCacheService)> {
    let db_path = config
        .database_url
        .as_ref()
        .map(|url| {
            // Handle sqlite: prefix
            if url.starts_with("sqlite:") {
                PathBuf::from(url.trim_start_matches("sqlite:"))
            } else {
                PathBuf::from(url)
            }
        })
        .unwrap_or_else(|| PathBuf::from("./temper.db"));

    log::info!(target: "temper_server", "Opening database at {:?}", db_path);

    let db = Database::open(&db_path)?;
    db.init().await?;

    let storage = StorageService::new(&db, config.clone());
    let contract_cache = ContractCacheService::new(&db, config.etherscan_key.clone());

    Ok((storage, contract_cache))
}

/// Web UI routes for viewing simulations
fn web_ui_routes(
    storage: Option<Arc<StorageService>>,
) -> impl Filter<Extract = (impl warp::Reply,), Error = warp::Rejection> + Clone {
    // GET /simulation/{id} - Serve web UI for viewing simulation
    let simulation_page = warp::path!("simulation" / String)
        .and(warp::get())
        .and(warp::any().map(move || storage.clone()))
        .and_then(serve_simulation_page);

    // GET /docs - Serve documentation page
    let docs_page = warp::path!("docs")
        .and(warp::get())
        .map(|| warp::reply::html(include_str!("../../../web/docs.html")));

    // GET /debug - Serve debugger page
    let debug_page = warp::path!("debug")
        .and(warp::get())
        .map(serve_debug_page);

    // Static files
    let static_files = warp::path("static")
        .and(warp::fs::dir("web"));

    // Root redirects to static
    let root = warp::path::end()
        .map(|| warp::reply::html(include_str!("../../../web/index.html")));

    simulation_page.or(docs_page).or(debug_page).or(static_files).or(root)
}

fn serve_debug_page() -> impl warp::Reply {
    let html = r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Temper Debugger</title>
    <link rel="stylesheet" href="/static/style.css?v=11">
    <link rel="stylesheet" href="/static/debugger.css?v=11">
</head>
<body>
    <div id="app">
        <div class="loading-container">
            <div class="loading-spinner"></div>
            <div>Initializing debugger...</div>
        </div>
    </div>
    <script src="/static/debugger.js?v=11"></script>
    <script>
        const params = new URLSearchParams(window.location.search);
        const txHash = params.get('tx');
        const chainId = parseInt(params.get('chain') || '1');

        if (txHash) {
            loadDebugTrace(txHash, chainId);
        } else {
            document.getElementById('app').innerHTML = `
                <div class="error-container">
                    <div class="error-icon">⚠</div>
                    <div class="error-title">Missing transaction hash</div>
                    <div class="error-message-text">Please provide a tx parameter in the URL</div>
                </div>
            `;
        }
    </script>
</body>
</html>"#;
    warp::reply::html(html)
}

async fn serve_simulation_page(
    id: String,
    storage: Option<Arc<StorageService>>,
) -> Result<impl warp::Reply, warp::Rejection> {
    // For now, just serve the HTML with the simulation ID embedded
    // The JavaScript will fetch the simulation data
    let html = format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Simulation {id} - Temper</title>
    <link rel="stylesheet" href="/static/style.css?v=11">
</head>
<body>
    <div id="app">
        <h1>Temper Simulation Viewer</h1>
        <div id="simulation-id" data-id="{id}"></div>
        <div id="trace-container"></div>
    </div>
    <script src="/static/script.js?v=11"></script>
    <script src="/static/temper-adapter.js?v=11"></script>
    <script>
        window.SIMULATION_ID = "{id}";
        if (typeof loadSimulation === 'function') {{
            loadSimulation("{id}");
        }}
    </script>
</body>
</html>"#,
        id = id
    );

    // Check if simulation exists
    if let Some(ref storage) = storage {
        match storage.get_simulation(&id).await {
            Ok(Some(_)) => Ok(warp::reply::html(html)),
            Ok(None) => Err(warp::reject::not_found()),
            Err(_) => Err(warp::reject::not_found()),
        }
    } else {
        // No storage, just serve the page
        Ok(warp::reply::html(html))
    }
}
