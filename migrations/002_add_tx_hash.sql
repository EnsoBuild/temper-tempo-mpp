-- Add tx_hash column for storing original transaction hash
ALTER TABLE simulations ADD COLUMN tx_hash TEXT;
