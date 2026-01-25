//! TIP-20 Token Precompile Implementation
//!
//! This module implements TIP-20 token precompile support for Tempo chain simulation.
//! TIP-20 tokens are native precompiles on Tempo with addresses starting with 0x20c0.
//!
//! Storage Layout (from tempo-foundry):
//! - Slot 0-1: RolesAuth (roles and roleAdmins mappings)
//! - Slot 2: name (string)
//! - Slot 3: symbol (string)
//! - Slot 4: currency (string)
//! - Slot 5: domainSeparator (bytes32, unused)
//! - Slot 6: quoteToken (address)
//! - Slot 7: nextQuoteToken + transferPolicyId
//! - Slot 8: totalSupply (uint256)
//! - Slot 9: balances mapping(address => uint256)
//! - Slot 10: allowances mapping(address => mapping(address => uint256))

use alloy_primitives::{keccak256, Address, Bytes, U256};

/// TIP-20 address prefix (first 2 bytes are 0x20c0)
pub const TIP20_PREFIX: [u8; 2] = [0x20, 0xc0];

/// TIP-20 storage slots
pub mod slots {
    use alloy_primitives::U256;

    pub const TOTAL_SUPPLY: U256 = U256::from_limbs([8, 0, 0, 0]);
    pub const BALANCES: U256 = U256::from_limbs([9, 0, 0, 0]);
    pub const ALLOWANCES: U256 = U256::from_limbs([10, 0, 0, 0]);
}

/// TIP-20 function selectors
pub mod selectors {
    /// balanceOf(address) -> uint256
    pub const BALANCE_OF: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];
    /// totalSupply() -> uint256
    pub const TOTAL_SUPPLY: [u8; 4] = [0x18, 0x16, 0x0d, 0xdd];
    /// allowance(address,address) -> uint256
    pub const ALLOWANCE: [u8; 4] = [0xdd, 0x62, 0xed, 0x3e];
    /// decimals() -> uint8
    pub const DECIMALS: [u8; 4] = [0x31, 0x3c, 0xe5, 0x67];
    /// transfer(address,uint256) -> bool
    pub const TRANSFER: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];
    /// approve(address,uint256) -> bool
    pub const APPROVE: [u8; 4] = [0x09, 0x5e, 0xa7, 0xb3];
    /// transferFrom(address,address,uint256) -> bool
    pub const TRANSFER_FROM: [u8; 4] = [0x23, 0xb8, 0x72, 0xdd];
    /// name() -> string
    pub const NAME: [u8; 4] = [0x06, 0xfd, 0xde, 0x03];
    /// symbol() -> string
    pub const SYMBOL: [u8; 4] = [0x95, 0xd8, 0x9b, 0x41];
}

/// Check if an address is a TIP-20 token address (starts with 0x20c0)
pub fn is_tip20_address(address: &Address) -> bool {
    let bytes = address.as_slice();
    bytes.len() >= 2 && bytes[0] == TIP20_PREFIX[0] && bytes[1] == TIP20_PREFIX[1]
}

/// Compute the storage slot for a mapping entry: keccak256(abi.encode(key, slot))
pub fn compute_mapping_slot(key: U256, slot: U256) -> U256 {
    let mut data = [0u8; 64];
    key.to_be_bytes::<32>().iter().enumerate().for_each(|(i, b)| data[i] = *b);
    slot.to_be_bytes::<32>().iter().enumerate().for_each(|(i, b)| data[32 + i] = *b);
    U256::from_be_bytes(keccak256(data).0)
}

/// Compute the storage slot for a nested mapping entry: keccak256(abi.encode(key2, keccak256(abi.encode(key1, slot))))
pub fn compute_nested_mapping_slot(key1: U256, key2: U256, slot: U256) -> U256 {
    let inner_slot = compute_mapping_slot(key1, slot);
    compute_mapping_slot(key2, inner_slot)
}

/// Compute the balance slot for an address
pub fn balance_slot(owner: Address) -> U256 {
    let owner_u256 = U256::from_be_bytes({
        let mut bytes = [0u8; 32];
        bytes[12..32].copy_from_slice(owner.as_slice());
        bytes
    });
    compute_mapping_slot(owner_u256, slots::BALANCES)
}

