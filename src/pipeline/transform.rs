//! Normalized model -> EVM JSON-RPC shapes.

use std::sync::Arc;

use alloy_primitives::{Address, B256, Bytes, U64, U256, address, keccak256};
use serde_json::json;

use super::Transformer;
use super::mapper::{DefaultMapper, IdMapper};
use super::normalized::{EventValue, NormalizedBlock, NormalizedEvent, NormalizedTx};
use crate::chains::ChainInfo;
use crate::error::Result;
use crate::types::{Block, BlockTransactions, Log, NativeMeta, Receipt, Transaction};

/// Address used as the `emitter` of native-coin events (the common
/// "native token" placeholder used by DEX aggregators and bridges).
pub const NATIVE_TOKEN_ADDRESS: Address = address!("0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE");

/// How native amounts are expressed in `value`, fees and native-coin event data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueScale {
    /// Keep base units (lamports, satoshis).
    Native,
    /// Rescale to this many decimals, e.g. `Decimals(18)` makes 1 SOL read as 1e18,
    /// like 1 ETH in wei, so EVM tooling formats it correctly.
    Decimals(u8),
}

impl Default for ValueScale {
    fn default() -> Self {
        ValueScale::Decimals(18)
    }
}

impl ValueScale {
    pub fn apply(self, amount: u128, native_decimals: u8) -> U256 {
        let v = U256::from(amount);
        match self {
            ValueScale::Native => v,
            ValueScale::Decimals(target) if target >= native_decimals => {
                v * U256::from(10u64).pow(U256::from(target - native_decimals))
            }
            ValueScale::Decimals(target) => {
                v / U256::from(10u64).pow(U256::from(native_decimals - target))
            }
        }
    }
}

/// The default transformer: produces [`Block`], [`Transaction`], [`Receipt`] and [`Log`].
///
/// Gas modelling: non-EVM chains have no gas, so the fee is expressed as
/// `gasUsed = fee` with `effectiveGasPrice = 1`. `gasUsed * effectiveGasPrice` therefore
/// equals the real fee (after [`ValueScale`]), which is what fee-tracking code computes.
#[derive(Debug, Clone)]
pub struct EvmTransformer {
    pub mapper: Arc<dyn IdMapper>,
    pub scale: ValueScale,
}

impl Default for EvmTransformer {
    fn default() -> Self {
        Self::new(DefaultMapper::default())
    }
}

impl EvmTransformer {
    pub fn new(mapper: impl IdMapper + 'static) -> Self {
        Self {
            mapper: Arc::new(mapper),
            scale: ValueScale::default(),
        }
    }

    pub fn with_scale(mut self, scale: ValueScale) -> Self {
        self.scale = scale;
        self
    }

    fn amount(&self, chain: &ChainInfo, v: u128) -> U256 {
        self.scale.apply(v, chain.native_decimals)
    }

    fn native_meta(&self, chain: &ChainInfo, tx: &NormalizedTx) -> NativeMeta {
        NativeMeta {
            chain: chain.key.clone(),
            id: tx.id.clone(),
            from: tx.from.clone(),
            to: tx.to.clone(),
            value: Some(tx.value.to_string()),
            extra: tx.extra.clone(),
        }
    }

    fn event_to_log(&self, chain: &ChainInfo, ev: &NormalizedEvent) -> Log {
        let native = ev.emitter.is_none();
        let mut topics = vec![keccak256(ev.signature.as_bytes())];
        topics.extend(
            ev.indexed
                .iter()
                .map(|a| self.mapper.address(a).into_word()),
        );
        let mut data = Vec::with_capacity(32 * ev.values.len());
        for v in &ev.values {
            let word = match *v {
                EventValue::Uint(x) if native => self.amount(chain, x),
                EventValue::Uint(x) => U256::from(x),
                EventValue::Int(x) => {
                    let mag = if native {
                        self.amount(chain, x.unsigned_abs())
                    } else {
                        U256::from(x.unsigned_abs())
                    };
                    // int256 two's complement
                    if x < 0 { mag.wrapping_neg() } else { mag }
                }
            };
            data.extend_from_slice(&word.to_be_bytes::<32>());
        }
        Log {
            address: ev
                .emitter
                .as_deref()
                .map(|e| self.mapper.address(e))
                .unwrap_or(NATIVE_TOKEN_ADDRESS),
            topics,
            data: Bytes::from(data),
            block_number: None,
            block_hash: None,
            transaction_hash: None,
            transaction_index: None,
            log_index: None,
            removed: false,
        }
    }
}

fn u64_opt(v: Option<u64>) -> Option<U64> {
    v.map(U64::from)
}

impl Transformer for EvmTransformer {
    type Block = Block;
    type Tx = Transaction;
    type Receipt = Receipt;
    type Log = Log;

