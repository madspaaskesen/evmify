//! Native identifiers -> 20-byte addresses and 32-byte hashes.

use std::collections::{HashMap, VecDeque};
use std::fmt::Debug;
use std::sync::Mutex;

use alloy_primitives::{Address, B256, keccak256};

/// How native ids become EVM addresses and hashes, and (where possible) back again.
pub trait IdMapper: Send + Sync + Debug {
    fn address(&self, native: &str) -> Address;

    fn hash(&self, native: &str) -> B256;

    /// Recover the native id from a hash produced by [`IdMapper::hash`], if this mapper can.
    fn native_hash(&self, hash: &B256) -> Option<String>;
}

/// Deterministic, stateless mapping.
///
/// * `0x` + 40 hex chars is parsed as an address; anything else becomes
///   `keccak256(utf8(native))[12..]`.
/// * 64 hex chars (with or without `0x`) is parsed as a hash, so Bitcoin txids and block
///   hashes keep their value; anything else (Solana's base58 signatures) becomes
///   `keccak256(utf8(native))`.
///
/// Reverse lookup only works for the hex case, so set `hex_ids_are_native` for chains
/// whose ids are hex (Bitcoin) and wrap it in [`IndexedMapper`] for the others.
#[derive(Debug, Clone, Default)]
pub struct DefaultMapper {
    pub hex_ids_are_native: bool,
}

impl DefaultMapper {
    pub fn new(hex_ids_are_native: bool) -> Self {
        Self { hex_ids_are_native }
    }
}

fn is_hex(s: &str, len: usize) -> bool {
    s.len() == len && s.bytes().all(|b| b.is_ascii_hexdigit())
}

impl IdMapper for DefaultMapper {
    fn address(&self, native: &str) -> Address {
        match native.strip_prefix("0x") {
            Some(hex) if is_hex(hex, 40) => hex.parse().expect("validated hex"),
            _ => Address::from_slice(&keccak256(native.as_bytes())[12..]),
        }
    }

    fn hash(&self, native: &str) -> B256 {
        let hex = native.strip_prefix("0x").unwrap_or(native);
        if is_hex(hex, 64) {
            hex.parse().expect("validated hex")
        } else {
            keccak256(native.as_bytes())
        }
    }

    fn native_hash(&self, hash: &B256) -> Option<String> {
        // Strip the 0x: Bitcoin RPC wants bare hex.
        self.hex_ids_are_native
            .then(|| alloy_primitives::hex::encode(hash.as_slice()))
    }
}

/// Wraps another mapper and remembers the last `capacity` hashes it produced, so hashes
/// seen in earlier blocks can be looked up again even when the mapping is one-way.
///
/// For a long-running indexer, implement [`IdMapper`] over your own database instead.
#[derive(Debug)]
pub struct IndexedMapper<M> {
    inner: M,
    capacity: usize,
    seen: Mutex<(HashMap<B256, String>, VecDeque<B256>)>,
}

impl<M: IdMapper> IndexedMapper<M> {
    pub fn new(inner: M, capacity: usize) -> Self {
        Self {
            inner,
            capacity: capacity.max(1),
            seen: Mutex::new((HashMap::new(), VecDeque::new())),
        }
    }
}

impl<M: IdMapper> IdMapper for IndexedMapper<M> {
    fn address(&self, native: &str) -> Address {
        self.inner.address(native)
    }

    fn hash(&self, native: &str) -> B256 {
        let h = self.inner.hash(native);
        let mut guard = self.seen.lock().unwrap();
        let (map, order) = &mut *guard;
        if map.insert(h, native.to_string()).is_none() {
            order.push_back(h);
            if order.len() > self.capacity
                && let Some(old) = order.pop_front()
            {
                map.remove(&old);
            }
        }
        h
    }

    fn native_hash(&self, hash: &B256) -> Option<String> {
        self.seen
            .lock()
            .unwrap()
            .0
            .get(hash)
            .cloned()
            .or_else(|| self.inner.native_hash(hash))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_passes_through_everything_else_hashes() {
        let m = DefaultMapper::new(true);
        let a = "0x00000000000000000000000000000000000000ff";
        assert_eq!(m.address(a), a.parse::<Address>().unwrap());
        assert_eq!(m.address(crate::pipeline::NULL_ACCOUNT), Address::ZERO);
        assert_eq!(
            m.address("abc"),
            Address::from_slice(&keccak256("abc")[12..])
        );

        let txid = "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16";
        let h = m.hash(txid);
        assert_eq!(m.native_hash(&h).as_deref(), Some(txid));
        assert_eq!(DefaultMapper::new(false).native_hash(&h), None);
    }

    #[test]
    fn indexed_mapper_remembers_and_evicts() {
        let m = IndexedMapper::new(DefaultMapper::default(), 2);
        let (a, b, c) = (m.hash("sigA"), m.hash("sigB"), m.hash("sigC"));
        assert_eq!(m.native_hash(&a), None, "evicted");
        assert_eq!(m.native_hash(&b).as_deref(), Some("sigB"));
        assert_eq!(m.native_hash(&c).as_deref(), Some("sigC"));
    }
}