/// Compute the allowance slot for owner/spender
pub fn allowance_slot(owner: Address, spender: Address) -> U256 {
    let owner_u256 = U256::from_be_bytes({
        let mut bytes = [0u8; 32];
        bytes[12..32].copy_from_slice(owner.as_slice());
        bytes
    });
    let spender_u256 = U256::from_be_bytes({
        let mut bytes = [0u8; 32];
        bytes[12..32].copy_from_slice(spender.as_slice());
        bytes
    });
    compute_nested_mapping_slot(owner_u256, spender_u256, slots::ALLOWANCES)
}

/// TIP-20 precompile call result
#[derive(Debug, Clone)]
pub struct Tip20Result {
    pub success: bool,
    pub return_data: Bytes,
    /// Storage writes to apply (slot -> value)
    pub storage_writes: Vec<(U256, U256)>,
}

impl Tip20Result {
    pub fn success_with_data(data: Bytes) -> Self {
        Self {
            success: true,
            return_data: data,
            storage_writes: Vec::new(),
        }
    }

    pub fn success_with_bool(value: bool) -> Self {
        let mut data = [0u8; 32];
        if value {
            data[31] = 1;
        }
        Self::success_with_data(Bytes::from(data.to_vec()))
    }

    pub fn success_with_u256(value: U256) -> Self {
        Self::success_with_data(Bytes::from(value.to_be_bytes::<32>().to_vec()))
    }

    pub fn revert() -> Self {
        Self {
            success: false,
            return_data: Bytes::new(),
            storage_writes: Vec::new(),
        }
    }
}

/// Parse an address from calldata (32 bytes, right-aligned)
fn parse_address(data: &[u8], offset: usize) -> Option<Address> {
    if data.len() < offset + 32 {
        return None;
    }
    // Address is in the last 20 bytes of the 32-byte slot
    let addr_bytes: [u8; 20] = data[offset + 12..offset + 32].try_into().ok()?;
    Some(Address::from(addr_bytes))
}

/// Parse a U256 from calldata
fn parse_u256(data: &[u8], offset: usize) -> Option<U256> {
    if data.len() < offset + 32 {
        return None;
    }
    let bytes: [u8; 32] = data[offset..offset + 32].try_into().ok()?;
    Some(U256::from_be_bytes(bytes))
}

