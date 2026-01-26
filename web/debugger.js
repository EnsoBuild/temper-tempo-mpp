(function() {
    'use strict';

    let debugTrace = null;
    let callTrace = []; // Filtered to only CALL-level operations
    let contractSources = new Map(); // address -> ContractSource
    let currentStep = 0;
    let isPlaying = false;
    let playInterval = null;
    let currentChainId = 1;
    let addressStack = []; // Track which contract we're in at each depth
    let topLevelCall = null; // Track the initial transaction call

    // Extract function calls from the raw trace
    function extractCallTrace(structLogs) {
        const calls = [];
        const callStack = []; // Track nested calls
        const callOps = ['CALL', 'DELEGATECALL', 'STATICCALL', 'CALLCODE', 'CREATE', 'CREATE2'];
        const returnOps = ['RETURN', 'REVERT', 'STOP', 'INVALID', 'SELFDESTRUCT'];

        for (let i = 0; i < structLogs.length; i++) {
            const step = structLogs[i];
            const op = step.op;

            if (callOps.includes(op)) {
                // Extract target address from stack
                // CALL: gas, addr, value, argsOffset, argsLength, retOffset, retLength
                // STATICCALL/DELEGATECALL: gas, addr, argsOffset, argsLength, retOffset, retLength
                let targetAddr = null;
                let value = '0x0';
                let argsOffset = 0;
                let argsLength = 0;

                if (step.stack && step.stack.length >= 2) {
                    const addrHex = step.stack[step.stack.length - 2];
                    if (addrHex) {
                        targetAddr = '0x' + addrHex.padStart(40, '0').slice(-40).toLowerCase();
                    }
                }

                if (op === 'CALL' || op === 'CALLCODE') {
                    // CALL has value at position 3
                    if (step.stack && step.stack.length >= 3) {
                        value = '0x' + (step.stack[step.stack.length - 3] || '0');
                    }
                    if (step.stack && step.stack.length >= 5) {
                        argsOffset = parseInt(step.stack[step.stack.length - 4] || '0', 16);
                        argsLength = parseInt(step.stack[step.stack.length - 5] || '0', 16);
                    }
                } else {
                    // STATICCALL/DELEGATECALL don't have value
                    if (step.stack && step.stack.length >= 4) {
                        argsOffset = parseInt(step.stack[step.stack.length - 3] || '0', 16);
                        argsLength = parseInt(step.stack[step.stack.length - 4] || '0', 16);
                    }
                }

                // Extract input data (function selector) from memory
                let inputData = '';
                if (step.memory && argsLength > 0) {
                    // Memory is array of 32-byte chunks
                    const startChunk = Math.floor(argsOffset / 32);
                    const endChunk = Math.floor((argsOffset + argsLength) / 32) + 1;

                    let memHex = '';
                    for (let c = startChunk; c < endChunk && c < step.memory.length; c++) {
                        memHex += step.memory[c] || '';
                    }

                    const startOffset = (argsOffset % 32) * 2;
                    inputData = '0x' + memHex.slice(startOffset, startOffset + argsLength * 2);
                }

                const call = {
                    type: 'call',
                    opcode: op,
                    index: i,
                    depth: step.depth,
                    gas: step.gas,
                    to: targetAddr,
                    value: value,
                    pc: step.pc,
                    inputData: inputData,
                    selector: inputData.length >= 10 ? inputData.slice(0, 10) : null,
                    success: true,
                    children: [],
                    gasUsed: 0,
                };

                if (callStack.length > 0) {
                    callStack[callStack.length - 1].children.push(call);
                } else {
                    calls.push(call);
                }
                callStack.push(call);

            } else if (returnOps.includes(op)) {
                if (callStack.length > 0) {
                    const call = callStack.pop();
                    call.success = op === 'RETURN' || op === 'STOP';
                    call.gasUsed = call.gas - step.gas;
                    call.returnOp = op;
                }
            }
        }

        return calls;
    }

    // Extract calls from callTracer result (preferred - has input data)
    function extractCallsFromCallTree(callTree, depth = 0) {
        if (!callTree) return [];

        const calls = [];

        // Parse gas values (can be hex strings)
        const parseGas = (g) => {
            if (!g) return 0;
            if (typeof g === 'number') return g;
            if (typeof g === 'string') {
                return g.startsWith('0x') ? parseInt(g, 16) : parseInt(g, 10);
            }
            return 0;
        };

        const gas = parseGas(callTree.gas);
        const gasUsed = parseGas(callTree.gasUsed);

        const call = {
            type: 'call',
            opcode: callTree.type || 'CALL',
            depth: depth,
            gas: gas,
            gasUsed: gasUsed,
            to: callTree.to ? callTree.to.toLowerCase() : null,
            from: callTree.from ? callTree.from.toLowerCase() : null,
            value: callTree.value || '0x0',
            inputData: callTree.input || '',
            selector: callTree.input && callTree.input.length >= 10 ? callTree.input.slice(0, 10) : null,
            output: callTree.output || '',
            success: !callTree.error,
            error: callTree.error,
            revertReason: callTree.revertReason,
            children: [],
            isTopLevel: depth === 0,
        };

        calls.push(call);

        // Process nested calls
        if (callTree.calls && callTree.calls.length > 0) {
            for (const nestedCall of callTree.calls) {
                const nestedCalls = extractCallsFromCallTree(nestedCall, depth + 1);
                calls.push(...nestedCalls);
            }
        }

        return calls;
    }

    // Flatten call tree for stepping
    function flattenCalls(calls, result = []) {
        for (const call of calls) {
            result.push(call);
            if (call.children && call.children.length > 0) {
                flattenCalls(call.children, result);
            }
        }
        return result;
    }

    // Fetch contract source from backend
    async function fetchContractSource(address) {
        if (contractSources.has(address.toLowerCase())) {
            return contractSources.get(address.toLowerCase());
        }

        try {
            const response = await fetch(`/api/v1/contract/${currentChainId}/${address}/source`);
            if (!response.ok) return null;

            const source = await response.json();
            contractSources.set(address.toLowerCase(), source);
            return source;
        } catch (e) {
            console.error('Failed to fetch source for', address, e);
            return null;
        }
    }

    // Simple Solidity syntax highlighting
    function highlightSolidity(code) {
        const keywords = ['pragma', 'solidity', 'contract', 'interface', 'library', 'function', 'modifier',
            'event', 'struct', 'enum', 'mapping', 'public', 'private', 'internal', 'external', 'view',
            'pure', 'payable', 'memory', 'storage', 'calldata', 'returns', 'return', 'if', 'else',
            'for', 'while', 'do', 'break', 'continue', 'require', 'assert', 'revert', 'emit', 'new',
            'delete', 'true', 'false', 'this', 'super', 'constructor', 'receive', 'fallback',
            'virtual', 'override', 'abstract', 'immutable', 'constant', 'indexed', 'anonymous'];

        const types = ['address', 'bool', 'string', 'bytes', 'byte', 'int', 'uint', 'fixed', 'ufixed',
            'int8', 'int16', 'int32', 'int64', 'int128', 'int256',
            'uint8', 'uint16', 'uint32', 'uint64', 'uint128', 'uint256',
            'bytes1', 'bytes2', 'bytes4', 'bytes8', 'bytes16', 'bytes32'];

        let result = code
            // Escape HTML
            .replace(/&/g, '&amp;')
            .replace(/</g, '&lt;')
            .replace(/>/g, '&gt;')
            // Comments
            .replace(/(\/\/.*$)/gm, '<span class="sol-comment">$1</span>')
            .replace(/(\/\*[\s\S]*?\*\/)/g, '<span class="sol-comment">$1</span>')
            // Strings
            .replace(/("(?:[^"\\]|\\.)*")/g, '<span class="sol-string">$1</span>')
            .replace(/('(?:[^'\\]|\\.)*')/g, '<span class="sol-string">$1</span>')
            // Numbers (hex and decimal)
            .replace(/\b(0x[a-fA-F0-9]+)\b/g, '<span class="sol-number">$1</span>')
            .replace(/\b(\d+)\b/g, '<span class="sol-number">$1</span>');

        // Keywords
        for (const kw of keywords) {
            const regex = new RegExp(`\\b(${kw})\\b`, 'g');
            result = result.replace(regex, '<span class="sol-keyword">$1</span>');
        }

        // Types
        for (const t of types) {
            const regex = new RegExp(`\\b(${t})\\b`, 'g');
            result = result.replace(regex, '<span class="sol-type">$1</span>');
        }

        return result;
    }

    // Render source code with line numbers
    function renderSourceCode(source, highlightLines = []) {
        if (!source) return '<div class="no-source">No verified source code available</div>';

        const lines = source.split('\n');
        return lines.map((line, i) => {
            const lineNum = i + 1;
            const isHighlighted = highlightLines.includes(lineNum);
            const highlightedCode = highlightSolidity(line || ' ');
            return `<div class="source-line ${isHighlighted ? 'highlighted' : ''}" data-line="${lineNum}" id="line-${lineNum}">
                <span class="line-number">${lineNum}</span>
                <span class="line-code">${highlightedCode}</span>
            </div>`;
        }).join('');
    }

    // Find function in source code by selector or name
    function findFunctionInSource(source, funcSelector, abi) {
        if (!source || !funcSelector) return null;

        // Try to get function name from ABI
        let funcName = null;
        if (abi && Array.isArray(abi)) {
            for (const item of abi) {
                if (item.type === 'function' && item.name) {
                    // Compute selector
                    const sig = `${item.name}(${(item.inputs || []).map(i => i.type).join(',')})`;
                    // We'd need to hash this, but for now just store the name
                    // and we'll match by searching
                }
            }
        }

        // Search for function definitions in source
        const lines = source.split('\n');
        const functionMatches = [];

        // Look for "function X(" patterns
        const funcRegex = /function\s+(\w+)\s*\(/g;

        for (let i = 0; i < lines.length; i++) {
            const line = lines[i];
            let match;
            while ((match = funcRegex.exec(line)) !== null) {
                functionMatches.push({
                    name: match[1],
                    line: i + 1,
                    // Find the closing brace of this function
                    endLine: findFunctionEnd(lines, i)
                });
            }
            funcRegex.lastIndex = 0; // Reset regex
        }

        return functionMatches;
    }

    // Find the end of a function (closing brace)
    function findFunctionEnd(lines, startLine) {
        let braceCount = 0;
        let started = false;

        for (let i = startLine; i < lines.length; i++) {
            const line = lines[i];
            for (const char of line) {
                if (char === '{') {
                    braceCount++;
                    started = true;
                } else if (char === '}') {
                    braceCount--;
                    if (started && braceCount === 0) {
                        return i + 1;
                    }
                }
            }
        }
        return startLine + 10; // Fallback
    }

    // Try to decode function name from selector using 4byte directory
    async function decodeFunctionSelector(selector) {
        if (!selector || selector.length < 10) return null;

        const sel = selector.slice(0, 10);
        try {
            const response = await fetch(`https://www.4byte.directory/api/v1/signatures/?hex_signature=${sel}`);
            if (response.ok) {
                const data = await response.json();
                if (data.results && data.results.length > 0) {
                    // Get the most likely one (lowest ID = oldest = most common)
                    const sig = data.results.sort((a, b) => a.id - b.id)[0].text_signature;
                    const funcName = sig.split('(')[0];
                    return funcName;
                }
            }
        } catch (e) {
            console.log('Failed to decode selector', sel);
        }
        return null;
    }

    // Decode function input data using ABI
    function decodeInputData(inputData, abi, funcName) {
        if (!inputData || inputData.length < 10 || !abi || !Array.isArray(abi)) {
            return null;
        }

        // Find the function in ABI
        const funcAbi = abi.find(item =>
            item.type === 'function' && item.name === funcName
        );

        if (!funcAbi || !funcAbi.inputs || funcAbi.inputs.length === 0) {
            return null;
        }

        // Remove selector (first 4 bytes / 10 chars including 0x)
        const paramData = inputData.slice(10);

        // Simple decoder - assumes 32-byte aligned parameters
        const params = [];
        const inputs = funcAbi.inputs;

        for (let i = 0; i < inputs.length; i++) {
            const input = inputs[i];
            const offset = i * 64; // Each param is 32 bytes = 64 hex chars
            const rawValue = paramData.slice(offset, offset + 64);

            if (!rawValue) break;

            let decoded = '0x' + rawValue;

            // Try to format based on type
            if (input.type === 'address') {
                decoded = '0x' + rawValue.slice(-40);
            } else if (input.type.startsWith('uint') || input.type.startsWith('int')) {
                // Parse as BigInt and format
                try {
                    const bn = BigInt('0x' + rawValue);
                    if (bn < 1000000n) {
                        decoded = bn.toString();
                    } else if (bn < BigInt(10 ** 18) * 1000n) {
                        // Could be wei - show as decimal
                        decoded = bn.toString();
                    } else {
                        decoded = '0x' + rawValue.replace(/^0+/, '') || '0';
                    }
                } catch {
                    decoded = '0x' + rawValue;
                }
            } else if (input.type === 'bool') {
                decoded = rawValue.endsWith('1') ? 'true' : 'false';
            } else if (input.type === 'bytes32') {
                decoded = '0x' + rawValue;
            }

            params.push({
                name: input.name || `arg${i}`,
                type: input.type,
                value: decoded,
            });
        }

        return params;
    }

    // Format decoded params for display
    function formatDecodedParams(params) {
        if (!params || params.length === 0) return '';

        return params.map(p => `
            <div class="param-row">
                <span class="param-name">${p.name}</span>
                <span class="param-type">${p.type}</span>
                <span class="param-value" title="${p.value}">${formatParamValue(p.value, p.type)}</span>
            </div>
        `).join('');
    }

    // Format a parameter value for display
    function formatParamValue(value, type) {
        if (!value) return '-';

        if (type === 'address') {
            return value.slice(0, 10) + '...' + value.slice(-6);
        }

        if (typeof value === 'string' && value.length > 30) {
            return value.slice(0, 15) + '...' + value.slice(-10);
        }

        return value;
    }

    // Find function line number in source by name
    function findFunctionLineByName(source, funcName) {
        if (!source || !funcName) return null;

        const lines = source.split('\n');
        const regex = new RegExp(`function\\s+${funcName}\\s*\\(`);

        for (let i = 0; i < lines.length; i++) {
            if (regex.test(lines[i])) {
                // Return the range of lines for this function
                const endLine = findFunctionEnd(lines, i);
                return { start: i + 1, end: endLine };
            }
        }
        return null;
    }

    // Get all functions from source code
    function getAllFunctions(source) {
        if (!source) return [];

        const lines = source.split('\n');
        const functions = [];
        const funcRegex = /function\s+(\w+)\s*\(/g;

        for (let i = 0; i < lines.length; i++) {
            const line = lines[i];
            let match;
            while ((match = funcRegex.exec(line)) !== null) {
                const name = match[1];
                const endLine = findFunctionEnd(lines, i);
                functions.push({
                    name: name,
                    line: i + 1,
                    endLine: endLine,
                });
            }
            funcRegex.lastIndex = 0;
        }

        return functions;
    }

    // Update the function index panel
    function updateFunctionIndex(source, currentFuncName, currentFuncRange) {
        const funcPanel = document.getElementById('functionIndexPanel');
        const funcCount = document.getElementById('funcCount');
        if (!funcPanel) return;

        const functions = getAllFunctions(source);

        if (funcCount) {
            funcCount.textContent = `(${functions.length})`;
        }

        if (functions.length === 0) {
            funcPanel.innerHTML = '<div class="no-source">No functions found</div>';
            return;
        }

        funcPanel.innerHTML = functions.map(fn => {
            const isActive = fn.name === currentFuncName;
            return `
                <div class="func-index-item ${isActive ? 'active' : ''}" data-line="${fn.line}">
                    <span class="func-index-name">${fn.name}</span>
                    <span class="func-index-line">:${fn.line}</span>
                </div>
            `;
        }).join('');

        // Add click handlers
        funcPanel.querySelectorAll('.func-index-item').forEach(el => {
            el.addEventListener('click', () => {
                const line = parseInt(el.dataset.line);
                const lineEl = document.getElementById(`line-${line}`);
                if (lineEl) {
                    // Remove previous highlights
                    document.querySelectorAll('.source-line.highlighted').forEach(l => {
                        l.classList.remove('highlighted');
                    });
                    // Add highlight
                    lineEl.classList.add('highlighted');
                    lineEl.scrollIntoView({ block: 'center', behavior: 'smooth' });
                }
            });
        });
    }

    // Get current contract address based on call depth
    function getCurrentContractAddress(step) {
        // This is simplified - in reality we'd need to track CALL/CREATE operations
        // For now, we'll try to get it from the trace context
        return addressStack[step.depth] || null;
    }

    // Opcode categories for coloring
    const OPCODE_CATEGORIES = {
        arithmetic: ['ADD', 'SUB', 'MUL', 'DIV', 'SDIV', 'MOD', 'SMOD', 'ADDMOD', 'MULMOD', 'EXP', 'SIGNEXTEND'],
        comparison: ['LT', 'GT', 'SLT', 'SGT', 'EQ', 'ISZERO'],
        bitwise: ['AND', 'OR', 'XOR', 'NOT', 'BYTE', 'SHL', 'SHR', 'SAR'],
        memory: ['MLOAD', 'MSTORE', 'MSTORE8', 'MSIZE'],
        storage: ['SLOAD', 'SSTORE'],
        flow: ['JUMP', 'JUMPI', 'JUMPDEST', 'PC'],
        stack: ['POP', 'DUP1', 'DUP2', 'DUP3', 'DUP4', 'DUP5', 'DUP6', 'DUP7', 'DUP8', 'DUP9', 'DUP10', 'DUP11', 'DUP12', 'DUP13', 'DUP14', 'DUP15', 'DUP16', 'SWAP1', 'SWAP2', 'SWAP3', 'SWAP4', 'SWAP5', 'SWAP6', 'SWAP7', 'SWAP8', 'SWAP9', 'SWAP10', 'SWAP11', 'SWAP12', 'SWAP13', 'SWAP14', 'SWAP15', 'SWAP16'],
        push: ['PUSH0', 'PUSH1', 'PUSH2', 'PUSH3', 'PUSH4', 'PUSH5', 'PUSH6', 'PUSH7', 'PUSH8', 'PUSH9', 'PUSH10', 'PUSH11', 'PUSH12', 'PUSH13', 'PUSH14', 'PUSH15', 'PUSH16', 'PUSH17', 'PUSH18', 'PUSH19', 'PUSH20', 'PUSH21', 'PUSH22', 'PUSH23', 'PUSH24', 'PUSH25', 'PUSH26', 'PUSH27', 'PUSH28', 'PUSH29', 'PUSH30', 'PUSH31', 'PUSH32'],
        call: ['CALL', 'CALLCODE', 'DELEGATECALL', 'STATICCALL', 'CREATE', 'CREATE2'],
        log: ['LOG0', 'LOG1', 'LOG2', 'LOG3', 'LOG4'],
        system: ['STOP', 'RETURN', 'REVERT', 'INVALID', 'SELFDESTRUCT'],
    };

    function getOpcodeCategory(op) {
        for (const [category, ops] of Object.entries(OPCODE_CATEGORIES)) {
            if (ops.includes(op)) return category;
        }
        return 'other';
    }

    function formatHex(hex, maxLen = 20) {
        if (!hex) return '';
        if (hex.length <= maxLen) return hex;
        return hex.slice(0, maxLen / 2) + '...' + hex.slice(-maxLen / 2);
    }

    function formatStack(stack) {
        if (!stack || stack.length === 0) return '<empty>';
        return stack.map((item, i) => {
            const formatted = formatHex(item, 40);
            return `<div class="stack-item"><span class="stack-index">${stack.length - 1 - i}</span><span class="stack-value">${formatted}</span></div>`;
        }).reverse().join('');
    }

    function formatStorage(storage) {
        if (!storage || Object.keys(storage).length === 0) return '<empty>';
        return Object.entries(storage).map(([slot, value]) => {
            return `<div class="storage-item"><span class="storage-slot">${formatHex(slot, 20)}</span><span class="storage-value">${formatHex(value, 40)}</span></div>`;
        }).join('');
    }

    function formatMemory(memory) {
        if (!memory || memory.length === 0) return '<empty>';
        return memory.map((chunk, i) => {
            const offset = (i * 32).toString(16).padStart(4, '0');
            return `<div class="memory-item"><span class="memory-offset">0x${offset}</span><span class="memory-value">${chunk}</span></div>`;
        }).join('');
    }

    function renderStep(step, index) {
        const category = getOpcodeCategory(step.op);
        return `
            <div class="debug-step ${index === currentStep ? 'active' : ''}" data-index="${index}">
                <span class="step-index">#${index}</span>
                <span class="step-pc">PC:${step.pc}</span>
                <span class="step-op ${category}">${step.op}</span>
                <span class="step-gas">${step.gas.toLocaleString()} gas</span>
                <span class="step-depth">depth:${step.depth}</span>
            </div>
        `;
    }

    // Render a function call (not raw opcode)
    async function renderCallStep(call, index) {
        const statusClass = call.success ? 'success' : 'reverted';
        const statusIcon = call.success ? '✓' : '✗';

        // Get contract name from source
        let contractName = call.to ? call.to.slice(0, 10) + '...' : 'Unknown';
        if (call.to && contractSources.has(call.to.toLowerCase())) {
            const source = contractSources.get(call.to.toLowerCase());
            if (source && source.name) {
                contractName = source.name;
            }
        }

        // Build function display
        let funcDisplay = call.opcode;
        if (call.decodedFunc) {
            funcDisplay = call.decodedFunc + '()';
        }

        // Indent based on depth (top level is 0)
        const paddingLeft = call.isTopLevel ? 0 : (call.depth * 20);

        return `
            <div class="call-step ${index === currentStep ? 'active' : ''} ${statusClass}" data-index="${index}" style="padding-left: ${paddingLeft}px">
                <span class="call-status ${statusClass}">${statusIcon}</span>
                <span class="call-type">${call.opcode}</span>
                <span class="call-func">${funcDisplay}</span>
                <span class="call-target">${contractName}</span>
                <span class="call-address" title="${call.to || ''}">${call.to ? call.to.slice(0, 10) + '...' : ''}</span>
                <span class="call-gas">${call.gasUsed ? call.gasUsed.toLocaleString() : '?'} gas</span>
            </div>
        `;
    }

    function updateDisplay() {
        if (!callTrace || callTrace.length === 0) {
            return;
        }

        const call = callTrace[currentStep];
        if (!call) return;

        // Update step counter
        document.getElementById('stepCounter').textContent = `${currentStep + 1} / ${callTrace.length}`;

        // Update current call display
        const statusClass = call.success ? 'success' : 'reverted';
        document.getElementById('currentOp').innerHTML = `
            <span class="current-op-badge ${call.opcode === 'CALL' ? 'call' : 'other'}">${call.opcode}</span>
            <span class="current-target">${call.to ? call.to.slice(0, 14) + '...' : 'Unknown'}</span>
            <span class="current-gas">${call.gasUsed ? call.gasUsed.toLocaleString() + ' gas' : ''}</span>
            <span class="current-status ${statusClass}">${call.success ? 'SUCCESS' : 'REVERTED'}</span>
        `;

        // Update call details panel
        const detailsPanel = document.getElementById('callDetailsPanel');
        if (detailsPanel) {
            const funcDisplay = call.decodedFunc ? `${call.decodedFunc}()` : (call.selector || 'Unknown');

            // Try to decode input parameters if we have ABI
            let paramsHtml = '';
            if (call.to && call.decodedFunc && call.inputData && contractSources.has(call.to.toLowerCase())) {
                const source = contractSources.get(call.to.toLowerCase());
                if (source && source.abi) {
                    const params = decodeInputData(call.inputData, source.abi, call.decodedFunc);
                    if (params && params.length > 0) {
                        paramsHtml = `
                            <div class="params-section">
                                <div class="params-header">Input Parameters</div>
                                ${formatDecodedParams(params)}
                            </div>
                        `;
                    }
                }
            }

            // Show raw input data if no decoded params
            let inputDataHtml = '';
            if (call.inputData && call.inputData.length > 10) {
                const truncatedInput = call.inputData.length > 100
                    ? call.inputData.slice(0, 50) + '...' + call.inputData.slice(-20)
                    : call.inputData;
                inputDataHtml = `<div class="detail-row"><span class="detail-label">Input</span><span class="detail-value mono" title="${call.inputData}">${truncatedInput}</span></div>`;
            }

            detailsPanel.innerHTML = `
                <div class="detail-row"><span class="detail-label">Type</span><span class="detail-value">${call.opcode}${call.isTopLevel ? ' (Entry)' : ''}</span></div>
                <div class="detail-row"><span class="detail-label">Function</span><span class="detail-value call-func-detail">${funcDisplay}</span></div>
                <div class="detail-row"><span class="detail-label">To</span><span class="detail-value mono">${call.to || 'N/A'}</span></div>
                <div class="detail-row"><span class="detail-label">From</span><span class="detail-value mono">${call.from || 'N/A'}</span></div>
                <div class="detail-row"><span class="detail-label">Depth</span><span class="detail-value">${call.depth}</span></div>
                <div class="detail-row"><span class="detail-label">Gas Used</span><span class="detail-value">${call.gasUsed ? call.gasUsed.toLocaleString() : 'N/A'}</span></div>
                <div class="detail-row"><span class="detail-label">Status</span><span class="detail-value ${statusClass}">${call.success ? 'Success' : call.error || 'Reverted'}</span></div>
                ${inputDataHtml}
                ${paramsHtml}
            `;
        }

        // Update source code panel with function highlighting
        const sourceContent = document.getElementById('sourceContent');
        const sourceFileName = document.getElementById('sourceFileName');

        if (call.to && contractSources.has(call.to.toLowerCase())) {
            const source = contractSources.get(call.to.toLowerCase());
            const mainFile = source.sourceList?.[0] || Object.keys(source.sources)[0];
            const mainSource = source.sources[mainFile];

            if (mainSource) {
                // Try to find which function is being called
                let highlightLines = [];
                let funcName = call.decodedFunc; // Cached from earlier decode

                if (!funcName && call.inputData && call.inputData.length >= 10) {
                    // Try to decode from ABI first
                    const selector = call.inputData.slice(0, 10);
                    if (source.abi && Array.isArray(source.abi)) {
                        for (const item of source.abi) {
                            if (item.type === 'function') {
                                // Simple selector matching - compute keccak would be better
                                // For now, store the name and we'll search
                            }
                        }
                    }
                }

                // Find function in source
                let funcRange = null;
                if (funcName) {
                    funcRange = findFunctionLineByName(mainSource, funcName);
                    if (funcRange) {
                        // Highlight the function definition and first few lines
                        for (let i = funcRange.start; i <= Math.min(funcRange.start + 5, funcRange.end); i++) {
                            highlightLines.push(i);
                        }
                        call.funcRange = funcRange;
                    }
                }

                const contractName = source.name || 'Contract';
                const fileName = mainFile.split('/').pop();
                sourceFileName.textContent = funcName
                    ? `${contractName}.${funcName}() - ${fileName}`
                    : `${contractName} - ${fileName}`;

                sourceContent.innerHTML = renderSourceCode(mainSource, highlightLines);

                // Scroll to highlighted line
                const scrollToLine = highlightLines.length > 0 ? highlightLines[0] : null;
                if (scrollToLine) {
                    setTimeout(() => {
                        const lineEl = document.getElementById(`line-${scrollToLine}`);
                        if (lineEl) {
                            lineEl.scrollIntoView({ block: 'center', behavior: 'smooth' });
                        }
                    }, 50);
                }

                // Update function index panel
                updateFunctionIndex(mainSource, funcName, funcRange);
            }
        } else if (call.to) {
            sourceFileName.textContent = call.to.slice(0, 14) + '...';
            sourceContent.innerHTML = `
                <div class="no-source">
                    <div class="no-source-icon">📄</div>
                    <div class="no-source-text">Source not verified</div>
                    <div class="no-source-hint">${call.to}</div>
                </div>
            `;

            // Clear function index
            const funcPanel = document.getElementById('functionIndexPanel');
            if (funcPanel) {
                funcPanel.innerHTML = '<div class="no-source">No verified source</div>';
            }
            const funcCount = document.getElementById('funcCount');
            if (funcCount) funcCount.textContent = '';
        }

        // Scroll call list to current call
        const stepList = document.getElementById('stepList');
        const activeStep = stepList.querySelector('.call-step.active');
        if (activeStep) {
            activeStep.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
        }

        // Update active call in list
        stepList.querySelectorAll('.call-step').forEach((el, i) => {
            el.classList.toggle('active', i === currentStep);
        });
    }

    function goToStep(index) {
        if (!callTrace || callTrace.length === 0) return;
        currentStep = Math.max(0, Math.min(index, callTrace.length - 1));
        updateDisplay();
    }

    function stepForward() {
        goToStep(currentStep + 1);
    }

    function stepBack() {
        goToStep(currentStep - 1);
    }

    function togglePlay() {
        isPlaying = !isPlaying;
        const btn = document.getElementById('playBtn');
        btn.textContent = isPlaying ? '⏸' : '▶';
        btn.classList.toggle('playing', isPlaying);

        if (isPlaying) {
            playInterval = setInterval(() => {
                if (currentStep >= callTrace.length - 1) {
                    togglePlay();
                    return;
                }
                stepForward();
            }, 500); // Slower for function calls
        } else {
            clearInterval(playInterval);
        }
    }

    function goToStart() {
        goToStep(0);
    }

    function goToEnd() {
        if (!callTrace || callTrace.length === 0) return;
        goToStep(callTrace.length - 1);
    }

    async function loadDebugTrace(txHash, chainId) {
        const app = document.getElementById('app');
        app.className = 'debugger-container';

        app.innerHTML = `
            <div class="loading-container">
                <div class="loading-spinner"></div>
                <div>Loading debug trace<span class="loading-inline"><span class="loading-dot"></span><span class="loading-dot"></span><span class="loading-dot"></span></span></div>
            </div>
        `;

        try {
            const response = await fetch('/api/v1/debug', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ txHash, chainId })
            });

            if (!response.ok) {
                throw new Error(`Failed to fetch debug trace: ${response.status}`);
            }

            debugTrace = await response.json();
            currentStep = 0;

            renderDebugger(txHash, chainId);
        } catch (error) {
            app.innerHTML = `
                <div class="error-container">
                    <div class="error-icon">⚠</div>
                    <div class="error-title">Failed to load debug trace</div>
                    <div class="error-message-text">${error.message}</div>
                    <div class="error-hint">Note: Debug tracing requires an archive node with debug_traceTransaction support</div>
                </div>
            `;
        }
    }

    async function renderDebugger(txHash, chainId) {
        const app = document.getElementById('app');
        currentChainId = chainId;

        // Use callTracer result if available (has input data for all calls)
        const callTreeData = debugTrace.callTree || debugTrace.call_tree;

        if (callTreeData) {
            // Extract calls from callTracer result - includes input data for all nested calls
            callTrace = extractCallsFromCallTree(callTreeData);
            console.log(`Extracted ${callTrace.length} function calls from callTracer`);
        } else {
            // Fallback: Create the top-level call from transaction data
            const txInput = debugTrace.txInput || debugTrace.tx_input || '';
            const txTo = debugTrace.txTo || debugTrace.tx_to || '';
            const txFrom = debugTrace.txFrom || debugTrace.tx_from || '';

            topLevelCall = {
                type: 'call',
                opcode: 'CALL',
                index: -1,
                depth: 0,
                gas: debugTrace.gas || 0,
                to: txTo.toLowerCase(),
                from: txFrom.toLowerCase(),
                value: '0x0',
                pc: 0,
                inputData: txInput,
                selector: txInput && txInput.length >= 10 ? txInput.slice(0, 10) : null,
                success: !debugTrace.failed,
                children: [],
                gasUsed: debugTrace.gas || 0,
                isTopLevel: true,
            };

            // Extract function calls from raw trace (fallback)
            const rawCalls = extractCallTrace(debugTrace.structLogs || []);
            callTrace = [topLevelCall, ...flattenCalls(rawCalls)];
            console.log(`Extracted ${callTrace.length} function calls from structLogs (fallback)`);
        }

        console.log('Calls:', callTrace.map(c => ({ to: c.to, selector: c.selector, depth: c.depth })));

        // Helper to check if address is a precompiled contract (first 256 addresses)
        const isPrecompile = (addr) => {
            if (!addr) return false;
            const addrNum = parseInt(addr.slice(2), 16);
            return addrNum < 256;
        };

        // Pre-fetch sources for all unique addresses (excluding precompiles)
        const uniqueAddresses = [...new Set(callTrace.map(c => c.to).filter(a => a && !isPrecompile(a)))];
        for (const addr of uniqueAddresses.slice(0, 15)) { // Limit to first 15 contracts
            try {
                const response = await fetch(`/api/v1/contract/${chainId}/${addr}/source`);
                if (response.ok) {
                    const source = await response.json();
                    contractSources.set(addr.toLowerCase(), source);
                }
            } catch (e) {
                console.log('No source for', addr);
            }
        }

        // Decode function selectors for calls
        const uniqueSelectors = [...new Set(callTrace.map(c => c.selector).filter(Boolean))];
        const selectorMap = new Map();

        for (const sel of uniqueSelectors.slice(0, 30)) {
            const funcName = await decodeFunctionSelector(sel);
            if (funcName) {
                selectorMap.set(sel, funcName);
            }
        }

        // Attach function names to calls
        for (const call of callTrace) {
            if (call.selector && selectorMap.has(call.selector)) {
                call.decodedFunc = selectorMap.get(call.selector);
            }
        }

        // Render call steps
        const callStepsHtml = await Promise.all(callTrace.map((call, i) => renderCallStep(call, i)));

        // Get simulation ID from URL if available
        const urlParams = new URLSearchParams(window.location.search);
        const simId = urlParams.get('sim');

        app.innerHTML = `
            <header class="debugger-header">
                <div class="logo">
                    <button class="back-btn" id="backToSimBtn" title="Back to simulation">←</button>
                    <div class="logo-icon">T</div>
                    <h1>Temper Debugger</h1>
                </div>
                <div class="tx-info">
                    <span class="tx-hash">${formatHex(txHash, 24)}</span>
                    <span class="chain-badge">Chain ${chainId}</span>
                    <span class="step-count">${callTrace.length} calls</span>
                </div>
            </header>

            <div class="debugger-layout">
                <div class="source-panel">
                    <div class="panel-header">
                        <span>Source Code</span>
                        <span class="source-file" id="sourceFileName">Select a call</span>
                    </div>
                    <div class="source-content" id="sourceContent">
                        <div class="no-source">
                            <div class="no-source-icon">👆</div>
                            <div class="no-source-text">Select a function call to view source</div>
                        </div>
                    </div>
                </div>

                <div class="debugger-main">
                    <div class="controls-bar">
                        <div class="step-controls">
                            <button class="control-btn" id="startBtn" title="Go to start (Home)">⏮</button>
                            <button class="control-btn" id="backBtn" title="Previous call (←)">◀</button>
                            <button class="control-btn play-btn" id="playBtn" title="Play/Pause (Space)">▶</button>
                            <button class="control-btn" id="forwardBtn" title="Next call (→)">▶</button>
                            <button class="control-btn" id="endBtn" title="Go to end (End)">⏭</button>
                        </div>
                        <div class="step-counter" id="stepCounter">1 / ${callTrace.length}</div>
                        <div class="current-op" id="currentOp"></div>
                    </div>

                    <div class="step-list-container">
                        <div class="call-list" id="stepList">
                            ${callStepsHtml.join('')}
                        </div>
                    </div>
                </div>

                <div class="debugger-sidebar">
                    <div class="panel call-info-panel">
                        <div class="panel-header">Call Details</div>
                        <div class="panel-content" id="callDetailsPanel"></div>
                    </div>
                    <div class="panel function-index-panel">
                        <div class="panel-header">Functions <span class="panel-count" id="funcCount"></span></div>
                        <div class="panel-content" id="functionIndexPanel">
                            <div class="no-source">Select a call to see functions</div>
                        </div>
                    </div>
                </div>
            </div>

            <div class="debugger-footer">
                <span class="status ${debugTrace.failed ? 'failed' : 'success'}">${debugTrace.failed ? 'REVERTED' : 'SUCCESS'}</span>
                <span class="total-gas">Total gas: ${debugTrace.gas?.toLocaleString() || 'N/A'}</span>
                <span class="keyboard-hint">← → Navigate calls | Space Play | Home/End Jump</span>
            </div>
        `;

        // Wire up controls
        document.getElementById('startBtn').addEventListener('click', goToStart);
        document.getElementById('backBtn').addEventListener('click', stepBack);
        document.getElementById('playBtn').addEventListener('click', togglePlay);
        document.getElementById('forwardBtn').addEventListener('click', stepForward);
        document.getElementById('endBtn').addEventListener('click', goToEnd);

        // Back to simulation button
        document.getElementById('backToSimBtn').addEventListener('click', () => {
            if (simId) {
                window.location.href = `/simulation/${simId}`;
            } else {
                window.history.back();
            }
        });

        // Click on call to jump
        document.getElementById('stepList').addEventListener('click', (e) => {
            const stepEl = e.target.closest('.call-step');
            if (stepEl) {
                goToStep(parseInt(stepEl.dataset.index));
            }
        });

        // Keyboard controls
        document.addEventListener('keydown', (e) => {
            if (e.key === 'ArrowRight' || e.key === 'l') stepForward();
            else if (e.key === 'ArrowLeft' || e.key === 'h') stepBack();
            else if (e.key === ' ') { e.preventDefault(); togglePlay(); }
            else if (e.key === 'Home') goToStart();
            else if (e.key === 'End') goToEnd();
        });

        // Initial display
        updateDisplay();
    }

    // Export
    window.loadDebugTrace = loadDebugTrace;
})();
