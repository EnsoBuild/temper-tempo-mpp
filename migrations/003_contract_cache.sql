-- Contract metadata cache for faster lookups
CREATE TABLE IF NOT EXISTS contract_cache (
    chain_id INTEGER NOT NULL,
    address TEXT NOT NULL,
    name TEXT,
    abi TEXT,                         -- JSON ABI
    selectors TEXT,                   -- JSON map: selector -> function signature
    source_code TEXT,                 -- Optional: full source for debugger
    cached_at INTEGER NOT NULL,
    last_accessed INTEGER NOT NULL,
    PRIMARY KEY (chain_id, address)
);

CREATE INDEX IF NOT EXISTS idx_contract_cache_last_accessed
ON contract_cache(last_accessed);
