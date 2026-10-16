//! EVM-shaped output types.
//!
//! These mirror the JSON returned by `eth_getBlockByNumber`, `eth_getTransactionByHash`,
//! `eth_getTransactionReceipt` and `eth_getLogs`. Fields that not every EVM chain returns are
//! optional, and any field this crate does not model (e.g. Arbitrum's `l1BlockNumber`,
//! OP-stack deposit fields) is preserved in `other` so nothing is lost on round-trips.
//!
//! Values produced from a non-EVM chain carry a [`NativeMeta`] in `native` (serialized as
//! `_native`) with the original identifiers, so the EVM view is never a dead end.

use std::collections::BTreeMap;

use alloy_primitives::{Address, B256, Bytes, U64, U256};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Extra JSON fields that are not modelled explicitly.
pub type OtherFields = BTreeMap<String, Value>;

/// Which block to query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BlockId {
    Number(u64),
    #[default]
    Latest,
    Earliest,
    Pending,
    Safe,
    Finalized,
}

impl BlockId {
    /// JSON-RPC parameter form (`"latest"`, `"0x10"`, ...).
    pub fn to_param(self) -> Value {
        match self {
            BlockId::Number(n) => Value::String(format!("{n:#x}")),
            BlockId::Latest => "latest".into(),
            BlockId::Earliest => "earliest".into(),
            BlockId::Pending => "pending".into(),
            BlockId::Safe => "safe".into(),
            BlockId::Finalized => "finalized".into(),
        }
    }
}

impl From<u64> for BlockId {
    fn from(n: u64) -> Self {
        BlockId::Number(n)
    }
}

/// Where a value came from when it was translated from a non-EVM chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeMeta {
    /// Chain key, e.g. `"solana"`.
    pub chain: String,
    /// The original identifier (Solana signature / slot hash, Bitcoin txid, ...).
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Native value in the chain's base unit (lamports, satoshis), before any scaling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Chain-specific extras the normalizer chose to keep.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

/// `transactions` in a block: hashes only, or full objects.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BlockTransactions {
    Hashes(Vec<B256>),
    Full(Vec<Transaction>),
}

impl Default for BlockTransactions {
    fn default() -> Self {
        BlockTransactions::Hashes(Vec::new())
    }
}

