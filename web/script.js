(function() {
    'use strict';

    const selectorCache = new Map();
    const contractCache = new Map(); // Stores { name, abi }
    const abiSelectorCache = new Map(); // Maps selector -> { name, inputs } from ABIs
    let currentChainId = 1;

    // Well-known function signatures with parameter names
    const KNOWN_FUNCTIONS = {
        // ERC20
        'transfer(address,uint256)': { name: 'transfer', inputs: ['to', 'amount'] },
        'transferFrom(address,address,uint256)': { name: 'transferFrom', inputs: ['from', 'to', 'amount'] },
        'approve(address,uint256)': { name: 'approve', inputs: ['spender', 'amount'] },
        'balanceOf(address)': { name: 'balanceOf', inputs: ['account'] },
        'allowance(address,address)': { name: 'allowance', inputs: ['owner', 'spender'] },
        // ERC721
        'safeTransferFrom(address,address,uint256)': { name: 'safeTransferFrom', inputs: ['from', 'to', 'tokenId'] },
        'safeTransferFrom(address,address,uint256,bytes)': { name: 'safeTransferFrom', inputs: ['from', 'to', 'tokenId', 'data'] },
        'setApprovalForAll(address,bool)': { name: 'setApprovalForAll', inputs: ['operator', 'approved'] },
        'ownerOf(uint256)': { name: 'ownerOf', inputs: ['tokenId'] },
        // Common DeFi
        'swap(uint256,uint256,address,bytes)': { name: 'swap', inputs: ['amount0Out', 'amount1Out', 'to', 'data'] },
        'swapExactTokensForTokens(uint256,uint256,address[],address,uint256)': { name: 'swapExactTokensForTokens', inputs: ['amountIn', 'amountOutMin', 'path', 'to', 'deadline'] },
        'swapExactETHForTokens(uint256,address[],address,uint256)': { name: 'swapExactETHForTokens', inputs: ['amountOutMin', 'path', 'to', 'deadline'] },
        'swapExactTokensForETH(uint256,uint256,address[],address,uint256)': { name: 'swapExactTokensForETH', inputs: ['amountIn', 'amountOutMin', 'path', 'to', 'deadline'] },
        'multicall(bytes[])': { name: 'multicall', inputs: ['data'] },
        'multicall(uint256,bytes[])': { name: 'multicall', inputs: ['deadline', 'data'] },
        'execute(bytes,bytes[],uint256)': { name: 'execute', inputs: ['commands', 'inputs', 'deadline'] },
        'deposit()': { name: 'deposit', inputs: [] },
        'withdraw(uint256)': { name: 'withdraw', inputs: ['amount'] },
        // Permit
        'permit(address,address,uint256,uint256,uint8,bytes32,bytes32)': { name: 'permit', inputs: ['owner', 'spender', 'value', 'deadline', 'v', 'r', 's'] },
    };

    function getKnownParamNames(signature) {
        const known = KNOWN_FUNCTIONS[signature];
        return known ? known.inputs : null;
    }

    // Tempo chain IDs
    const TEMPO_CHAIN_IDS = new Set([4217, 42429, 42431]);

    function isTempoChain(chainId) {
        return TEMPO_CHAIN_IDS.has(chainId);
    }

    // Returns true if the address is a TIP-20 precompile (0x20c0 prefix)
    function isTip20Address(addr) {
        if (!addr) return false;
        const lower = addr.toLowerCase();
        return lower.startsWith('0x20c0');
    }

    // Native token config by chain ID
    const NATIVE_TOKENS = {
        1: { symbol: 'ETH', decimals: 18, logo: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/ethereum/info/logo.png' },
        10: { symbol: 'ETH', decimals: 18, logo: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/optimism/info/logo.png' },
        56: { symbol: 'BNB', decimals: 18, logo: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/binance/info/logo.png' },
        100: { symbol: 'xDAI', decimals: 18, logo: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/xdai/info/logo.png' },
        137: { symbol: 'POL', decimals: 18, logo: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/polygon/info/logo.png' },
        250: { symbol: 'FTM', decimals: 18, logo: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/fantom/info/logo.png' },
        999: { symbol: 'HYPE', decimals: 18, logo: null },
        4217: { symbol: 'ETH', decimals: 18, logo: null },  // Tempo Mainnet
        42429: { symbol: 'ETH', decimals: 18, logo: null }, // Tempo Testnet
        42431: { symbol: 'ETH', decimals: 18, logo: null }, // Tempo Moderato
        8453: { symbol: 'ETH', decimals: 18, logo: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/base/info/logo.png' },
        42161: { symbol: 'ETH', decimals: 18, logo: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/arbitrum/info/logo.png' },
        43114: { symbol: 'AVAX', decimals: 18, logo: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/avalanchec/info/logo.png' },
    };

    function getNativeToken() {
        return NATIVE_TOKENS[currentChainId]?.symbol || 'ETH';
    }

    function getNativeTokenConfig() {
        return NATIVE_TOKENS[currentChainId] || { symbol: 'ETH', decimals: 18, logo: null };
    }

    // Chain names
    const CHAIN_NAMES = {
        1: 'Ethereum',
        10: 'Optimism',
        56: 'BNB Chain',
        100: 'Gnosis',
        137: 'Polygon',
        250: 'Fantom',
        999: 'HyperEVM',
        4217: 'Tempo Mainnet',
        8453: 'Base',
        42161: 'Arbitrum',
        42429: 'Tempo Testnet',
        42431: 'Tempo Moderato',
        43114: 'Avalanche',
    };

    function getChainName(chainId) {
        return CHAIN_NAMES[chainId] || `Chain ${chainId}`;
    }

    function getChainIcon(chainId) {
        const icons = {
            1: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/ethereum/info/logo.png',
            10: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/optimism/info/logo.png',
            56: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/binance/info/logo.png',
            137: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/polygon/info/logo.png',
            8453: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/base/info/logo.png',
            42161: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/arbitrum/info/logo.png',
            43114: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/avalanchec/info/logo.png',
        };
        return icons[chainId] || null;
    }

    const EXPLORER_TX_URLS = {
        1:     'https://etherscan.io/tx/',
        10:    'https://optimistic.etherscan.io/tx/',
        56:    'https://bscscan.com/tx/',
        100:   'https://gnosisscan.io/tx/',
        137:   'https://polygonscan.com/tx/',
        8453:  'https://basescan.org/tx/',
        42161: 'https://arbiscan.io/tx/',
        43114: 'https://snowtrace.io/tx/',
        // Tempo
        4217:  'https://explore.tempo.xyz/tx/',
        42429: 'https://explore.testnet.tempo.xyz/tx/',
        42431: 'https://explore.moderato.tempo.xyz/tx/',
    };

    const EXPLORER_ADDR_URLS = {
        1:     'https://etherscan.io/address/',
        10:    'https://optimistic.etherscan.io/address/',
        56:    'https://bscscan.com/address/',
        100:   'https://gnosisscan.io/address/',
        137:   'https://polygonscan.com/address/',
        8453:  'https://basescan.org/address/',
        42161: 'https://arbiscan.io/address/',
        43114: 'https://snowtrace.io/address/',
        // Tempo
        4217:  'https://explore.tempo.xyz/address/',
        42429: 'https://explore.testnet.tempo.xyz/address/',
        42431: 'https://explore.moderato.tempo.xyz/address/',
    };

    function getExplorerTxUrl(chainId, txHash) {
        const base = EXPLORER_TX_URLS[chainId];
        return base ? base + txHash : null;
    }

    function getExplorerAddrUrl(chainId, addr) {
        const base = EXPLORER_ADDR_URLS[chainId];
        return base ? base + addr : null;
    }

    const ZERO_ADDRESS = '0x0000000000000000000000000000000000000000';

    // ─── Address Resolution ────────────────────────────────────────────
    //
    // Priority: system label → TIP-20 token symbol → Sourcify/Etherscan
    //           contract name → shortened address
    //
    // Returns: { label, type, tooltip }
    //   type: "system" | "token" | "contract" | "address"

    // Known system/infrastructure addresses per chain
    const SYSTEM_ADDRESSES = {
        // Tempo chains — zero address acts as the tx entrypoint/dispatcher
        4217:  { [ZERO_ADDRESS]: 'Entrypoint', '0xfeec000000000000000000000000000000000000': 'Fee Collector' },
        42429: { [ZERO_ADDRESS]: 'Entrypoint', '0xfeec000000000000000000000000000000000000': 'Fee Collector' },
        42431: { [ZERO_ADDRESS]: 'Entrypoint', '0xfeec000000000000000000000000000000000000': 'Fee Collector' },
    };

    // Known TIP-20 tokens from tokenlist.tempo.xyz (all Tempo chains)
    const TIP20_TOKENS = {
        '4217': {
            '0x20c0000000000000000000000000000000000000': { symbol: 'pathUSD', name: 'PathUSD', decimals: 6 },
            '0x20c000000000000000000000b9537d11c60e8b50': { symbol: 'USDC.e', name: 'Bridged USDC (Stargate)', decimals: 6 },
            '0x20c0000000000000000000001621e21f71cf12fb': { symbol: 'EURC.e', name: 'Bridged EURC (Stargate)', decimals: 6 },
            '0x20c00000000000000000000014f22ca97301eb73': { symbol: 'USDT0', name: 'USDT0', decimals: 6 },
            '0x20c0000000000000000000003554d28269e0f3c2': { symbol: 'frxUSD', name: 'Frax USD', decimals: 6 },
            '0x20c0000000000000000000000520792dcccccccc': { symbol: 'cUSD', name: 'Cap USD', decimals: 6 },
            '0x20c00000000000000000000031f228af88888888': { symbol: 'stcUSD', name: 'Staked Cap USD', decimals: 6 },
        },
        '42429': {
            '0x20c0000000000000000000000000000000000000': { symbol: 'pathUSD', name: 'PathUSD', decimals: 6 },
            '0x20c0000000000000000000000000000000000001': { symbol: 'alphaUSD', name: 'AlphaUSD', decimals: 6 },
            '0x20c0000000000000000000000000000000000002': { symbol: 'betaUSD', name: 'BetaUSD', decimals: 6 },
            '0x20c0000000000000000000000000000000000003': { symbol: 'thetaUSD', name: 'ThetaUSD', decimals: 6 },
            '0x20c0000000000000000000009e8d7eb59b783726': { symbol: 'USDC.e', name: 'Bridged USDC (Stargate)', decimals: 6 },
            '0x20c000000000000000000000d72572838bbee59c': { symbol: 'EURC.e', name: 'Bridged EURC (Stargate)', decimals: 6 },
        },
        '42431': {
            '0x20c0000000000000000000000000000000000000': { symbol: 'pathUSD', name: 'PathUSD', decimals: 6 },
            '0x20c0000000000000000000000000000000000001': { symbol: 'alphaUSD', name: 'AlphaUSD', decimals: 6 },
            '0x20c0000000000000000000000000000000000002': { symbol: 'betaUSD', name: 'BetaUSD', decimals: 6 },
            '0x20c0000000000000000000000000000000000003': { symbol: 'thetaUSD', name: 'ThetaUSD', decimals: 6 },
        },
    };

    // Unified cache for all address resolution
    const addressNameCache = new Map();

    /**
     * Resolve a human-readable label for any address.
     * Returns { label, type, tooltip } where type helps style the result.
     */
    async function resolveAddressName(address, chainId) {
        if (!address) return null;
        const lower = address.toLowerCase();
        const cacheKey = `${chainId}:${lower}`;
        if (addressNameCache.has(cacheKey)) return addressNameCache.get(cacheKey);

        let result = null;

        // 1. System addresses (instant, no fetch)
        const systemMap = SYSTEM_ADDRESSES[chainId];
        if (systemMap && systemMap[lower]) {
            result = { label: systemMap[lower], type: 'system', tooltip: `${systemMap[lower]} (${address})` };
        }

        // 2. TIP-20 tokens (local map → tokenlist API)
        if (!result && isTip20Address(lower)) {
            const chainTokens = TIP20_TOKENS[String(chainId)];
            const local = chainTokens && chainTokens[lower];
            if (local) {
                result = { label: local.symbol, type: 'token', tooltip: `${local.name} (${address})` };
            } else {
                // Fetch from tokenlist API
                try {
                    const resp = await fetch(`https://tokenlist.tempo.xyz/asset/${chainId}/${lower}`, { signal: AbortSignal.timeout(2000) });
                    if (resp.ok) {
                        const data = await resp.json();
                        if (data.symbol) {
                            result = { label: data.symbol, type: 'token', tooltip: `${data.name || data.symbol} (${address})` };
                        }
                    }
                } catch {}
                // If tokenlist doesn't know it, it's still a TIP-20 — mark as unknown token
                if (!result) {
                    result = { label: shortenAddress(lower), type: 'token', tooltip: `TIP-20 Token (${address})` };
                }
            }
        }

        // 3. Verified contract name (Sourcify / Etherscan)
        if (!result) {
            const contractInfo = await fetchContractInfo(lower, chainId);
            if (contractInfo?.name) {
                result = { label: contractInfo.name, type: 'contract', tooltip: `${contractInfo.name} (${address})`, abi: contractInfo.abi };
            }
        }

        // 4. Fallback: shortened address
        if (!result) {
            result = { label: shortenAddress(lower), type: 'address', tooltip: address };
        }

        addressNameCache.set(cacheKey, result);
        return result;
    }

    // Shorten an address to 0x1234…abcd form
    function shortenAddress(addr) {
        if (!addr || addr.length < 10) return addr;
        return addr.slice(0, 6) + '\u2026' + addr.slice(-4);
    }

    function truncateValue(value) {
        if (!value) return '';
        const str = String(value);
        if (str.length <= 20) return str;
        if (str.startsWith('0x')) {
            return str.slice(0, 10) + '...' + str.slice(-8);
        }
        return str.slice(0, 16) + '...';
    }

    // Token metadata cache
    const tokenMetadataCache = new Map();

    // Chain-specific token list URLs
    const TOKEN_LIST_URLS = {
        1: 'https://tokens.coingecko.com/uniswap/all.json',
        137: 'https://tokens.coingecko.com/polygon-pos/all.json',
        42161: 'https://tokens.coingecko.com/arbitrum-one/all.json',
        10: 'https://tokens.coingecko.com/optimistic-ethereum/all.json',
        8453: 'https://tokens.coingecko.com/base/all.json',
    };

    // Well-known tokens (fallback when token list fails)
    const WELL_KNOWN_TOKENS = {
        // Ethereum mainnet
        '1:0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2': { symbol: 'WETH', decimals: 18, logo: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/ethereum/assets/0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2/logo.png' },
        '1:0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48': { symbol: 'USDC', decimals: 6, logo: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/ethereum/assets/0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48/logo.png' },
        '1:0xdac17f958d2ee523a2206206994597c13d831ec7': { symbol: 'USDT', decimals: 6, logo: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/ethereum/assets/0xdAC17F958D2ee523a2206206994597C13D831ec7/logo.png' },
        '1:0x6b175474e89094c44da98b954eedeac495271d0f': { symbol: 'DAI', decimals: 18, logo: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/ethereum/assets/0x6B175474E89094C44Da98b954EesdeAC495271d0F/logo.png' },
        '1:0x2260fac5e5542a773aa44fbcfedf7c193bc2c599': { symbol: 'WBTC', decimals: 8, logo: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/ethereum/assets/0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599/logo.png' },
        // Tempo Mainnet TIP-20 tokens
        '4217:0x20c0000000000000000000000000000000000000': { symbol: 'pathUSD', decimals: 6, logo: 'https://tokenlist.tempo.xyz/icon/4217/0x20c0000000000000000000000000000000000000' },
        '4217:0x20c000000000000000000000b9537d11c60e8b50': { symbol: 'USDC.e', decimals: 6, logo: 'https://tokenlist.tempo.xyz/icon/4217/0x20c000000000000000000000b9537d11c60e8b50' },
        '4217:0x20c0000000000000000000001621e21f71cf12fb': { symbol: 'EURC.e', decimals: 6, logo: 'https://tokenlist.tempo.xyz/icon/4217/0x20c0000000000000000000001621e21f71cf12fb' },
        '4217:0x20c00000000000000000000014f22ca97301eb73': { symbol: 'USDT0', decimals: 6, logo: 'https://tokenlist.tempo.xyz/icon/4217/0x20c00000000000000000000014f22ca97301eb73' },
        '4217:0x20c0000000000000000000003554d28269e0f3c2': { symbol: 'frxUSD', decimals: 6, logo: 'https://tokenlist.tempo.xyz/icon/4217/0x20c0000000000000000000003554d28269e0f3c2' },
        '4217:0x20c0000000000000000000000520792dcccccccc': { symbol: 'cUSD', decimals: 6, logo: 'https://tokenlist.tempo.xyz/icon/4217/0x20c0000000000000000000000520792dcccccccc' },
        '4217:0x20c00000000000000000000031f228af88888888': { symbol: 'stcUSD', decimals: 6, logo: 'https://tokenlist.tempo.xyz/icon/4217/0x20c00000000000000000000031f228af88888888' },
        // Common across chains
        'usdc': { symbol: 'USDC', decimals: 6, logo: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/ethereum/assets/0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48/logo.png' },
        'usdt': { symbol: 'USDT', decimals: 6, logo: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/ethereum/assets/0xdAC17F958D2ee523a2206206994597C13D831ec7/logo.png' },
        'weth': { symbol: 'WETH', decimals: 18, logo: 'https://raw.githubusercontent.com/trustwallet/assets/master/blockchains/ethereum/assets/0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2/logo.png' },
    };

    // Fetch token metadata (symbol, decimals, logo)
    async function fetchTokenMetadata(tokenAddress, chainId) {
        const cacheKey = `${chainId}:${tokenAddress.toLowerCase()}`;
        if (tokenMetadataCache.has(cacheKey)) {
            return tokenMetadataCache.get(cacheKey);
        }

        // Check well-known tokens first
        if (WELL_KNOWN_TOKENS[cacheKey]) {
            tokenMetadataCache.set(cacheKey, WELL_KNOWN_TOKENS[cacheKey]);
            return WELL_KNOWN_TOKENS[cacheKey];
        }

        // Try to get from contract name cache (might have symbol in name)
        const contractInfo = contractCache.get(cacheKey);
        if (contractInfo?.name) {
            // Use contract name as fallback symbol
            const metadata = {
                symbol: contractInfo.name.length <= 10 ? contractInfo.name : contractInfo.name.slice(0, 8),
                decimals: 18,
                logo: null
            };
            tokenMetadataCache.set(cacheKey, metadata);
            return metadata;
        }

        // Default fallback
        const fallback = { symbol: 'TOKEN', decimals: 18, logo: null };
        tokenMetadataCache.set(cacheKey, fallback);
        return fallback;
    }

    // Format token amount with proper decimals
    function formatTokenAmount(rawAmount, decimals = 18) {
        try {
            const amount = BigInt(rawAmount);
            const divisor = BigInt(10 ** decimals);
            const intPart = amount / divisor;
            const fracPart = amount % divisor;

            // Format the fractional part with leading zeros
            let fracStr = fracPart.toString().padStart(decimals, '0');
            // Trim trailing zeros but keep at least 2 decimal places for readability
            fracStr = fracStr.replace(/0+$/, '');
            if (fracStr.length < 2) fracStr = fracStr.padEnd(2, '0');
            if (fracStr.length > 6) fracStr = fracStr.slice(0, 6);

            const formatted = `${intPart.toLocaleString()}.${fracStr}`;
            return formatted;
        } catch {
            return rawAmount;
        }
    }

    // Extract revert reason from trace (recursively searches call tree)
    function extractRevertReason(trace) {
        if (!trace) return null;

        // Check current call for error
        if (trace.error || trace.revertReason) {
            const reason = trace.revertReason || trace.error;
            // Try to decode if it's hex data
            if (reason && reason.startsWith('0x')) {
                return decodeRevertReason(reason);
            }
            return reason;
        }

        // Search subcalls recursively
        const subcalls = trace.calls || trace.subtraces || [];
        for (const subcall of subcalls) {
            const reason = extractRevertReason(subcall);
            if (reason) return reason;
        }

        return null;
    }

    // Decode hex revert reason (Error(string) or custom error)
    function decodeRevertReason(hexData) {
        if (!hexData || hexData.length < 10) return hexData;

        // Error(string) selector: 0x08c379a0
        if (hexData.startsWith('0x08c379a0') && hexData.length > 138) {
            try {
                // Skip selector (4 bytes) + offset (32 bytes) + length position
                const lengthHex = hexData.slice(74, 138);
                const length = parseInt(lengthHex, 16);
                const stringHex = hexData.slice(138, 138 + length * 2);
                const bytes = [];
                for (let i = 0; i < stringHex.length; i += 2) {
                    bytes.push(parseInt(stringHex.substr(i, 2), 16));
                }
                return new TextDecoder().decode(new Uint8Array(bytes));
            } catch (e) {
                return hexData;
            }
        }

        // Panic(uint256) selector: 0x4e487b71
        if (hexData.startsWith('0x4e487b71') && hexData.length >= 74) {
            const panicCode = parseInt(hexData.slice(10, 74), 16);
            const panicMessages = {
                0x00: 'Generic panic',
                0x01: 'Assertion failed',
                0x11: 'Arithmetic overflow/underflow',
                0x12: 'Division by zero',
                0x21: 'Invalid enum value',
                0x22: 'Storage byte array corrupted',
                0x31: 'Pop on empty array',
                0x32: 'Array index out of bounds',
                0x41: 'Allocation too much memory',
                0x51: 'Zero internal function call',
            };
            return panicMessages[panicCode] || `Panic(0x${panicCode.toString(16)})`;
        }

        // Return shortened hex if we can't decode
        return hexData.length > 20 ? hexData.slice(0, 20) + '...' : hexData;
    }

    // Block explorer API URLs by chain ID (Etherscan-compatible)
    // Tempo chains use Sourcify only (no Etherscan API) - fetchFromSourcify handles them
    const EXPLORER_APIS = {
        1: 'https://api.etherscan.io/api',
        137: 'https://api.polygonscan.com/api',
        56: 'https://api.bscscan.com/api',
        42161: 'https://api.arbiscan.io/api',
        10: 'https://api-optimistic.etherscan.io/api',
        8453: 'https://api.basescan.org/api',
        43114: 'https://api.snowtrace.io/api',
        250: 'https://api.ftmscan.com/api',
        100: 'https://api.gnosisscan.io/api',
        999: 'https://www.hyperscan.com/api', // HyperEVM via Blockscout
        // Tempo chains (4217, 42429, 42431) use Sourcify only — no Etherscan API
    };

    // Fetch contract info from Sourcify first, then Blockscout/Etherscan fallback
    async function fetchContractInfo(address, chainId) {
        const cacheKey = `${chainId}:${address.toLowerCase()}`;
        if (contractCache.has(cacheKey)) {
            return contractCache.get(cacheKey);
        }

        // Race all available sources in parallel - use first successful result
        const promises = [fetchFromSourcify(address, chainId)];

        // Tempo chains: also try Tempo's own Sourcify instance + explorer ABI endpoint
        if (isTempoChain(chainId)) {
            promises.push(fetchFromTempoContracts(address, chainId));
            promises.push(fetchFromTempoExplorer(address, chainId));
        }

        const explorerApi = EXPLORER_APIS[chainId];
        if (explorerApi) {
            promises.push(fetchFromEtherscan(address, chainId, explorerApi));
        }

        try {
            // Promise.any returns first successful result
            const result = await Promise.any(promises);
            if (result) {
                contractCache.set(cacheKey, result);
                cacheAbiSelectors(result.abi);
                return result;
            }
        } catch {
            // All failed
        }

        contractCache.set(cacheKey, null);
        return null;
    }

    async function fetchFromSourcify(address, chainId) {
        const controller = new AbortController();
        const timeout = setTimeout(() => controller.abort(), 2000);

        const response = await fetch(
            `https://sourcify.dev/server/files/any/${chainId}/${address}`,
            { signal: controller.signal }
        );
        clearTimeout(timeout);

        if (!response.ok) throw new Error('Not found');

        const data = await response.json();
        let contractName = null;
        let abi = null;

        if (data.files) {
            const metadataFile = data.files.find(f => f.name === 'metadata.json');
            if (metadataFile && metadataFile.content) {
                const metadata = JSON.parse(metadataFile.content);
                const compilationTarget = metadata.settings?.compilationTarget;
                if (compilationTarget) {
                    contractName = Object.values(compilationTarget)[0];
                }
                if (metadata.output?.abi) {
                    abi = metadata.output.abi;
                }
            }
        }

        if (contractName || abi) {
            return { name: contractName, abi };
        }
        throw new Error('No contract info');
    }

    // Tempo's own Sourcify instance at contracts.tempo.xyz (v2 API)
    async function fetchFromTempoContracts(address, chainId) {
        const resp = await fetch(
            `https://contracts.tempo.xyz/v2/contract/${chainId}/${address}?fields=name,abi`,
            { signal: AbortSignal.timeout(3000) }
        );
        if (!resp.ok) throw new Error('Not found');
        const data = await resp.json();
        if (data.name || data.abi) {
            return { name: data.name || null, abi: data.abi || null };
        }
        throw new Error('No contract info');
    }

    // Tempo explorer ABI batch endpoint — uses whatsabi for unverified contracts too
    const TEMPO_EXPLORER_URLS = {
        4217:  'https://explore.tempo.xyz',
        42429: 'https://explore.testnet.tempo.xyz',
        42431: 'https://explore.moderato.tempo.xyz',
    };

    async function fetchFromTempoExplorer(address, chainId) {
        const base = TEMPO_EXPLORER_URLS[chainId];
        if (!base) throw new Error('No explorer');
        const resp = await fetch(`${base}/api/abi/batch`, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ addresses: [address.toLowerCase()], selectors: [] }),
            signal: AbortSignal.timeout(3000),
        });
        if (!resp.ok) throw new Error('Failed');
        const data = await resp.json();
        const abi = data.abis?.[address.toLowerCase()];
        if (abi && Array.isArray(abi) && abi.length > 0) {
            // Try to extract contract name from ABI (constructor or contract-level)
            return { name: null, abi };
        }
        throw new Error('No ABI');
    }

    async function fetchFromEtherscan(address, chainId, explorerApi) {
        const controller = new AbortController();
        const timeout = setTimeout(() => controller.abort(), 2000);

        const response = await fetch(
            `${explorerApi}?module=contract&action=getsourcecode&address=${address}`,
            { signal: controller.signal }
        );
        clearTimeout(timeout);

        if (!response.ok) throw new Error('Request failed');

        const data = await response.json();
        if (data.status === '1' && data.result && data.result.length > 0) {
            const result = data.result[0];
            const contractName = result.ContractName || null;
            let abi = null;

            if (result.ABI && result.ABI !== 'Contract source code not verified') {
                try {
                    abi = JSON.parse(result.ABI);
                } catch {}
            }

            if (contractName || abi) {
                return { name: contractName, abi };
            }
        }
        throw new Error('No contract info');
    }

    // Build a selector -> function info map from an ABI
    function buildAbiFunctionMap(abi) {
        if (!abi || !Array.isArray(abi)) return new Map();

        const map = new Map();
        for (const item of abi) {
            if (item.type === 'function' && item.name) {
                const inputs = item.inputs || [];
                const sig = `${item.name}(${inputs.map(i => i.type).join(',')})`;
                // Store by signature so we can look it up after getting selector from 4byte
                map.set(sig, {
                    name: item.name,
                    inputs: inputs.map(i => ({ name: i.name || '', type: i.type }))
                });
            }
        }
        return map;
    }

    // Find function info in ABI by matching signature pattern
    function findInAbi(abiMap, signature) {
        if (!abiMap || abiMap.size === 0) return null;

        // First try exact match
        if (abiMap.has(signature)) {
            return abiMap.get(signature);
        }

        // Try to find by function name (first part before parenthesis)
        const parsed = parseSignature(signature);
        for (const [sig, info] of abiMap) {
            if (info.name === parsed.name && info.inputs.length === parsed.params.length) {
                return info;
            }
        }

        return null;
    }

    // Get a human label for any address (used for inline parameter display)
    async function fetchContractName(address, chainId) {
        const resolved = await resolveAddressName(address, chainId);
        return resolved?.type !== 'address' ? resolved.label : null;
    }



    // Decode function selector from 4byte.directory (simple approach like reference)
    async function decodeSelector(selector) {
        if (selectorCache.has(selector)) {
            return selectorCache.get(selector);
        }

        try {
            const response = await fetch(`https://www.4byte.directory/api/v1/signatures/?hex_signature=${selector}`);
            if (!response.ok) {
                selectorCache.set(selector, []);
                return [];
            }

            const data = await response.json();
            const signatures = data.results
                .sort((a, b) => a.id - b.id)
                .map(r => r.text_signature);
            selectorCache.set(selector, signatures);
            return signatures;
        } catch {
            selectorCache.set(selector, []);
            return [];
        }
    }

    function parseSignature(sig) {
        const firstParen = sig.indexOf('(');
        if (firstParen === -1) return { name: sig, params: [] };

        const name = sig.slice(0, firstParen);
        let depth = 0;
        let paramsEnd = -1;

        for (let i = firstParen; i < sig.length; i++) {
            if (sig[i] === '(') depth++;
            else if (sig[i] === ')') {
                depth--;
                if (depth === 0) {
                    paramsEnd = i;
                    break;
                }
            }
        }

        if (paramsEnd === -1) return { name, params: [] };

        const paramsStr = sig.slice(firstParen + 1, paramsEnd);
        if (!paramsStr) return { name, params: [] };

        const params = [];
        depth = 0;
        let current = '';

        for (const char of paramsStr) {
            if (char === '(' || char === '[') depth++;
            else if (char === ')' || char === ']') depth--;

            if (char === ',' && depth === 0) {
                params.push(current.trim());
                current = '';
            } else {
                current += char;
            }
        }
        if (current.trim()) params.push(current.trim());

        return { name, params };
    }

    function isDynamicType(paramType) {
        if (paramType.endsWith('[]')) return true;
        const fixedArrayMatch = paramType.match(/^(.+)\[(\d+)\]$/);
        if (fixedArrayMatch) return isDynamicType(fixedArrayMatch[1]);
        if (paramType === 'bytes' || paramType === 'string') return true;
        if (paramType.startsWith('(')) {
            const inner = paramType.slice(1, -1);
            let depth = 0;
            let current = '';
            for (const char of inner) {
                if (char === '(' || char === '[') depth++;
                else if (char === ')' || char === ']') depth--;
                if (char === ',' && depth === 0) {
                    if (isDynamicType(current.trim())) return true;
                    current = '';
                } else {
                    current += char;
                }
            }
            if (current.trim() && isDynamicType(current.trim())) return true;
        }
        return false;
    }

    // Decode function parameters from calldata
    function decodeParams(input, paramTypes) {
        if (!input || input.length <= 10 || paramTypes.length === 0) return [];

        const data = input.slice(10);
        const decoded = [];
        let offset = 0;

        for (const paramType of paramTypes) {
            if (offset >= data.length) break;

            const chunk = data.slice(offset, offset + 64);
            if (!chunk) break;

            let value;
            let rawValue;
            let isDynamic = false;

            if (isDynamicType(paramType)) {
                isDynamic = true;
                const dataOffset = parseInt(chunk, 16);

                if (paramType === 'bytes' || paramType === 'string') {
                    try {
                        const lenPos = dataOffset * 2;
                        const len = parseInt(data.slice(lenPos, lenPos + 64), 16);
                        const content = data.slice(lenPos + 64, lenPos + 64 + len * 2);
                        rawValue = '0x' + content;
                        if (paramType === 'string') {
                            // Try to decode as UTF-8
                            let str = '';
                            for (let i = 0; i < content.length; i += 2) {
                                const code = parseInt(content.slice(i, i + 2), 16);
                                if (code >= 32 && code < 127) str += String.fromCharCode(code);
                                else str += '.';
                            }
                            value = `"${str}"`;
                        } else {
                            value = rawValue.length > 66 ? rawValue.slice(0, 34) + '...' + rawValue.slice(-32) : rawValue;
                        }
                    } catch {
                        value = '[bytes]';
                        rawValue = '0x' + chunk;
                    }
                } else if (paramType.endsWith('[]')) {
                    try {
                        const lenPos = dataOffset * 2;
                        const len = parseInt(data.slice(lenPos, lenPos + 64), 16);
                        value = `[${len} items]`;
                        rawValue = `array length: ${len}`;
                    } catch {
                        value = '[array]';
                        rawValue = '0x' + chunk;
                    }
                } else {
                    value = '[dynamic]';
                    rawValue = '0x' + chunk;
                }
            } else if (paramType === 'address') {
                value = '0x' + chunk.slice(24).toLowerCase();
                rawValue = value;
            } else if (paramType.startsWith('uint')) {
                const bn = BigInt('0x' + (chunk || '0'));
                rawValue = bn.toString();
                // Format nicely with commas
                value = bn.toLocaleString();
            } else if (paramType.startsWith('int')) {
                let bn = BigInt('0x' + (chunk || '0'));
                const bits = parseInt(paramType.replace('int', '') || '256');
                const maxPos = (1n << BigInt(bits - 1)) - 1n;
                if (bn > maxPos) bn = bn - (1n << BigInt(bits));
                rawValue = bn.toString();
                value = bn.toLocaleString();
            } else if (paramType === 'bool') {
                value = chunk.endsWith('1') ? 'true' : 'false';
                rawValue = value;
            } else if (paramType === 'bytes32') {
                rawValue = '0x' + chunk;
                // Try to show as ASCII if printable
                let ascii = '';
                let isPrintable = true;
                for (let i = 0; i < 64 && isPrintable; i += 2) {
                    const code = parseInt(chunk.slice(i, i + 2), 16);
                    if (code === 0) break;
                    if (code >= 32 && code < 127) ascii += String.fromCharCode(code);
                    else isPrintable = false;
                }
                value = isPrintable && ascii.length > 0 ? `"${ascii}"` : rawValue;
            } else if (/^bytes([1-9]|[12][0-9]|3[01])$/.test(paramType)) {
                const size = parseInt(paramType.replace('bytes', ''));
                value = '0x' + chunk.slice(0, size * 2);
                rawValue = value;
            } else {
                value = '0x' + chunk;
                rawValue = value;
            }

            decoded.push({ type: paramType, value, rawValue: rawValue || value, isDynamic });
            offset += 64;
        }

        return decoded;
    }

    function formatValue(value) {
        if (!value || value === '0x0' || value === '0x') return null;
        const wei = BigInt(value);
        if (wei === 0n) return null;
        const eth = Number(wei) / 1e18;
        if (eth < 0.0001) return `${wei} wei`;
        return `${eth.toFixed(6)} ${getNativeToken()}`;
    }

    function formatGas(gas) {
        if (!gas) return '';
        const num = typeof gas === 'string' && gas.startsWith('0x')
            ? parseInt(gas, 16)
            : parseInt(gas);
        return num.toLocaleString();
    }

    function createCopyIcon(textToCopy) {
        const icon = document.createElement('span');
        icon.className = 'copy-icon';
        icon.innerHTML = '&#x2398;';
        icon.title = 'Copy';
        icon.addEventListener('click', async (e) => {
            e.stopPropagation();
            e.preventDefault();
            try {
                await navigator.clipboard.writeText(textToCopy);
                icon.classList.add('copied');
                icon.innerHTML = '&#x2713;';
                setTimeout(() => {
                    icon.classList.remove('copied');
                    icon.innerHTML = '&#x2398;';
                }, 1500);
            } catch {}
        });
        return icon;
    }

    async function renderCall(call, depth, parentGuides = []) {
        const node = document.createElement('div');
        node.className = 'call-node';

        const hasChildren = call.calls && call.calls.length > 0;
        const isRoot = depth === 0;
        const callType = call.type || call.callType || 'CALL';

        const row = document.createElement('div');
        row.className = 'call-row';

        // Indent guides
        const indent = document.createElement('div');
        indent.className = 'call-indent';

        for (const guide of parentGuides) {
            const guideEl = document.createElement('div');
            guideEl.className = `indent-guide ${guide}`;
            indent.appendChild(guideEl);
        }

        // Toggle
        const toggle = document.createElement('div');
        toggle.className = `call-toggle ${isRoot ? 'root' : ''} ${!hasChildren ? 'no-children' : ''}`;

        const toggleBtn = document.createElement('div');
        toggleBtn.className = 'toggle-btn';
        toggleBtn.textContent = hasChildren ? '−' : '';
        toggle.appendChild(toggleBtn);

        indent.appendChild(toggle);
        row.appendChild(indent);

        // Content
        const content = document.createElement('div');
        content.className = 'call-content';

        // Type badge
        const typeBadge = document.createElement('span');
        typeBadge.className = `call-type ${callType}`;
        typeBadge.textContent = callType;
        content.appendChild(typeBadge);

        // Call info
        const info = document.createElement('div');
        info.className = 'call-info';

        const mainLine = document.createElement('div');
        mainLine.className = 'call-main';

        // Resolve target address: system → token → contract → shortened
        const toAddr = call.to;
        const isTip20 = toAddr && isTip20Address(toAddr);
        const resolved = toAddr ? await resolveAddressName(toAddr, currentChainId) : null;
        const abiMap = resolved?.abi ? buildAbiFunctionMap(resolved.abi) : null;

        if (toAddr) {
            // TIP-20 precompile badge
            if (isTip20) {
                const tip20Badge = document.createElement('span');
                tip20Badge.className = 'call-type TIP20';
                tip20Badge.textContent = 'TIP-20';
                tip20Badge.title = 'Tempo TIP-20 native token precompile';
                content.insertBefore(tip20Badge, content.firstChild);
            }

            const explorerUrl = getExplorerAddrUrl(currentChainId, toAddr);

            const contractSpan = document.createElement('span');
            contractSpan.className = `contract-name addr-type-${resolved.type}`;

            if (explorerUrl) {
                const link = document.createElement('a');
                link.href = explorerUrl;
                link.target = '_blank';
                link.rel = 'noopener noreferrer';
                link.className = 'addr-link';
                link.textContent = resolved.label;
                link.title = resolved.tooltip;
                contractSpan.appendChild(link);
            } else {
                contractSpan.textContent = resolved.label;
                contractSpan.title = resolved.tooltip;
            }

            mainLine.appendChild(contractSpan);
            mainLine.appendChild(createCopyIcon(toAddr));
        } else if (callType === 'CREATE' || callType === 'CREATE2') {
            const contractSpan = document.createElement('span');
            contractSpan.className = 'contract-name';
            contractSpan.textContent = '[CREATE]';
            mainLine.appendChild(contractSpan);
        }

        // Function signature
        const input = call.input;
        if (input && input.length >= 10) {
            const selector = input.slice(0, 10);
            const signatures = await decodeSelector(selector);

            const sep = document.createElement('span');
            sep.className = 'call-separator';
            sep.textContent = '.';
            mainLine.appendChild(sep);

            const funcSpan = document.createElement('span');
            funcSpan.className = 'function-name';

            if (signatures.length > 0) {
                const parsed = parseSignature(signatures[0]);
                // Try to get parameter names from ABI, fall back to known functions
                const abiInfo = abiMap ? findInAbi(abiMap, signatures[0]) : null;
                const knownNames = getKnownParamNames(signatures[0]);
                const paramNames = abiInfo ? abiInfo.inputs.map(i => i.name) : (knownNames || []);

                const sigTypes = parsed.params.length > 0 ? `(${parsed.params.join(', ')})` : '()';
                funcSpan.textContent = `${parsed.name}${sigTypes}`;
                mainLine.appendChild(funcSpan);

                // Decoded params
                const decoded = decodeParams(input, parsed.params);
                // Add parameter names to decoded params
                for (let i = 0; i < decoded.length; i++) {
                    decoded[i].name = paramNames[i] || `arg${i}`;
                }
                if (decoded.length > 0) {
                    const paramsSpan = document.createElement('span');
                    paramsSpan.className = 'call-params';
                    paramsSpan.appendChild(document.createTextNode(' → '));

                    for (let i = 0; i < decoded.length; i++) {
                        const d = decoded[i];
                        const isAddr = d.type === 'address';
                        const isNum = d.type.startsWith('uint') || d.type.startsWith('int');

                        const valueSpan = document.createElement('span');
                        valueSpan.className = 'param-value';
                        valueSpan.title = d.value;

                        if (isAddr) {
                            valueSpan.classList.add('address');
                            const addrName = await fetchContractName(d.value, currentChainId);
                            if (addrName) {
                                valueSpan.textContent = addrName;
                                valueSpan.classList.add('named');
                            } else {
                                valueSpan.textContent = d.value;
                            }
                            paramsSpan.appendChild(valueSpan);
                            paramsSpan.appendChild(createCopyIcon(d.value));
                        } else if (isNum) {
                            valueSpan.classList.add('number');
                            valueSpan.textContent = d.value;
                            paramsSpan.appendChild(valueSpan);
                        } else if (d.isDynamic) {
                            valueSpan.classList.add('dynamic');
                            valueSpan.textContent = d.value;
                            paramsSpan.appendChild(valueSpan);
                        } else {
                            valueSpan.textContent = d.value;
                            paramsSpan.appendChild(valueSpan);
                        }

                        if (i < decoded.length - 1) {
                            paramsSpan.appendChild(document.createTextNode(', '));
                        }
                    }
                    mainLine.appendChild(paramsSpan);
                }
            } else {
                funcSpan.textContent = selector;
                funcSpan.style.color = 'var(--text-muted)';
                mainLine.appendChild(funcSpan);
            }
        } else {
            const sep = document.createElement('span');
            sep.className = 'call-separator';
            sep.textContent = '.';
            mainLine.appendChild(sep);

            const funcSpan = document.createElement('span');
            funcSpan.className = 'function-name';
            funcSpan.textContent = '(receive)';
            funcSpan.style.opacity = '0.6';
            mainLine.appendChild(funcSpan);
        }

        info.appendChild(mainLine);
        content.appendChild(info);

        // Meta (gas, value, error)
        const meta = document.createElement('div');
        meta.className = 'call-meta';

        const formattedValue = formatValue(call.value);
        if (formattedValue) {
            const valueSpan = document.createElement('span');
            valueSpan.className = 'call-value';
            valueSpan.textContent = formattedValue;
            meta.appendChild(valueSpan);
        }

        const gasUsed = call.gasUsed || call.gas_used;
        if (gasUsed) {
            const gasSpan = document.createElement('span');
            gasSpan.className = 'call-gas';
            gasSpan.textContent = `${formatGas(gasUsed)} gas`;
            meta.appendChild(gasSpan);
        }

        if (call.error) {
            const errorBadge = document.createElement('span');
            errorBadge.className = 'call-error-badge';
            errorBadge.textContent = call.revertReason || call.error;
            meta.appendChild(errorBadge);
        }

        content.appendChild(meta);
        row.appendChild(content);
        node.appendChild(row);

        // Details panel
        let detailsVisible = false;
        const details = document.createElement('div');
        details.className = 'call-details';
        details.style.display = 'none';
        details.style.setProperty('--indent-width', depth);

        // Decoded function inputs
        if (input && input.length >= 10) {
            const selector = input.slice(0, 10);
            const sigs = await decodeSelector(selector);

            if (sigs.length > 0) {
                const parsed = parseSignature(sigs[0]);
                const decoded = decodeParams(input, parsed.params);
                // Get parameter names from ABI, fall back to known functions
                const abiInfo = abiMap ? findInAbi(abiMap, sigs[0]) : null;
                const knownNames = getKnownParamNames(sigs[0]);
                const paramNames = abiInfo ? abiInfo.inputs.map(i => i.name) : (knownNames || []);

                if (decoded.length > 0) {
                    const decodedSection = document.createElement('div');
                    decodedSection.className = 'decoded-inputs';

                    const title = document.createElement('div');
                    title.className = 'decoded-title';
                    title.textContent = 'Decoded Inputs';
                    decodedSection.appendChild(title);

                    const table = document.createElement('div');
                    table.className = 'decoded-table';

                    for (let i = 0; i < decoded.length; i++) {
                        const d = decoded[i];
                        const paramName = paramNames[i] || `arg${i}`;
                        const paramType = parsed.params[i] || 'unknown';

                        const row = document.createElement('div');
                        row.className = 'decoded-row';

                        const nameCell = document.createElement('span');
                        nameCell.className = 'decoded-name';
                        nameCell.textContent = paramName;
                        row.appendChild(nameCell);

                        const typeCell = document.createElement('span');
                        typeCell.className = 'decoded-type';
                        typeCell.textContent = paramType;
                        row.appendChild(typeCell);

                        const valueCell = document.createElement('div');
                        valueCell.className = 'decoded-value';

                        if (d.type === 'address') {
                            valueCell.classList.add('address');
                            const addrName = await fetchContractName(d.value, currentChainId);
                            if (addrName) {
                                const namedSpan = document.createElement('span');
                                namedSpan.className = 'named';
                                namedSpan.textContent = addrName;
                                valueCell.appendChild(namedSpan);
                                valueCell.appendChild(document.createElement('br'));
                            }
                            const addrSpan = document.createElement('span');
                            addrSpan.className = 'raw-address';
                            addrSpan.textContent = d.value;
                            valueCell.appendChild(addrSpan);
                        } else if (d.type.startsWith('uint') || d.type.startsWith('int')) {
                            // Show formatted value and raw
                            const formattedSpan = document.createElement('span');
                            formattedSpan.className = 'formatted-number';
                            formattedSpan.textContent = d.value;
                            valueCell.appendChild(formattedSpan);
                            if (d.rawValue !== d.value) {
                                const rawSpan = document.createElement('span');
                                rawSpan.className = 'raw-number';
                                rawSpan.textContent = ` (${d.rawValue})`;
                                valueCell.appendChild(rawSpan);
                            }
                        } else {
                            valueCell.textContent = d.value;
                            if (d.rawValue && d.rawValue !== d.value) {
                                const rawSpan = document.createElement('div');
                                rawSpan.className = 'raw-value';
                                rawSpan.textContent = d.rawValue;
                                valueCell.appendChild(rawSpan);
                            }
                        }

                        row.appendChild(valueCell);
                        row.appendChild(createCopyIcon(d.rawValue || d.value));

                        table.appendChild(row);
                    }

                    decodedSection.appendChild(table);
                    details.appendChild(decodedSection);
                }
            }
        }

        // Raw input data
        if (input && input.length > 2) {
            const wrapper = document.createElement('div');
            wrapper.className = 'calldata-wrapper';
            const label = document.createElement('span');
            label.className = 'calldata-label';
            label.textContent = 'Raw Input:';
            wrapper.appendChild(label);
            const value = document.createElement('span');
            value.className = 'calldata-value';
            value.textContent = input;
            wrapper.appendChild(value);
            wrapper.appendChild(createCopyIcon(input));
            details.appendChild(wrapper);
        }

        const output = call.output;
        if (output && output !== '0x') {
            const returnDiv = document.createElement('div');
            returnDiv.className = 'return-data';
            const label = document.createElement('span');
            label.className = 'return-label';
            label.textContent = 'Output:';
            returnDiv.appendChild(label);
            const value = document.createElement('span');
            value.className = 'return-value';
            value.textContent = output;
            returnDiv.appendChild(value);
            returnDiv.appendChild(createCopyIcon(output));
            details.appendChild(returnDiv);
        }

        if (call.error) {
            const errorDiv = document.createElement('div');
            errorDiv.className = 'error-message';
            errorDiv.textContent = call.revertReason || call.error;
            details.appendChild(errorDiv);
        }

        if (details.children.length > 0) {
            node.appendChild(details);
        }

        row.addEventListener('click', (e) => {
            if (e.target.closest('.toggle-btn') || e.target.closest('.copy-icon')) return;
            if (details.children.length > 0) {
                detailsVisible = !detailsVisible;
                details.style.display = detailsVisible ? 'block' : 'none';
            }
        });

        // Children
        if (hasChildren) {
            const childrenContainer = document.createElement('div');
            childrenContainer.className = 'call-children';

            for (let i = 0; i < call.calls.length; i++) {
                const isLast = i === call.calls.length - 1;
                const childGuides = [...parentGuides];
                if (!isRoot) childGuides.push(isLast ? 'empty' : 'line');
                const childNode = await renderCall(call.calls[i], depth + 1, childGuides);
                childrenContainer.appendChild(childNode);
            }

            node.appendChild(childrenContainer);

            toggleBtn.addEventListener('click', (e) => {
                e.stopPropagation();
                const isCollapsed = childrenContainer.classList.toggle('collapsed');
                toggleBtn.textContent = isCollapsed ? '+' : '−';
            });
        }

        return node;
    }

    // Parse balance changes from simulation logs
    function parseBalanceChanges(simulation, overrideFrom, overrideTo) {
        const trace = simulation.enhancedTrace || (simulation.trace && simulation.trace[0]);
        const txFrom = (overrideFrom || simulation.txFrom || trace?.from || '').toLowerCase();
        const txTo = (overrideTo || simulation.txTo || trace?.to || '').toLowerCase();

        // Track balance changes per address per token
        // Key: `${address}:${tokenAddress}` (tokenAddress = 'native' for native token)
        const changes = new Map();

        // Helper to add/update a change
        const addChange = (address, tokenAddress, amount, direction) => {
            const key = `${address.toLowerCase()}:${tokenAddress.toLowerCase()}`;
            if (!changes.has(key)) {
                changes.set(key, { address: address.toLowerCase(), tokenAddress: tokenAddress.toLowerCase(), amount: 0n });
            }
            const change = changes.get(key);
            change.amount += direction === 'in' ? BigInt(amount) : -BigInt(amount);
        };

        // Native token transfer
        const traceValue = trace?.value;
        if (traceValue && traceValue !== '0' && traceValue !== '0x0' && traceValue !== '0x') {
            try {
                const value = BigInt(traceValue);
                if (value > 0n) {
                    addChange(txFrom, 'native', value, 'out');
                    if (txTo) addChange(txTo, 'native', value, 'in');
                }
            } catch {}
        }

        // ERC20 Transfer events
        const transferTopic = '0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef';
        // ERC721 Transfer has same topic but different data structure

        if (simulation.logs && simulation.logs.length > 0) {
            for (const log of simulation.logs) {
                const topics = log.topics || [];
                if (topics[0] !== transferTopic) continue;
                if (topics.length < 3) continue; // Need from, to in topics

                const tokenAddress = log.address?.toLowerCase();
                if (!tokenAddress) continue;

                // ERC20: Transfer(address indexed from, address indexed to, uint256 value)
                // topics[1] = from, topics[2] = to, data = value
                const from = '0x' + (topics[1] || '').slice(-40).toLowerCase();
                const to = '0x' + (topics[2] || '').slice(-40).toLowerCase();

                // Get amount from data (for ERC20) or topics[3] (for ERC721)
                let amount;
                try {
                    if (log.data && log.data !== '0x' && log.data.length > 2) {
                        amount = BigInt(log.data);
                    } else if (topics[3]) {
                        // ERC721 - tokenId in topics[3], treat as amount of 1
                        amount = 1n;
                    } else {
                        amount = 0n;
                    }
                } catch {
                    amount = 0n;
                }

                if (amount > 0n) {
                    addChange(from, tokenAddress, amount, 'out');
                    addChange(to, tokenAddress, amount, 'in');
                }
            }
        }

        // Filter to only show changes for addresses involved in the tx (from/to)
        // and convert to array sorted by relevance
        const relevantAddresses = new Set([txFrom, txTo].filter(Boolean));
        const result = [];

        for (const [key, change] of changes) {
            if (change.amount === 0n) continue;
            if (!relevantAddresses.has(change.address)) continue;
            result.push(change);
        }

        // Sort: tx sender first, then by absolute amount (descending)
        result.sort((a, b) => {
            if (a.address === txFrom && b.address !== txFrom) return -1;
            if (b.address === txFrom && a.address !== txFrom) return 1;
            const absA = a.amount < 0n ? -a.amount : a.amount;
            const absB = b.amount < 0n ? -b.amount : b.amount;
            return absB > absA ? 1 : -1;
        });

        return { changes: result, txFrom, txTo };
    }

    async function renderBalanceChanges(simulation, overrideFrom, overrideTo) {
        const { changes, txFrom } = parseBalanceChanges(simulation, overrideFrom, overrideTo);

        if (changes.length === 0) {
            return null;
        }

        // Pre-fetch all token metadata in parallel
        const tokenAddresses = changes
            .filter(c => c.tokenAddress !== 'native')
            .map(c => c.tokenAddress);
        await Promise.all(tokenAddresses.map(addr => fetchTokenMetadata(addr, currentChainId)));

        const container = document.createElement('div');
        container.className = 'balance-changes';

        const header = document.createElement('div');
        header.className = 'balance-changes-header';

        const title = document.createElement('div');
        title.className = 'balance-changes-title';
        title.textContent = 'Balance Changes';
        header.appendChild(title);

        // Show whose balance we're showing
        const subtitle = document.createElement('div');
        subtitle.className = 'balance-changes-subtitle';
        const txFromResolved = txFrom ? await resolveAddressName(txFrom, currentChainId) : null;
        subtitle.textContent = txFromResolved ? `for ${txFromResolved.type !== 'address' ? txFromResolved.label : shortenAddress(txFrom)}` : '';
        header.appendChild(subtitle);

        container.appendChild(header);

        const grid = document.createElement('div');
        grid.className = 'balance-changes-grid';

        // Render each balance change (metadata already cached)
        for (const change of changes) {
            const item = document.createElement('div');
            item.className = 'balance-change-item';

            const isPositive = change.amount > 0n;
            const isNative = change.tokenAddress === 'native';

            // Token icon
            const iconWrapper = document.createElement('div');
            iconWrapper.className = 'balance-change-icon-wrapper';

            if (isNative) {
                const nativeConfig = getNativeTokenConfig();
                if (nativeConfig.logo) {
                    const img = document.createElement('img');
                    img.src = nativeConfig.logo;
                    img.alt = nativeConfig.symbol;
                    img.className = 'token-logo';
                    img.onerror = () => { img.style.display = 'none'; iconWrapper.textContent = 'Ξ'; };
                    iconWrapper.appendChild(img);
                } else {
                    iconWrapper.className = 'balance-change-icon';
                    iconWrapper.textContent = 'Ξ';
                }
            } else {
                // Get from cache (already fetched above)
                const metadata = await fetchTokenMetadata(change.tokenAddress, currentChainId);
                if (metadata.logo) {
                    const img = document.createElement('img');
                    img.src = metadata.logo;
                    img.alt = metadata.symbol;
                    img.className = 'token-logo';
                    img.onerror = () => {
                        img.style.display = 'none';
                        const fallback = document.createElement('div');
                        fallback.className = 'token-logo-fallback';
                        fallback.textContent = metadata.symbol.slice(0, 2);
                        iconWrapper.appendChild(fallback);
                    };
                    iconWrapper.appendChild(img);
                } else {
                    const fallback = document.createElement('div');
                    fallback.className = 'token-logo-fallback';
                    fallback.textContent = metadata.symbol.slice(0, 2);
                    iconWrapper.appendChild(fallback);
                }
            }
            item.appendChild(iconWrapper);

            // Token info
            const info = document.createElement('div');
            info.className = 'balance-change-info';

            const tokenName = document.createElement('div');
            tokenName.className = 'balance-change-token';

            if (isNative) {
                tokenName.textContent = getNativeToken();
            } else {
                const metadata = await fetchTokenMetadata(change.tokenAddress, currentChainId);
                tokenName.textContent = metadata.symbol;
            }
            info.appendChild(tokenName);

            // Show token address for non-native tokens
            if (!isNative) {
                const addr = document.createElement('div');
                addr.className = 'balance-change-address';
                addr.textContent = change.tokenAddress.slice(0, 10) + '...' + change.tokenAddress.slice(-4);
                addr.title = change.tokenAddress;
                info.appendChild(addr);
            }

            item.appendChild(info);

            // Amount with direction indicator
            const amountWrapper = document.createElement('div');
            amountWrapper.className = 'balance-change-amount-wrapper';

            const direction = document.createElement('div');
            direction.className = `balance-change-direction ${isPositive ? 'positive' : 'negative'}`;
            direction.textContent = isPositive ? '↓ IN' : '↑ OUT';
            amountWrapper.appendChild(direction);

            const amount = document.createElement('div');
            amount.className = `balance-change-amount ${isPositive ? 'positive' : 'negative'}`;

            const absAmount = change.amount < 0n ? -change.amount : change.amount;

            if (isNative) {
                const nativeConfig = getNativeTokenConfig();
                const formatted = formatTokenAmount(absAmount.toString(), nativeConfig.decimals);
                amount.textContent = `${isPositive ? '+' : '-'}${formatted} ${nativeConfig.symbol}`;
            } else {
                const metadata = await fetchTokenMetadata(change.tokenAddress, currentChainId);
                const formatted = formatTokenAmount(absAmount.toString(), metadata.decimals);
                amount.textContent = `${isPositive ? '+' : '-'}${formatted} ${metadata.symbol}`;
            }
            amountWrapper.appendChild(amount);

            item.appendChild(amountWrapper);

            grid.appendChild(item);
        }

        container.appendChild(grid);
        return container;
    }

    // Collect all unique addresses and selectors from trace for pre-fetching
    function collectTraceData(trace, addresses = new Set(), selectors = new Set()) {
        if (!trace) return { addresses, selectors };

        if (trace.from) addresses.add(trace.from.toLowerCase());
        if (trace.to) addresses.add(trace.to.toLowerCase());

        if (trace.input && trace.input.length >= 10) {
            selectors.add(trace.input.slice(0, 10));
        }

        if (trace.calls) {
            for (const call of trace.calls) {
                collectTraceData(call, addresses, selectors);
            }
        }

        return { addresses, selectors };
    }

    // Pre-fetch all contract info and selectors in parallel
    async function prefetchTraceData(trace, chainId) {
        const { addresses, selectors } = collectTraceData(trace);
        console.log(`Prefetching ${addresses.length} contracts, ${selectors.length} selectors`);

        const startTime = performance.now();

        // Fetch all in parallel
        const promises = [];

        for (const addr of addresses) {
            promises.push(fetchContractInfo(addr, chainId).catch(() => null));
        }

        for (const sel of selectors) {
            promises.push(decodeSelector(sel).catch(() => []));
        }

        await Promise.all(promises);

        console.log(`Prefetch completed in ${(performance.now() - startTime).toFixed(0)}ms`);
    }

    async function loadSimulation(simulationId) {
        const app = document.getElementById('app');
        app.className = 'container';

        app.innerHTML = `
            <div class="loading-container">
                <div class="loading-spinner"></div>
                <div>Loading simulation<span class="loading-inline"><span class="loading-dot"></span><span class="loading-dot"></span><span class="loading-dot"></span></span></div>
            </div>
        `;

        try {
            console.time('API: fetch simulation');
            const response = await fetch(`/api/v1/simulation/${simulationId}`);
            console.timeEnd('API: fetch simulation');
            if (!response.ok) {
                throw new Error(`Simulation not found (${response.status})`);
            }

            const simulation = await response.json();
            currentChainId = simulation.chainId || 1;

            app.innerHTML = '';

            // Get trace for display
            const trace = simulation.enhancedTrace || (simulation.trace && simulation.trace[0]);
            const chainName = getChainName(currentChainId);
            const chainIcon = getChainIcon(currentChainId);
            const isTempoTx = simulation.isTempoTx || isTempoChain(currentChainId);
            const isBatchTx = simulation.transactionType === 'batch' || (simulation.callResults && simulation.callResults.length > 0);
            const feeToken = simulation.feeToken;
            const feeTokenResolved = feeToken ? await resolveAddressName(feeToken, currentChainId) : null;

            // Use txFrom/txTo from receipt (correct on Tempo), fall back to trace
            const senderAddr = simulation.txFrom || trace?.from;
            const recipientAddr = simulation.txTo || trace?.to;
            const [senderResolved, recipientResolved] = await Promise.all([
                senderAddr ? resolveAddressName(senderAddr, currentChainId) : null,
                recipientAddr ? resolveAddressName(recipientAddr, currentChainId) : null,
            ]);

            // Tenderly-style header
            const header = document.createElement('div');
            header.className = 'tenderly-header';

            // Build network badge — show Tempo badge for Tempo chains
            let networkBadgeHtml = '';
            if (isTempoTx) {
                networkBadgeHtml = `<div class="tenderly-network tempo-network">
                    <span class="tempo-wordmark">TEMPO</span>
                    <span class="tempo-network-name">${chainName}</span>
                </div>`;
            } else {
                networkBadgeHtml = `<div class="tenderly-network">
                    ${chainIcon ? `<img src="${chainIcon}" class="chain-icon" alt="${chainName}">` : ''}
                    <span>${chainName}</span>
                </div>`;
            }

            // Build transaction type badges
            let txTypeBadges = '';
            if (isBatchTx) {
                txTypeBadges += `<span class="tx-type-badge batch">Batch (0x76)</span>`;
            }
            if (feeToken && isTip20Address(feeToken)) {
                txTypeBadges += `<span class="tx-type-badge sponsored">Fee Sponsored</span>`;
            }

            header.innerHTML = `
                <div class="tenderly-header-top">
                    <div class="tenderly-title">
                        <span class="tenderly-sim-label">Simulation</span>
                        <span class="tenderly-sim-id">${simulationId}</span>
                        ${txTypeBadges}
                    </div>
                    ${networkBadgeHtml}
                </div>
            `;
            app.appendChild(header);

            // Main summary card (Tenderly-style)
            const summaryCard = document.createElement('div');
            summaryCard.className = 'tenderly-summary';

            // Status badge
            const statusBadge = simulation.success
                ? '<span class="status-badge success">Success</span>'
                : '<span class="status-badge failed">Failed</span>';

            // Decode function call if possible
            let functionDisplay = '';
            if (trace && trace.input && trace.input !== '0x' && trace.input.length >= 10) {
                const selector = trace.input.slice(0, 10);
                const sigs = await decodeSelector(selector);
                const sig = sigs && sigs[0];
                if (sig) {
                    const parsed = parseSignature(sig);
                    const knownNames = getKnownParamNames(sig);
                    const paramTypes = parsed.params;
                    const rawParams = decodeParams(trace.input, paramTypes);
                    const params = rawParams.map((v, i) => ({
                        name: knownNames[i] || `arg${i}`,
                        value: v,
                    }));
                    functionDisplay = `
                        <div class="summary-row function-row">
                            <span class="summary-label">Function</span>
                            <div class="function-call">
                                <span class="function-name">${parsed.name}</span>
                                <span class="function-params">(${params.map(p =>
                                    `<span class="param"><span class="param-name">${p.name}</span>: <span class="param-value" title="${p.value}">${truncateValue(p.value)}</span></span>`
                                ).join(', ')})</span>
                            </div>
                        </div>
                    `;
                } else {
                    functionDisplay = `
                        <div class="summary-row">
                            <span class="summary-label">Function</span>
                            <span class="summary-value mono">${selector}</span>
                        </div>
                    `;
                }
            }

            // Revert reason if failed
            let revertDisplay = '';
            if (!simulation.success) {
                const revertReason = extractRevertReason(trace);
                revertDisplay = `
                    <div class="revert-section">
                        <span class="revert-label">Revert Reason</span>
                        <span class="revert-message">${revertReason || 'No revert reason provided'}</span>
                    </div>
                `;
            }

            // Fee token row for Tempo sponsored transactions
            const feeTokenRow = feeToken ? `
                <div class="summary-row">
                    <span class="summary-label">Fee Token</span>
                    <span class="summary-value address addr-type-${feeTokenResolved.type}" title="${feeTokenResolved.tooltip}">
                        ${feeTokenResolved.label}
                    </span>
                    ${createCopyIcon(feeToken).outerHTML}
                </div>
            ` : '';

            // Batch call summary for 0x76 transactions
            const batchSummary = isBatchTx && simulation.callResults ? `
                <div class="summary-row">
                    <span class="summary-label">Batch Calls</span>
                    <span class="summary-value">${simulation.callResults.length} calls (${simulation.callResults.filter(r => r.success).length} succeeded)</span>
                </div>
            ` : '';

            // Derive action name from decoded function or tx type
            const actionName = functionDisplay
                ? (functionDisplay.match(/class="function-name">([^<]+)</) || [])[1]
                : (isBatchTx ? 'Batch' : null);

            summaryCard.innerHTML = `
                <div class="tx-overview">
                    <div class="tx-overview-logo">
                        ${isTempoTx ? '<span class="tx-tempo-wordmark">TEMPO</span>' : (chainIcon ? `<img src="${chainIcon}" class="tx-chain-icon" alt="${chainName}">` : `<span class="tx-chain-name">${chainName}</span>`)}
                    </div>
                    <div class="tx-overview-fields">
                        <div class="tx-field-row">
                            <span class="tx-field-label">Status</span>
                            <span class="tx-field-value">${statusBadge}</span>
                        </div>
                        <div class="tx-field-row">
                            <span class="tx-field-label">Block</span>
                            <span class="tx-field-value tx-field-link">#${simulation.blockNumber?.toLocaleString() || 'N/A'}</span>
                        </div>
                        <div class="tx-field-row">
                            <span class="tx-field-label">From</span>
                            <span class="tx-field-value tx-field-link addr-type-${senderResolved?.type || 'address'}" title="${senderResolved?.tooltip || ''}">${senderAddr ? (senderResolved?.type !== 'address' ? senderResolved.label : shortenAddress(senderAddr)) : 'N/A'}</span>
                            ${senderAddr ? createCopyIcon(senderAddr).outerHTML : ''}
                            ${senderAddr && getExplorerAddrUrl(currentChainId, senderAddr) ? `<a href="${getExplorerAddrUrl(currentChainId, senderAddr)}" target="_blank" rel="noopener noreferrer" class="explorer-link" title="View on explorer">↗</a>` : ''}
                        </div>
                        <div class="tx-field-row">
                            <span class="tx-field-label">To</span>
                            <span class="tx-field-value tx-field-link addr-type-${recipientResolved?.type || 'address'}" title="${recipientResolved?.tooltip || ''}">${recipientAddr ? (recipientResolved?.type !== 'address' ? recipientResolved.label : shortenAddress(recipientAddr)) : 'N/A'}</span>
                            ${recipientAddr ? createCopyIcon(recipientAddr).outerHTML : ''}
                            ${recipientAddr && getExplorerAddrUrl(currentChainId, recipientAddr) ? `<a href="${getExplorerAddrUrl(currentChainId, recipientAddr)}" target="_blank" rel="noopener noreferrer" class="explorer-link" title="View on explorer">↗</a>` : ''}
                        </div>
                        ${simulation.txHash ? `
                        <div class="tx-field-row">
                            <span class="tx-field-label">Hash</span>
                            <span class="tx-field-value mono" title="${simulation.txHash}">${simulation.txHash.slice(0,14)}…${simulation.txHash.slice(-8)}</span>
                            ${createCopyIcon(simulation.txHash).outerHTML}
                            ${getExplorerTxUrl(currentChainId, simulation.txHash) ? `<a href="${getExplorerTxUrl(currentChainId, simulation.txHash)}" target="_blank" rel="noopener noreferrer" class="explorer-link" title="View on explorer">↗</a>` : ''}
                        </div>
                        ` : ''}
                        <div class="tx-field-row">
                            <span class="tx-field-label">Gas Used</span>
                            <span class="tx-field-value">${simulation.gasUsed?.toLocaleString() || 'N/A'}</span>
                        </div>
                        <div class="tx-field-row">
                            <span class="tx-field-label">Value</span>
                            <span class="tx-field-value">${formatValue(trace?.value) || `0 ${getNativeToken()}`}</span>
                        </div>
                        ${feeTokenResolved ? `<div class="tx-field-row"><span class="tx-field-label">Fee Token</span><span class="tx-field-value addr-type-${feeTokenResolved.type}">${feeTokenResolved.label}</span></div>` : ''}
                    </div>
                </div>
                ${actionName ? `
                <div class="tx-actions-list">
                    <div class="tx-action-item">
                        <span class="tx-action-num">1.</span>
                        <span class="tx-action-badge">${actionName}</span>
                        ${!isBatchTx && recipientResolved ? `<span class="tx-action-target">${recipientResolved.label}</span>` : ''}
                        ${isBatchTx ? `<span class="tx-action-note">${simulation.callResults?.length || 0} sub-calls</span>` : ''}
                    </div>
                </div>` : ''}
                ${revertDisplay}
            `;
            app.appendChild(summaryCard);

            // Balance changes (use txFrom/txTo for correct sender on Tempo)
            const balanceChanges = await renderBalanceChanges(simulation, senderAddr, recipientAddr);
            if (balanceChanges) {
                app.appendChild(balanceChanges);
            }

            // Trace output
            const traceSection = document.createElement('section');
            traceSection.className = 'trace-output';

            const traceHeader = document.createElement('div');
            traceHeader.className = 'trace-header';
            traceHeader.innerHTML = `
                <h2>Transaction Trace</h2>
                <div class="trace-actions">
                    <button class="action-btn" id="expandAllBtn">Expand All</button>
                    <button class="action-btn" id="collapseAllBtn">Collapse All</button>
                    <button class="action-btn toggle-btn-debug" id="debugModeBtn">Debug Mode</button>
                    ${simulation.txHash ? `<button class="action-btn debugger-btn" id="openDebuggerBtn">Open Debugger</button>` : ''}
                </div>
            `;
            traceSection.appendChild(traceHeader);

            const traceTree = document.createElement('div');
            traceTree.className = 'trace-tree';

            // Pre-fetch all contract info and selectors in parallel for speed
            if (trace) {
                const prefetchMsg = document.createElement('div');
                prefetchMsg.className = 'loading-container';
                prefetchMsg.innerHTML = `
                    <div class="loading-spinner"></div>
                    <div>Decoding contracts<span class="loading-inline"><span class="loading-dot"></span><span class="loading-dot"></span><span class="loading-dot"></span></span></div>
                `;
                traceTree.appendChild(prefetchMsg);

                await prefetchTraceData(trace, currentChainId);
                prefetchMsg.remove();

                const traceNode = await renderCall(trace, 0, []);
                traceTree.appendChild(traceNode);
            } else {
                traceTree.innerHTML = '<div class="loading-container"><div>No trace data available</div></div>';
            }

            traceSection.appendChild(traceTree);
            app.appendChild(traceSection);

            // Footer
            const footer = document.createElement('footer');
            footer.innerHTML = `
                <p>Powered by <a href="https://github.com/EnsoFinance/temper" target="_blank">Temper</a> · Contract data from <a href="https://sourcify.dev" target="_blank">Sourcify</a> · Signatures from <a href="https://4byte.directory" target="_blank">4byte.directory</a></p>
            `;
            app.appendChild(footer);

            // Wire up buttons
            document.getElementById('expandAllBtn').addEventListener('click', () => {
                document.querySelectorAll('.call-children').forEach(el => el.classList.remove('collapsed'));
                document.querySelectorAll('.toggle-btn').forEach(el => {
                    if (el.textContent === '+') el.textContent = '−';
                });
            });

            document.getElementById('collapseAllBtn').addEventListener('click', () => {
                document.querySelectorAll('.call-children').forEach(el => el.classList.add('collapsed'));
                document.querySelectorAll('.toggle-btn').forEach(el => {
                    if (el.textContent === '−') el.textContent = '+';
                });
            });

            // Debug mode toggle - shows all details panels
            document.getElementById('debugModeBtn').addEventListener('click', (e) => {
                const btn = e.target;
                const isDebug = document.body.classList.toggle('debug-mode');
                btn.textContent = isDebug ? 'Debug Mode ✓' : 'Debug Mode';
                btn.classList.toggle('active', isDebug);

                // Show/hide all details panels
                document.querySelectorAll('.call-details').forEach(el => {
                    el.style.display = isDebug ? 'block' : 'none';
                });
            });

            // Open debugger button
            const openDebuggerBtn = document.getElementById('openDebuggerBtn');
            if (openDebuggerBtn && simulation.txHash) {
                openDebuggerBtn.addEventListener('click', () => {
                    window.location.href = `/debug?tx=${simulation.txHash}&chain=${currentChainId}&sim=${simulationId}`;
                });
            }

            // Jump to revert button (in revert banner)
            const jumpToRevertBtn = document.getElementById('jumpToRevertBtn');
            if (jumpToRevertBtn && simulation.txHash) {
                jumpToRevertBtn.addEventListener('click', () => {
                    window.location.href = `/debug?tx=${simulation.txHash}&chain=${currentChainId}&sim=${simulationId}&jump=revert`;
                });
            }

        } catch (error) {
            app.innerHTML = `
                <div class="error-container">
                    <div class="error-icon">⚠️</div>
                    <div class="error-title">Failed to load simulation</div>
                    <div class="error-message-text">${error.message}</div>
                </div>
            `;
        }
    }

    // Export for use in HTML
    window.loadSimulation = loadSimulation;
})();
