//! Solana: `getSlot` / `getBlock` / `getTransaction`, normalized into EVM shape.
//!
//! Mapping:
//! * block number = slot; skipped slots return `None`
//! * `from` = fee payer; for a plain SOL transfer `to` / `value` are the recipient and
//!   amount, otherwise `to` is the first invoked program and `value` is 0
//! * every SOL and SPL-token balance change becomes a `BalanceChange(address,int256)` log;
//!   when a mint has exactly one sender and one receiver of equal amounts, an ERC-20
//!   `Transfer(address,address,uint256)` log is emitted too, with the mint as log address
//! * instructions, compute units, program logs, the transaction version and (v1) the
//!   `transactionConfig` compute budget are kept in `_native.extra`
//! * legacy, v0 (address lookup tables) and v1 transactions are all supported

use std::collections::BTreeMap;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::chains::ChainInfo;
use crate::error::{Error, Result};
use crate::pipeline::{
    Adapter, DefaultMapper, EvmTransformer, IndexedMapper, NormalizedBlock, NormalizedEvent,
    NormalizedTx, Normalizer, Pipeline,
};
use crate::transport::Transport;

const COMPUTE_BUDGET: &str = "ComputeBudget111111111111111111111111111111";

/// RPC error codes meaning "no block at this slot" rather than a real failure.
const NO_BLOCK_CODES: &[i64] = &[-32004, -32007, -32009];

// ---------------------------------------------------------------------------------------------
// Raw payloads (only the fields we use; serde ignores the rest)
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolanaBlock {
    /// Not part of the RPC payload; filled in by the adapter.
    #[serde(default)]
    pub slot: u64,
    pub blockhash: String,
    pub previous_blockhash: String,
    pub parent_slot: u64,
    pub block_time: Option<i64>,
    pub block_height: Option<u64>,
    #[serde(default)]
    pub transactions: Vec<SolanaTx>,
    #[serde(default)]
    pub rewards: Option<Vec<SolanaReward>>,
}

