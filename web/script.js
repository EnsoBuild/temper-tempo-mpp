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

    // Native token symbols by chain ID
    const NATIVE_TOKENS = {
        1: 'ETH',
        10: 'ETH',
        56: 'BNB',
        100: 'xDAI',
        137: 'POL',
        250: 'FTM',
        999: 'HYPE',
        8453: 'ETH',
        42161: 'ETH',
        43114: 'AVAX',
    };

    function getNativeToken() {
        return NATIVE_TOKENS[currentChainId] || 'ETH';
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

    // Block explorer API URLs by chain ID
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
    };

    // Fetch contract info from Sourcify first, then Blockscout/Etherscan fallback
    async function fetchContractInfo(address, chainId) {
        const cacheKey = `${chainId}:${address.toLowerCase()}`;
        if (contractCache.has(cacheKey)) {
            return contractCache.get(cacheKey);
        }

        // Try Sourcify first (with short timeout)
        try {
            const controller = new AbortController();
            const timeout = setTimeout(() => controller.abort(), 1500);

            const response = await fetch(
                `https://sourcify.dev/server/files/any/${chainId}/${address}`,
                { signal: controller.signal }
            );
            clearTimeout(timeout);

            if (response.ok) {
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
                    const info = { name: contractName, abi };
                    contractCache.set(cacheKey, info);
                    cacheAbiSelectors(abi);
                    return info;
                }
            }
        } catch {
            // Sourcify failed, try block explorer
        }

        // Try block explorer API (Etherscan/Blockscout)
        const explorerApi = EXPLORER_APIS[chainId];
        if (explorerApi) {
            try {
                const controller = new AbortController();
                const timeout = setTimeout(() => controller.abort(), 2000);

                const response = await fetch(
                    `${explorerApi}?module=contract&action=getsourcecode&address=${address}`,
                    { signal: controller.signal }
                );
                clearTimeout(timeout);

                if (response.ok) {
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
                            const info = { name: contractName, abi };
                            contractCache.set(cacheKey, info);
                            cacheAbiSelectors(abi);
                            return info;
                        }
                    }
                }
            } catch {
                // Block explorer failed
            }
        }

        contractCache.set(cacheKey, null);
        return null;
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

    // Get just the contract name (convenience wrapper)
    async function fetchContractName(address, chainId) {
        const info = await fetchContractInfo(address, chainId);
        return info?.name || null;
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

        // Fetch contract info (name + ABI) for the target address
        const toAddr = call.to;
        const contractInfo = toAddr ? await fetchContractInfo(toAddr, currentChainId) : null;
        const toName = contractInfo?.name || null;
        const abiMap = contractInfo?.abi ? buildAbiFunctionMap(contractInfo.abi) : null;

        if (toAddr) {
            const contractSpan = document.createElement('span');
            contractSpan.className = 'contract-name';
            contractSpan.textContent = toName || toAddr;
            if (toName) contractSpan.title = toAddr;
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

    function renderBalanceChanges(simulation) {
        const container = document.createElement('div');
        container.className = 'balance-changes';

        const title = document.createElement('div');
        title.className = 'balance-changes-title';
        title.textContent = 'Balance Changes';
        container.appendChild(title);

        const grid = document.createElement('div');
        grid.className = 'balance-changes-grid';

        // Get value from trace if available
        const trace = simulation.enhancedTrace || (simulation.trace && simulation.trace[0]);
        const traceValue = trace?.value;
        const traceFrom = trace?.from;

        // Show native token value transfer if any
        if (traceValue && traceValue !== '0' && traceValue !== '0x0' && traceValue !== '0x') {
            const ethItem = document.createElement('div');
            ethItem.className = 'balance-change-item';

            const icon = document.createElement('div');
            icon.className = 'balance-change-icon';
            icon.textContent = 'Ξ';
            ethItem.appendChild(icon);

            const info = document.createElement('div');
            info.className = 'balance-change-info';
            const token = document.createElement('div');
            token.className = 'balance-change-token';
            token.textContent = getNativeToken();
            info.appendChild(token);
            const addr = document.createElement('div');
            addr.className = 'balance-change-address';
            addr.textContent = traceFrom ? traceFrom.slice(0, 10) + '...' : '';
            info.appendChild(addr);
            ethItem.appendChild(info);

            const amount = document.createElement('div');
            amount.className = 'balance-change-amount negative';
            try {
                const wei = BigInt(traceValue);
                const eth = Number(wei) / 1e18;
                amount.textContent = `-${eth.toFixed(6)} ${getNativeToken()}`;
            } catch {
                amount.textContent = traceValue;
            }
            ethItem.appendChild(amount);

            grid.appendChild(ethItem);
        }

        // Parse Transfer events from logs
        const transferTopic = '0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef';
        if (simulation.logs && simulation.logs.length > 0) {
            for (const log of simulation.logs) {
                const topics = log.topics || [];
                if (topics[0] === transferTopic) {
                    const item = document.createElement('div');
                    item.className = 'balance-change-item';

                    const icon = document.createElement('div');
                    icon.className = 'balance-change-icon';
                    icon.textContent = '🪙';
                    item.appendChild(icon);

                    const info = document.createElement('div');
                    info.className = 'balance-change-info';
                    const token = document.createElement('div');
                    token.className = 'balance-change-token';
                    token.textContent = 'ERC20';
                    info.appendChild(token);
                    const addr = document.createElement('div');
                    addr.className = 'balance-change-address';
                    addr.textContent = log.address ? log.address.slice(0, 10) + '...' : '';
                    info.appendChild(addr);
                    item.appendChild(info);

                    const amount = document.createElement('div');
                    amount.className = 'balance-change-amount positive';
                    amount.textContent = 'Transfer';
                    item.appendChild(amount);

                    grid.appendChild(item);
                }
            }
        }

        if (grid.children.length === 0) {
            return null;
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

        // Fetch all in parallel
        const promises = [];

        for (const addr of addresses) {
            promises.push(fetchContractInfo(addr, chainId).catch(() => null));
        }

        for (const sel of selectors) {
            promises.push(decodeSelector(sel).catch(() => []));
        }

        await Promise.all(promises);
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
            const response = await fetch(`/api/v1/simulation/${simulationId}`);
            if (!response.ok) {
                throw new Error(`Simulation not found (${response.status})`);
            }

            const simulation = await response.json();
            currentChainId = simulation.chainId || 1;

            app.innerHTML = '';

            // Header
            const header = document.createElement('header');
            header.innerHTML = `
                <div class="logo">
                    <div class="logo-icon">T</div>
                    <h1>Temper</h1>
                </div>
                <div class="simulation-id">${simulationId}</div>
            `;
            app.appendChild(header);

            // Get trace for display
            const trace = simulation.enhancedTrace || (simulation.trace && simulation.trace[0]);

            // Revert banner (if transaction reverted) - show prominently at top
            if (!simulation.success) {
                const revertReason = extractRevertReason(trace);
                const revertBanner = document.createElement('div');
                revertBanner.className = 'revert-banner';
                revertBanner.innerHTML = `
                    <div class="revert-icon">⚠️</div>
                    <div class="revert-content">
                        <div class="revert-title">Transaction Reverted</div>
                        <div class="revert-reason">${revertReason || 'No revert reason provided'}</div>
                    </div>
                    ${simulation.txHash ? `<button class="revert-debug-btn" id="jumpToRevertBtn">Debug Revert →</button>` : ''}
                `;
                app.appendChild(revertBanner);
            }

            // Transaction Overview panel
            if (trace) {
                const overview = document.createElement('div');
                overview.className = 'tx-overview';

                const overviewTitle = document.createElement('div');
                overviewTitle.className = 'tx-overview-title';
                overviewTitle.textContent = 'Transaction Overview';
                overview.appendChild(overviewTitle);

                const overviewGrid = document.createElement('div');
                overviewGrid.className = 'tx-overview-grid';

                // Transaction Hash (if available)
                if (simulation.txHash) {
                    const hashRow = document.createElement('div');
                    hashRow.className = 'tx-overview-row';
                    hashRow.innerHTML = `<span class="tx-overview-label">Tx Hash</span>`;
                    const hashValue = document.createElement('span');
                    hashValue.className = 'tx-overview-value mono';
                    hashValue.textContent = simulation.txHash;
                    hashRow.appendChild(hashValue);
                    hashRow.appendChild(createCopyIcon(simulation.txHash));
                    overviewGrid.appendChild(hashRow);
                }

                // From
                const fromRow = document.createElement('div');
                fromRow.className = 'tx-overview-row';
                fromRow.innerHTML = `<span class="tx-overview-label">From</span>`;
                const fromValue = document.createElement('span');
                fromValue.className = 'tx-overview-value address';
                fromValue.textContent = trace.from || 'N/A';
                fromRow.appendChild(fromValue);
                if (trace.from) fromRow.appendChild(createCopyIcon(trace.from));
                overviewGrid.appendChild(fromRow);

                // To
                const toRow = document.createElement('div');
                toRow.className = 'tx-overview-row';
                toRow.innerHTML = `<span class="tx-overview-label">To</span>`;
                const toValue = document.createElement('span');
                toValue.className = 'tx-overview-value address';
                toValue.textContent = trace.to || 'Contract Creation';
                toRow.appendChild(toValue);
                if (trace.to) toRow.appendChild(createCopyIcon(trace.to));
                overviewGrid.appendChild(toRow);

                // Value
                const valueRow = document.createElement('div');
                valueRow.className = 'tx-overview-row';
                valueRow.innerHTML = `<span class="tx-overview-label">Value</span>`;
                const valueDisplay = document.createElement('span');
                valueDisplay.className = 'tx-overview-value';
                const formattedVal = formatValue(trace.value);
                valueDisplay.textContent = formattedVal || `0 ${getNativeToken()}`;
                valueRow.appendChild(valueDisplay);
                overviewGrid.appendChild(valueRow);

                // Input Data
                if (trace.input && trace.input !== '0x') {
                    const inputRow = document.createElement('div');
                    inputRow.className = 'tx-overview-row';
                    inputRow.innerHTML = `<span class="tx-overview-label">Input Data</span>`;
                    const inputValue = document.createElement('span');
                    inputValue.className = 'tx-overview-value mono';
                    const inputData = trace.input;
                    inputValue.textContent = inputData.length > 66
                        ? inputData.slice(0, 34) + '...' + inputData.slice(-32)
                        : inputData;
                    inputValue.title = inputData;
                    inputRow.appendChild(inputValue);
                    inputRow.appendChild(createCopyIcon(inputData));
                    overviewGrid.appendChild(inputRow);
                }

                overview.appendChild(overviewGrid);
                app.appendChild(overview);
            }

            // Balance changes
            const balanceChanges = renderBalanceChanges(simulation);
            if (balanceChanges) {
                app.appendChild(balanceChanges);
            }

            // Simulation meta
            const meta = document.createElement('div');
            meta.className = 'simulation-meta';
            meta.innerHTML = `
                <div class="meta-card">
                    <div class="meta-label">Status</div>
                    <div class="meta-value ${simulation.success ? 'success' : 'failed'}">${simulation.success ? 'Success' : 'Reverted'}</div>
                </div>
                <div class="meta-card">
                    <div class="meta-label">Gas Used</div>
                    <div class="meta-value">${simulation.gasUsed?.toLocaleString() || 'N/A'}</div>
                </div>
                <div class="meta-card">
                    <div class="meta-label">Block</div>
                    <div class="meta-value">${simulation.blockNumber?.toLocaleString() || 'N/A'}</div>
                </div>
                <div class="meta-card">
                    <div class="meta-label">Chain ID</div>
                    <div class="meta-value">${currentChainId}</div>
                </div>
            `;
            app.appendChild(meta);

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
                    window.location.href = `/debug?tx=${simulation.txHash}&chain=${currentChainId}`;
                });
            }

            // Jump to revert button (in revert banner)
            const jumpToRevertBtn = document.getElementById('jumpToRevertBtn');
            if (jumpToRevertBtn && simulation.txHash) {
                jumpToRevertBtn.addEventListener('click', () => {
                    window.location.href = `/debug?tx=${simulation.txHash}&chain=${currentChainId}&jump=revert`;
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
