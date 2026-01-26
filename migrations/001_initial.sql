-- Temper Database Schema v1
-- Initial migration for simulation storage

CREATE TABLE IF NOT EXISTS simulations (
    id TEXT PRIMARY KEY,              -- nanoid (21 chars, URL-safe)
    created_at INTEGER NOT NULL,
    chain_id INTEGER NOT NULL,
    block_number INTEGER NOT NULL,
    from_address TEXT NOT NULL,
    to_address TEXT,
    value TEXT NOT NULL,
    gas_limit TEXT NOT NULL,
    gas_used INTEGER NOT NULL,
    success INTEGER NOT NULL,
    exit_reason TEXT NOT NULL,
    return_data TEXT NOT NULL,
    input_data TEXT
);

CREATE TABLE IF NOT EXISTS traces (
    simulation_id TEXT PRIMARY KEY,
    trace_json TEXT NOT NULL,         -- Enhanced trace tree as JSON
    formatted_trace TEXT,
    FOREIGN KEY (simulation_id) REFERENCES simulations(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS logs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    simulation_id TEXT NOT NULL,
    address TEXT NOT NULL,
    topics TEXT NOT NULL,             -- JSON array of topic hex strings
    data TEXT NOT NULL,
    log_index INTEGER NOT NULL,
    FOREIGN KEY (simulation_id) REFERENCES simulations(id) ON DELETE CASCADE
);

-- Indexes for common queries
CREATE INDEX IF NOT EXISTS idx_simulations_created_at ON simulations(created_at);
CREATE INDEX IF NOT EXISTS idx_logs_simulation_id ON logs(simulation_id);