/// Execute a TIP-20 precompile call
///
/// # Arguments
/// * `calldata` - The calldata for the call
/// * `caller` - The address making the call
/// * `load_storage` - Function to load storage values
///
/// # Returns
/// * `Option<Tip20Result>` - The result if this is a recognized TIP-20 call
pub fn execute_tip20_call<F>(
    calldata: &[u8],
    caller: Address,
    mut load_storage: F,
) -> Option<Tip20Result>
where
    F: FnMut(U256) -> U256,
{
    if calldata.len() < 4 {
        return Some(Tip20Result::revert());
    }

    let selector: [u8; 4] = calldata[0..4].try_into().ok()?;

    match selector {
        selectors::BALANCE_OF => {
            let owner = parse_address(calldata, 4)?;
            let slot = balance_slot(owner);
            let balance = load_storage(slot);
            Some(Tip20Result::success_with_u256(balance))
        }

        selectors::TOTAL_SUPPLY => {
            let supply = load_storage(slots::TOTAL_SUPPLY);
            Some(Tip20Result::success_with_u256(supply))
        }

        selectors::ALLOWANCE => {
            let owner = parse_address(calldata, 4)?;
            let spender = parse_address(calldata, 36)?;
            let slot = allowance_slot(owner, spender);
            let allowance = load_storage(slot);
            Some(Tip20Result::success_with_u256(allowance))
        }

        selectors::DECIMALS => {
            // TIP-20 always uses 6 decimals
            Some(Tip20Result::success_with_u256(U256::from(6)))
        }

        selectors::TRANSFER => {
            let to = parse_address(calldata, 4)?;
            let amount = parse_u256(calldata, 36)?;

            // Load current balances
            let from_slot = balance_slot(caller);
            let to_slot = balance_slot(to);
            let from_balance = load_storage(from_slot);
            let to_balance = load_storage(to_slot);

            // Check sufficient balance
            if from_balance < amount {
                return Some(Tip20Result::revert());
            }

            // Compute new balances
            let new_from_balance = from_balance - amount;
            let new_to_balance = to_balance + amount;

            let mut result = Tip20Result::success_with_bool(true);
            result.storage_writes.push((from_slot, new_from_balance));
            result.storage_writes.push((to_slot, new_to_balance));
            Some(result)
        }

        selectors::APPROVE => {
            let spender = parse_address(calldata, 4)?;
            let amount = parse_u256(calldata, 36)?;

            let slot = allowance_slot(caller, spender);

            let mut result = Tip20Result::success_with_bool(true);
            result.storage_writes.push((slot, amount));
            Some(result)
        }

        selectors::TRANSFER_FROM => {
            let from = parse_address(calldata, 4)?;
            let to = parse_address(calldata, 36)?;
            let amount = parse_u256(calldata, 68)?;

            // Load balances and allowance
            let from_slot = balance_slot(from);
            let to_slot = balance_slot(to);
            let allowance_slot = allowance_slot(from, caller);

            let from_balance = load_storage(from_slot);
            let to_balance = load_storage(to_slot);
            let allowance = load_storage(allowance_slot);

            // Check sufficient balance and allowance
            if from_balance < amount || allowance < amount {
                return Some(Tip20Result::revert());
            }

            // Compute new values
            let new_from_balance = from_balance - amount;
            let new_to_balance = to_balance + amount;
            let new_allowance = allowance - amount;

            let mut result = Tip20Result::success_with_bool(true);
            result.storage_writes.push((from_slot, new_from_balance));
            result.storage_writes.push((to_slot, new_to_balance));
            result.storage_writes.push((allowance_slot, new_allowance));
            Some(result)
        }

        selectors::NAME | selectors::SYMBOL => {
            // Return "TIP20" as a simple string for both name and symbol
            // ABI-encoded string: offset (32) + length (32) + data (padded to 32)
            let mut data = vec![0u8; 96];
            // Offset to string data (0x20 = 32)
            data[31] = 0x20;
            // String length (5 for "TIP20")
            data[63] = 5;
            // String data "TIP20"
            data[64] = b'T';
            data[65] = b'I';
            data[66] = b'P';
            data[67] = b'2';
            data[68] = b'0';
            Some(Tip20Result::success_with_data(Bytes::from(data)))
        }

        _ => {
            // Unknown selector - return success with empty data (fallback)
            Some(Tip20Result::success_with_bool(true))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_tip20_address() {
        let tip20_addr = "0x20c0000000000000000000000000000000000000".parse::<Address>().unwrap();
        let normal_addr = "0x1234567890123456789012345678901234567890".parse::<Address>().unwrap();

        assert!(is_tip20_address(&tip20_addr));
        assert!(!is_tip20_address(&normal_addr));
    }

    #[test]
    fn test_balance_slot_computation() {
        let owner = "0x1234567890123456789012345678901234567890".parse::<Address>().unwrap();
        let slot = balance_slot(owner);
        // Slot should be non-zero and deterministic
        assert!(slot != U256::ZERO);
    }

    #[test]
    fn test_execute_decimals() {
        let calldata = [0x31, 0x3c, 0xe5, 0x67]; // decimals()
        let caller = Address::ZERO;

        let result = execute_tip20_call(&calldata, caller, |_| U256::ZERO).unwrap();
        assert!(result.success);
        assert_eq!(result.return_data.len(), 32);
        // Should return 6
        assert_eq!(result.return_data[31], 6);
    }

    #[test]
    fn test_execute_balance_of() {
        let mut calldata = vec![0x70, 0xa0, 0x82, 0x31]; // balanceOf selector
        // Add address parameter (32 bytes, right-aligned)
        calldata.extend_from_slice(&[0u8; 12]);
        calldata.extend_from_slice(&[0x12; 20]);

        let caller = Address::ZERO;
        let expected_balance = U256::from(1000);

        let result = execute_tip20_call(&calldata, caller, |_| expected_balance).unwrap();
        assert!(result.success);
        assert_eq!(result.return_data.len(), 32);
        let returned = U256::from_be_bytes::<32>(result.return_data.as_ref().try_into().unwrap());
        assert_eq!(returned, expected_balance);
    }
}