    fn block(&self, chain: &ChainInfo, b: &NormalizedBlock, full: bool) -> Result<Block> {
        let transactions = if full {
            BlockTransactions::Full(
                b.transactions
                    .iter()
                    .map(|t| self.transaction(chain, t))
                    .collect::<Result<_>>()?,
            )
        } else {
            BlockTransactions::Hashes(
                b.transactions
                    .iter()
                    .map(|t| self.mapper.hash(&t.id))
                    .collect(),
            )
        };
        Ok(Block {
            number: U64::from(b.height),
            hash: self.mapper.hash(&b.hash),
            parent_hash: if b.parent_hash.is_empty() {
                B256::ZERO
            } else {
                self.mapper.hash(&b.parent_hash)
            },
            timestamp: U64::from(b.timestamp),
            miner: b
                .producer
                .as_deref()
                .map(|p| self.mapper.address(p))
                .unwrap_or_default(),
            gas_used: U256::ZERO,
            gas_limit: U256::ZERO,
            base_fee_per_gas: None,
            transactions,
            native: Some(NativeMeta {
                chain: chain.key.clone(),
                id: b.hash.clone(),
                from: None,
                to: None,
                value: None,
                extra: json!({
                    "producer": b.producer,
                    "parentHash": b.parent_hash,
                    "info": b.extra,
                }),
            }),
            other: Default::default(),
        })
    }

    fn transaction(&self, chain: &ChainInfo, tx: &NormalizedTx) -> Result<Transaction> {
        let fee = self.amount(chain, tx.fee.unwrap_or(0));
        Ok(Transaction {
            hash: self.mapper.hash(&tx.id),
            nonce: U256::ZERO,
            block_hash: tx.block_hash.as_deref().map(|h| self.mapper.hash(h)),
            block_number: u64_opt(tx.block_height),
            transaction_index: u64_opt(tx.index),
            from: tx
                .from
                .as_deref()
                .map(|a| self.mapper.address(a))
                .unwrap_or_default(),
            to: tx.to.as_deref().map(|a| self.mapper.address(a)),
            value: self.amount(chain, tx.value),
            gas: fee,
            gas_price: Some(U256::from(1)),
            max_fee_per_gas: None,
            max_priority_fee_per_gas: None,
            input: Bytes::from(tx.input.clone()),
            chain_id: u64_opt(chain.evm_chain_id),
            tx_type: None,
            native: Some(self.native_meta(chain, tx)),
            other: Default::default(),
        })
    }

    fn receipt(&self, chain: &ChainInfo, tx: &NormalizedTx) -> Result<Receipt> {
        let t = self.transaction(chain, tx)?;
        let logs = self.logs(chain, tx, 0)?;
        Ok(Receipt {
            transaction_hash: t.hash,
            transaction_index: t.transaction_index,
            block_hash: t.block_hash,
            block_number: t.block_number,
            from: t.from,
            to: t.to,
            contract_address: None,
            cumulative_gas_used: t.gas,
            gas_used: t.gas,
            effective_gas_price: Some(U256::from(1)),
            status: tx.success.map(|ok| U64::from(ok as u64)),
            logs,
            native: t.native,
            other: Default::default(),
        })
    }

    fn logs(&self, chain: &ChainInfo, tx: &NormalizedTx, first_index: u64) -> Result<Vec<Log>> {
        let tx_hash = self.mapper.hash(&tx.id);
        let block_hash = tx.block_hash.as_deref().map(|h| self.mapper.hash(h));
        Ok(tx
            .events
            .iter()
            .enumerate()
            .map(|(i, ev)| Log {
                block_number: u64_opt(tx.block_height),
                block_hash,
                transaction_hash: Some(tx_hash),
                transaction_index: u64_opt(tx.index),
                log_index: Some(U64::from(first_index + i as u64)),
                ..self.event_to_log(chain, ev)
            })
            .collect())
    }

    fn native_tx_id(&self, hash: &B256) -> Option<String> {
        self.mapper.native_hash(hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling() {
        let one = U256::from(10u64).pow(U256::from(18));
        assert_eq!(ValueScale::Decimals(18).apply(1_000_000_000, 9), one); // 1 SOL
        assert_eq!(ValueScale::Decimals(18).apply(100_000_000, 8), one); // 1 BTC
        assert_eq!(
            ValueScale::Decimals(6).apply(1_234_567_890, 9),
            U256::from(1_234_567u64)
        );
        assert_eq!(ValueScale::Native.apply(42, 8), U256::from(42));
    }

    #[test]
    fn negative_native_event_is_scaled_twos_complement() {
        let t = EvmTransformer::default();
        let chain = ChainInfo::solana_mainnet();
        let ev = NormalizedEvent::balance_change(None, "x".into(), -1_000_000_000);
        let log = t.event_to_log(&chain, &ev);
        let word = U256::from_be_slice(&log.data);
        assert_eq!(word.wrapping_neg(), U256::from(10u64).pow(U256::from(18)));
        assert_eq!(log.address, NATIVE_TOKEN_ADDRESS);
    }
}
