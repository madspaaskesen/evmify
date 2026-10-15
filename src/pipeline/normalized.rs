//! The chain-agnostic middle layer.
//!
//! Normalizers translate native payloads into these types; transformers translate these
//! types into an output format. Identifiers stay as native strings here: turning them into
//! 20-byte addresses / 32-byte hashes is the transformer's job, so a different output format
//! is free to keep them as they are.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Use as an indexed event argument to mean "nobody" (mint / coinbase / burn).
/// The default mapper turns it into `0x000…000`.
pub const NULL_ACCOUNT: &str = "0x0000000000000000000000000000000000000000";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedBlock {
    /// Block number in the sense the chain uses for lookups (Solana: slot).
    pub height: u64,
    pub hash: String,
    pub parent_hash: String,
    /// Unix seconds; 0 if the chain did not report it.
    pub timestamp: u64,
    /// Leader / miner / validator, if known.
    pub producer: Option<String>,
    pub transactions: Vec<NormalizedTx>,
    #[serde(default)]
    pub extra: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedTx {
    pub id: String,
    pub block_height: Option<u64>,
    pub block_hash: Option<String>,
    pub index: Option<u64>,
    /// Payer / signer / first input owner.
    pub from: Option<String>,
    /// Primary recipient, or the program/contract that was called.
    pub to: Option<String>,
    /// Native coin moved to `to`, in base units (lamports, satoshis).
    pub value: u128,
    /// Fee paid, in base units.
    pub fee: Option<u128>,
    /// `None` when the chain cannot tell (e.g. unconfirmed).
    pub success: Option<bool>,
    #[serde(default)]
    pub input: Vec<u8>,
    #[serde(default)]
    pub events: Vec<NormalizedEvent>,
    #[serde(default)]
    pub extra: Value,
}

/// An EVM-log-shaped event: a Solidity-style signature, indexed accounts, and values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedEvent {
    /// Token / program that "emitted" it. `None` means the chain's native coin; transformers
    /// may scale the values of such events the same way they scale `value`.
    pub emitter: Option<String>,
    /// e.g. `"Transfer(address,address,uint256)"`; topic 0 is derived from it.
    pub signature: String,
    /// Native account ids that become indexed `address` topics.
    pub indexed: Vec<String>,
    /// Non-indexed values, ABI-encoded into `data` by the transformer.
    pub values: Vec<EventValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventValue {
    Uint(u128),
    Int(i128),
}

impl NormalizedEvent {
    pub const TRANSFER: &'static str = "Transfer(address,address,uint256)";
    pub const BALANCE_CHANGE: &'static str = "BalanceChange(address,int256)";

    /// ERC-20 compatible `Transfer`. `emitter = None` for the native coin.
    pub fn transfer(emitter: Option<String>, from: String, to: String, amount: u128) -> Self {
        Self {
            emitter,
            signature: Self::TRANSFER.into(),
            indexed: vec![from, to],
            values: vec![EventValue::Uint(amount)],
        }
    }

    /// Net balance change of `account` within one transaction.
    pub fn balance_change(emitter: Option<String>, account: String, delta: i128) -> Self {
        Self {
            emitter,
            signature: Self::BALANCE_CHANGE.into(),
            indexed: vec![account],
            values: vec![EventValue::Int(delta)],
        }
    }
}