/// A transaction with its status meta, as found in `getBlock` and `getTransaction`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolanaTx {
    /// Present in `getTransaction` responses.
    #[serde(default)]
    pub slot: Option<u64>,
    #[serde(default)]
    pub block_time: Option<i64>,
    /// `"legacy"`, `0` or `1`.
    #[serde(default)]
    pub version: Option<Value>,
    pub transaction: SolanaTxBody,
    pub meta: Option<SolanaMeta>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SolanaTxBody {
    pub signatures: Vec<String>,
    pub message: SolanaMessage,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolanaMessage {
    pub account_keys: Vec<String>,
    #[serde(default)]
    pub instructions: Vec<SolanaInstruction>,
    /// v1 transactions: compute budget and total priority fee, which replace the
    /// ComputeBudget program instructions of legacy/v0 transactions.
    #[serde(default)]
    pub transaction_config: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolanaInstruction {
    pub program_id_index: usize,
    #[serde(default)]
    pub accounts: Vec<usize>,
    #[serde(default)]
    pub data: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolanaMeta {
    pub err: Option<Value>,
    pub fee: u64,
    pub pre_balances: Vec<u64>,
    pub post_balances: Vec<u64>,
    #[serde(default)]
    pub pre_token_balances: Option<Vec<SolanaTokenBalance>>,
    #[serde(default)]
    pub post_token_balances: Option<Vec<SolanaTokenBalance>>,
    #[serde(default)]
    pub log_messages: Option<Vec<String>>,
    #[serde(default)]
    pub loaded_addresses: Option<SolanaLoadedAddresses>,
    #[serde(default)]
    pub compute_units_consumed: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SolanaLoadedAddresses {
    #[serde(default)]
    pub writable: Vec<String>,
    #[serde(default)]
    pub readonly: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolanaTokenBalance {
    pub account_index: usize,
    pub mint: String,
    #[serde(default)]
    pub owner: Option<String>,
    pub ui_token_amount: SolanaTokenAmount,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SolanaTokenAmount {
    pub amount: String,
    pub decimals: u8,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolanaReward {
    pub pubkey: String,
    pub lamports: i64,
    #[serde(default)]
    pub reward_type: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------------------------

/// Commitment level used for every request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Commitment {
    Confirmed,
    Finalized,
}

impl Commitment {
    fn as_str(self) -> &'static str {
        match self {
            Commitment::Confirmed => "confirmed",
            Commitment::Finalized => "finalized",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SolanaAdapter<T> {
    transport: T,
    chain: ChainInfo,
    pub commitment: Commitment,
    /// Highest transaction version requested from the node. The node fails the whole
    /// `getBlock` (error -32015) if a block holds a newer one, so keep this at the newest
    /// version the normalizer understands. Default: 1.
    pub max_transaction_version: u8,
}

/// Newest Solana transaction version this crate can normalize.
pub const MAX_SUPPORTED_TRANSACTION_VERSION: u8 = 1;

impl<T: Transport> SolanaAdapter<T> {
    pub fn new(transport: T, chain: ChainInfo) -> Self {
        Self {
            transport,
            chain,
            commitment: Commitment::Confirmed,
            max_transaction_version: MAX_SUPPORTED_TRANSACTION_VERSION,
        }
    }

    pub fn mainnet(transport: T) -> Self {
        Self::new(transport, ChainInfo::solana_mainnet())
    }

    async fn get_slot(&self, commitment: Commitment) -> Result<u64> {
        let v = self
            .transport
            .request("getSlot", json!([{ "commitment": commitment.as_str() }]))
            .await?;
        serde_json::from_value(v).map_err(|e| Error::decode("getSlot", e))
    }
}

#[async_trait]
impl<T: Transport> Adapter for SolanaAdapter<T> {
    type RawBlock = SolanaBlock;
    type RawTx = SolanaTx;

    fn chain(&self) -> &ChainInfo {
        &self.chain
    }

    async fn head(&self) -> Result<u64> {
        self.get_slot(self.commitment).await
    }

    async fn finalized_head(&self) -> Result<u64> {
        self.get_slot(Commitment::Finalized).await
    }

    async fn block(&self, slot: u64) -> Result<Option<SolanaBlock>> {
        let params = json!([slot, {
            "encoding": "json",
            "transactionDetails": "full",
            "rewards": true,
            "maxSupportedTransactionVersion": self.max_transaction_version,
            "commitment": self.commitment.as_str(),
        }]);
        let v = match self.transport.request("getBlock", params).await {
            Ok(v) => v,
            Err(Error::Rpc { code, .. }) if NO_BLOCK_CODES.contains(&code) => return Ok(None),
            Err(e) => return Err(e),
        };
        if v.is_null() {
            return Ok(None);
        }
        let mut block: SolanaBlock =
            serde_json::from_value(v).map_err(|e| Error::decode("getBlock", e))?;
        block.slot = slot;
        Ok(Some(block))
    }

    async fn transaction(&self, signature: &str) -> Result<Option<SolanaTx>> {
        let params = json!([signature, {
            "encoding": "json",
            "maxSupportedTransactionVersion": self.max_transaction_version,
            "commitment": self.commitment.as_str(),
        }]);
        let v = self.transport.request("getTransaction", params).await?;
        if v.is_null() {
            return Ok(None);
        }
        serde_json::from_value(v)
            .map(Some)
            .map_err(|e| Error::decode("getTransaction", e))
    }
}

// ---------------------------------------------------------------------------------------------
// Normalizer
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct SolanaNormalizer;

impl SolanaNormalizer {
    fn tx(
        &self,
        raw: SolanaTx,
        slot: Option<u64>,
        block_hash: Option<String>,
        index: Option<u64>,
    ) -> Result<NormalizedTx> {
        let body = raw.transaction;
        let version = raw.version.clone();
        let tx_config = body.message.transaction_config.clone();
        let id = body
            .signatures
            .first()
            .cloned()
            .ok_or_else(|| Error::Normalize("transaction without signatures".into()))?;

        // Full account list: static keys, then lookup-table writable, then readonly.
        let mut keys = body.message.account_keys;
        if let Some(meta) = &raw.meta
            && let Some(loaded) = &meta.loaded_addresses
        {
            keys.extend(loaded.writable.iter().cloned());
            keys.extend(loaded.readonly.iter().cloned());
        }
        let key = |i: usize| keys.get(i).cloned().unwrap_or_default();
        let payer = keys.first().cloned();

        let instructions: Vec<Value> = body
            .message
            .instructions
            .iter()
            .map(|ix| {
                json!({
                    "program": key(ix.program_id_index),
                    "accounts": ix.accounts.iter().map(|&a| key(a)).collect::<Vec<_>>(),
                    "data": ix.data,
                })
            })
            .collect();
        let main_program = body
            .message
            .instructions
            .iter()
            .map(|ix| key(ix.program_id_index))
            .find(|p| p != COMPUTE_BUDGET);

        let mut events = Vec::new();
        let (mut to, mut value) = (main_program, 0u128);
        let mut extra = json!({ "instructions": instructions });

        if let Some(meta) = &raw.meta {
            // Native SOL balance changes, with the fee added back to the payer so that only
            // value movements remain.
            let sol: Vec<(String, i128)> = meta
                .pre_balances
                .iter()
                .zip(&meta.post_balances)
                .enumerate()
                .map(|(i, (&pre, &post))| {
                    let mut d = post as i128 - pre as i128;
                    if i == 0 {
                        d += meta.fee as i128;
                    }
                    (key(i), d)
                })
                .filter(|(_, d)| *d != 0)
                .collect();
            if let Some((_, receiver, amount)) = single_transfer(&sol) {
                to = Some(receiver);
                value = amount;
            }
            push_events(&mut events, None, &sol);

            // SPL token balance changes, grouped per mint.
            let mut per_mint: BTreeMap<String, BTreeMap<String, i128>> = BTreeMap::new();
            let mut apply = |list: &Option<Vec<SolanaTokenBalance>>, sign: i128| {
                for b in list.iter().flatten() {
                    let amount: i128 = b.ui_token_amount.amount.parse().unwrap_or(0);
                    let holder = b.owner.clone().unwrap_or_else(|| key(b.account_index));
                    *per_mint
                        .entry(b.mint.clone())
                        .or_default()
                        .entry(holder)
                        .or_default() += sign * amount;
                }
            };
            apply(&meta.pre_token_balances, -1);
            apply(&meta.post_token_balances, 1);
            for (mint, holders) in per_mint {
                let deltas: Vec<(String, i128)> =
                    holders.into_iter().filter(|(_, d)| *d != 0).collect();
                push_events(&mut events, Some(mint), &deltas);
            }

            extra["computeUnits"] = json!(meta.compute_units_consumed);
            extra["logMessages"] = json!(meta.log_messages);
            if let Some(err) = &meta.err {
                extra["error"] = err.clone();
            }
        }

        let slot = slot.or(raw.slot);
        extra["slot"] = json!(slot);
        extra["version"] = version.unwrap_or(Value::Null);
        if let Some(config) = tx_config {
            extra["transactionConfig"] = config;
        }
        Ok(NormalizedTx {
            id,
            block_height: slot,
            block_hash,
            index,
            from: payer,
            to,
            value,
            fee: raw.meta.as_ref().map(|m| m.fee as u128),
            success: raw.meta.as_ref().map(|m| m.err.is_none()),
            input: Vec::new(),
            events,
            extra,
        })
    }
}

/// Exactly one sender and one receiver of the same amount -> `(from, to, amount)`.
fn single_transfer(deltas: &[(String, i128)]) -> Option<(String, String, u128)> {
    match deltas {
        [(a, da), (b, db)] if *da == -*db => {
            if *da < 0 {
                Some((a.clone(), b.clone(), db.unsigned_abs()))
            } else {
                Some((b.clone(), a.clone(), da.unsigned_abs()))
            }
        }
        _ => None,
    }
}

fn push_events(
    events: &mut Vec<NormalizedEvent>,
    emitter: Option<String>,
    deltas: &[(String, i128)],
) {
    if let Some((from, to, amount)) = single_transfer(deltas) {
        events.push(NormalizedEvent::transfer(emitter.clone(), from, to, amount));
    }
    for (account, d) in deltas {
        events.push(NormalizedEvent::balance_change(
            emitter.clone(),
            account.clone(),
            *d,
        ));
    }
}

impl Normalizer for SolanaNormalizer {
    type RawBlock = SolanaBlock;
    type RawTx = SolanaTx;

    fn normalize_block(&self, _chain: &ChainInfo, raw: SolanaBlock) -> Result<NormalizedBlock> {
        let producer = raw
            .rewards
            .iter()
            .flatten()
            .find(|r| r.reward_type.as_deref() == Some("Fee"))
            .map(|r| r.pubkey.clone());
        let transactions = raw
            .transactions
            .into_iter()
            .enumerate()
            .map(|(i, tx)| {
                self.tx(
                    tx,
                    Some(raw.slot),
                    Some(raw.blockhash.clone()),
                    Some(i as u64),
                )
            })
            .collect::<Result<_>>()?;
        Ok(NormalizedBlock {
            height: raw.slot,
            hash: raw.blockhash,
            parent_hash: raw.previous_blockhash,
            timestamp: raw.block_time.unwrap_or(0).max(0) as u64,
            producer,
            transactions,
            extra: json!({ "blockHeight": raw.block_height, "parentSlot": raw.parent_slot }),
        })
    }

    fn normalize_tx(&self, _chain: &ChainInfo, raw: SolanaTx) -> Result<NormalizedTx> {
        self.tx(raw, None, None, None)
    }
}

/// The ready-made Solana pipeline type.
pub type SolanaPipeline<T> = Pipeline<SolanaAdapter<T>, SolanaNormalizer, EvmTransformer>;

/// Solana signatures hash one-way, so the pipeline remembers the last `100_000` it saw,
/// letting `transaction(hash)` work for anything that appeared in a fetched block.
pub fn pipeline<T: Transport>(adapter: SolanaAdapter<T>) -> SolanaPipeline<T> {
    Pipeline::new(
        adapter,
        SolanaNormalizer,
        EvmTransformer::new(IndexedMapper::new(DefaultMapper::new(false), 100_000)),
    )
}

/// The ready-made pipeline over HTTP, from a URL or a key: `"solana"` (mainnet) or
/// `"solana-devnet"`. Keys honour `EVMIFY_RPC_SOLANA` / `EVMIFY_RPC_SOLANA_DEVNET`.
///
/// ```no_run
/// # async fn demo() -> evmify::Result<()> {
/// use evmify::{BlockId, ChainProvider, adapters::solana};
/// let sol = solana::connect("solana")?;
/// let block = sol.block(BlockId::Latest, false).await?;
/// # Ok(()) }
/// ```
#[cfg(feature = "http")]
pub fn connect(url_or_key: &str) -> Result<SolanaPipeline<crate::transport::HttpTransport>> {
    let rpc = crate::chains::resolve_rpc(url_or_key)?;
    let chain = match rpc.chain {
        Some(c) if c.kind == crate::chains::ChainKind::Solana => c.info(),
        Some(c) => {
            return Err(Error::Unsupported(format!(
                "`{}` is not a Solana network",
                c.key
            )));
        }
        None => ChainInfo::solana_mainnet(),
    };
    Ok(pipeline(SolanaAdapter::new(
        crate::transport::HttpTransport::new(rpc.url),
        chain,
    )))
}