impl BlockTransactions {
    pub fn len(&self) -> usize {
        match self {
            BlockTransactions::Hashes(h) => h.len(),
            BlockTransactions::Full(t) => t.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn hashes(&self) -> Vec<B256> {
        match self {
            BlockTransactions::Hashes(h) => h.clone(),
            BlockTransactions::Full(t) => t.iter().map(|t| t.hash).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub number: U64,
    pub hash: B256,
    pub parent_hash: B256,
    pub timestamp: U64,
    #[serde(default)]
    pub miner: Address,
    #[serde(default)]
    pub gas_used: U256,
    #[serde(default)]
    pub gas_limit: U256,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_fee_per_gas: Option<U256>,
    #[serde(default)]
    pub transactions: BlockTransactions,
    #[serde(rename = "_native", default, skip_serializing_if = "Option::is_none")]
    pub native: Option<NativeMeta>,
    #[serde(flatten)]
    pub other: OtherFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transaction {
    pub hash: B256,
    #[serde(default)]
    pub nonce: U256,
    #[serde(default)]
    pub block_hash: Option<B256>,
    #[serde(default)]
    pub block_number: Option<U64>,
    #[serde(default)]
    pub transaction_index: Option<U64>,
    #[serde(default)]
    pub from: Address,
    #[serde(default)]
    pub to: Option<Address>,
    #[serde(default)]
    pub value: U256,
    #[serde(default)]
    pub gas: U256,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gas_price: Option<U256>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fee_per_gas: Option<U256>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_priority_fee_per_gas: Option<U256>,
    #[serde(default)]
    pub input: Bytes,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_id: Option<U64>,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub tx_type: Option<U64>,
    #[serde(rename = "_native", default, skip_serializing_if = "Option::is_none")]
    pub native: Option<NativeMeta>,
    #[serde(flatten)]
    pub other: OtherFields,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Receipt {
    pub transaction_hash: B256,
    #[serde(default)]
    pub transaction_index: Option<U64>,
    #[serde(default)]
    pub block_hash: Option<B256>,
    #[serde(default)]
    pub block_number: Option<U64>,
    #[serde(default)]
    pub from: Address,
    #[serde(default)]
    pub to: Option<Address>,
    #[serde(default)]
    pub contract_address: Option<Address>,
    #[serde(default)]
    pub cumulative_gas_used: U256,
    #[serde(default)]
    pub gas_used: U256,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_gas_price: Option<U256>,
    /// `1` success, `0` failure (post-Byzantium).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<U64>,
    #[serde(default)]
    pub logs: Vec<Log>,
    #[serde(rename = "_native", default, skip_serializing_if = "Option::is_none")]
    pub native: Option<NativeMeta>,
    #[serde(flatten)]
    pub other: OtherFields,
}

impl Receipt {
    pub fn succeeded(&self) -> Option<bool> {
        self.status.map(|s| s == U64::from(1))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Log {
    pub address: Address,
    #[serde(default)]
    pub topics: Vec<B256>,
    #[serde(default)]
    pub data: Bytes,
    #[serde(default)]
    pub block_number: Option<U64>,
    #[serde(default)]
    pub block_hash: Option<B256>,
    #[serde(default)]
    pub transaction_hash: Option<B256>,
    #[serde(default)]
    pub transaction_index: Option<U64>,
    #[serde(default)]
    pub log_index: Option<U64>,
    #[serde(default)]
    pub removed: bool,
}

/// `eth_getLogs` filter.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_block: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_block: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_hash: Option<B256>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub address: Vec<Address>,
    /// Positional topic filters; `None` matches anything, a list matches any of its entries.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub topics: Vec<Option<Vec<B256>>>,
}

impl Filter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_block(mut self, b: impl Into<BlockId>) -> Self {
        self.from_block = Some(b.into().to_param());
        self
    }

    pub fn to_block(mut self, b: impl Into<BlockId>) -> Self {
        self.to_block = Some(b.into().to_param());
        self
    }

    pub fn at_block_hash(mut self, h: B256) -> Self {
        self.block_hash = Some(h);
        self
    }

    pub fn address(mut self, a: Address) -> Self {
        self.address.push(a);
        self
    }

    /// Match `topic` at position `index` (0 = event signature).
    pub fn topic(mut self, index: usize, topic: B256) -> Self {
        if self.topics.len() <= index {
            self.topics.resize(index + 1, None);
        }
        self.topics[index].get_or_insert_with(Vec::new).push(topic);
        self
    }

    /// Shorthand for `topic(0, keccak256(signature))`.
    pub fn event(self, signature: &str) -> Self {
        self.topic(0, alloy_primitives::keccak256(signature.as_bytes()))
    }

    /// Whether `log` passes the address and topic parts of this filter
    /// (block ranges are checked by the caller).
    pub fn matches(&self, log: &Log) -> bool {
        if !self.address.is_empty() && !self.address.contains(&log.address) {
            return false;
        }
        self.topics.iter().enumerate().all(|(i, want)| match want {
            None => true,
            Some(options) => log.topics.get(i).is_some_and(|t| options.contains(t)),
        })
    }
}

/// `eth_call` / `eth_estimateGas` request object.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CallRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Address>,
    pub to: Address,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gas: Option<U256>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<U256>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Bytes>,
}

impl CallRequest {
    pub fn new(to: Address, input: impl Into<Bytes>) -> Self {
        Self {
            to,
            input: Some(input.into()),
            ..Default::default()
        }
    }
}
