//! Serve a non-EVM chain in EVM shape:
//!
//! ```text
//!  native RPC ──► Adapter ──► RawBlock / RawTx ──► Normalizer ──► Normalized* ──► Transformer ──► EVM types
//!                (fetch)                           (understand)                  (present)
//! ```
//!
//! Each stage is a trait, so you can swap one without touching the others: a different
//! Solana RPC provider is a new [`Adapter`]; decoding SPL instructions differently is a new
//! [`Normalizer`]; emitting a different output format is a new [`Transformer`].

mod mapper;
mod normalized;
mod transform;

use alloy_primitives::B256;
use async_trait::async_trait;
use serde_json::Value;

pub use mapper::{DefaultMapper, IdMapper, IndexedMapper};
pub use normalized::{EventValue, NULL_ACCOUNT, NormalizedBlock, NormalizedEvent, NormalizedTx};
pub use transform::{EvmTransformer, NATIVE_TOKEN_ADDRESS, ValueScale};

use crate::chains::ChainInfo;
use crate::error::{Error, Result};
use crate::provider::ChainProvider;
use crate::types::{Block, BlockId, Filter, Log, Receipt, Transaction};

/// Stage 1 — talks to the native node and returns its payloads untouched.
#[async_trait]
pub trait Adapter: Send + Sync {
    type RawBlock: Send + 'static;
    type RawTx: Send + 'static;

    fn chain(&self) -> &ChainInfo;

    /// Latest block height (Solana: slot) the node will serve.
    async fn head(&self) -> Result<u64>;

    /// Latest height considered final. Defaults to [`Adapter::head`].
    async fn finalized_head(&self) -> Result<u64> {
        self.head().await
    }

    /// `Ok(None)` when the height does not exist (yet), or was skipped.
    async fn block(&self, height: u64) -> Result<Option<Self::RawBlock>>;

    async fn transaction(&self, native_id: &str) -> Result<Option<Self::RawTx>>;
}

/// Stage 2 — understands a chain's raw payloads and produces the chain-agnostic model.
/// Pure and synchronous, which makes it trivial to unit-test against JSON fixtures.
pub trait Normalizer: Send + Sync {
    type RawBlock;
    type RawTx;

    fn normalize_block(&self, chain: &ChainInfo, raw: Self::RawBlock) -> Result<NormalizedBlock>;

    fn normalize_tx(&self, chain: &ChainInfo, raw: Self::RawTx) -> Result<NormalizedTx>;
}

/// Stage 3 — presents the normalized model in some output format.
pub trait Transformer: Send + Sync {
    type Block;
    type Tx;
    type Receipt;
    type Log;

    fn block(&self, chain: &ChainInfo, block: &NormalizedBlock, full: bool) -> Result<Self::Block>;

    fn transaction(&self, chain: &ChainInfo, tx: &NormalizedTx) -> Result<Self::Tx>;

    fn receipt(&self, chain: &ChainInfo, tx: &NormalizedTx) -> Result<Self::Receipt>;

    /// Logs of one transaction, numbered from `first_index` (block-level log index).
    fn logs(
        &self,
        chain: &ChainInfo,
        tx: &NormalizedTx,
        first_index: u64,
    ) -> Result<Vec<Self::Log>>;

    /// Map an output hash back to a native id, if the transformer can.
    fn native_tx_id(&self, _hash: &B256) -> Option<String> {
        None
    }
}

/// The three stages wired together. With the default [`EvmTransformer`] it implements
/// [`ChainProvider`], so it can sit next to an [`crate::EvmClient`].
#[derive(Debug, Clone)]
pub struct Pipeline<A, N, T = EvmTransformer> {
    pub adapter: A,
    pub normalizer: N,
    pub transformer: T,
    /// Largest block range [`ChainProvider::logs`] will scan (each block is one RPC call).
    pub max_log_range: u64,
}

impl<A, N, T> Pipeline<A, N, T>
where
    A: Adapter,
    N: Normalizer<RawBlock = A::RawBlock, RawTx = A::RawTx>,
    T: Transformer,
{
    pub fn new(adapter: A, normalizer: N, transformer: T) -> Self {
        Self {
            adapter,
            normalizer,
            transformer,
            max_log_range: 50,
        }
    }

    pub fn with_max_log_range(mut self, blocks: u64) -> Self {
        self.max_log_range = blocks;
        self
    }

    /// Swap the output stage, keeping adapter and normalizer.
    pub fn with_transformer<T2: Transformer>(self, transformer: T2) -> Pipeline<A, N, T2> {
        Pipeline {
            adapter: self.adapter,
            normalizer: self.normalizer,
            transformer,
            max_log_range: self.max_log_range,
        }
    }

    pub async fn resolve(&self, at: BlockId) -> Result<u64> {
        match at {
            BlockId::Number(n) => Ok(n),
            BlockId::Earliest => Ok(0),
            BlockId::Latest | BlockId::Pending => self.adapter.head().await,
            BlockId::Safe | BlockId::Finalized => self.adapter.finalized_head().await,
        }
    }

    /// Fetch + normalize, without transforming. Useful for building your own output.
    pub async fn normalized_block(&self, at: BlockId) -> Result<Option<NormalizedBlock>> {
        let height = self.resolve(at).await?;
        match self.adapter.block(height).await? {
            Some(raw) => Ok(Some(
                self.normalizer.normalize_block(self.adapter.chain(), raw)?,
            )),
            None => Ok(None),
        }
    }

    pub async fn normalized_tx(&self, native_id: &str) -> Result<Option<NormalizedTx>> {
        match self.adapter.transaction(native_id).await? {
            Some(raw) => Ok(Some(
                self.normalizer.normalize_tx(self.adapter.chain(), raw)?,
            )),
            None => Ok(None),
        }
    }

    pub async fn transformed_block(&self, at: BlockId, full: bool) -> Result<Option<T::Block>> {
        match self.normalized_block(at).await? {
            Some(b) => Ok(Some(self.transformer.block(
                self.adapter.chain(),
                &b,
                full,
            )?)),
            None => Ok(None),
        }
    }

    pub async fn transformed_tx(&self, native_id: &str) -> Result<Option<T::Tx>> {
        match self.normalized_tx(native_id).await? {
            Some(t) => Ok(Some(
                self.transformer.transaction(self.adapter.chain(), &t)?,
            )),
            None => Ok(None),
        }
    }

    pub async fn transformed_receipt(&self, native_id: &str) -> Result<Option<T::Receipt>> {
        match self.normalized_tx(native_id).await? {
            Some(t) => Ok(Some(self.transformer.receipt(self.adapter.chain(), &t)?)),
            None => Ok(None),
        }
    }

    fn native_id_for(&self, hash: &B256) -> Result<String> {
        self.transformer.native_tx_id(hash).ok_or_else(|| {
            Error::Unsupported(format!(
                "{}: cannot map {hash} back to a native id; look it up with \
                 `transaction_by_native_id`, or fetch its block first (IndexedMapper)",
                self.adapter.chain().name
            ))
        })
    }
}

/// Parse a filter bound back from its JSON-RPC form.
fn parse_bound(v: &Option<Value>) -> Result<Option<BlockId>> {
    let Some(v) = v else { return Ok(None) };
    let s = v
        .as_str()
        .ok_or_else(|| Error::Unsupported(format!("block bound {v}")))?;
    Ok(Some(match s {
        "latest" => BlockId::Latest,
        "earliest" => BlockId::Earliest,
        "pending" => BlockId::Pending,
        "safe" => BlockId::Safe,
        "finalized" => BlockId::Finalized,
        hex => BlockId::Number(
            u64::from_str_radix(hex.trim_start_matches("0x"), 16)
                .map_err(|_| Error::Unsupported(format!("block bound {hex}")))?,
        ),
    }))
}

#[async_trait]
impl<A, N, T> ChainProvider for Pipeline<A, N, T>
where
    A: Adapter,
    N: Normalizer<RawBlock = A::RawBlock, RawTx = A::RawTx>,
    T: Transformer<Block = Block, Tx = Transaction, Receipt = Receipt, Log = Log>,
{
    fn chain(&self) -> &ChainInfo {
        self.adapter.chain()
    }

    async fn block_number(&self) -> Result<u64> {
        self.adapter.head().await
    }

    async fn block(&self, at: BlockId, full: bool) -> Result<Option<Block>> {
        self.transformed_block(at, full).await
    }

    async fn transaction(&self, hash: B256) -> Result<Option<Transaction>> {
        let id = self.native_id_for(&hash)?;
        self.transformed_tx(&id).await
    }

    async fn receipt(&self, hash: B256) -> Result<Option<Receipt>> {
        let id = self.native_id_for(&hash)?;
        self.transformed_receipt(&id).await
    }

    async fn transaction_by_native_id(&self, id: &str) -> Result<Option<Transaction>> {
        self.transformed_tx(id).await
    }

    async fn receipt_by_native_id(&self, id: &str) -> Result<Option<Receipt>> {
        self.transformed_receipt(id).await
    }

    /// Scans blocks one by one (non-EVM nodes have no log index), so the range is capped
    /// at [`Pipeline::max_log_range`].
    async fn logs(&self, filter: &Filter) -> Result<Vec<Log>> {
        if filter.block_hash.is_some() {
            return Err(Error::Unsupported(
                "blockHash filters on non-EVM chains; use a block range".into(),
            ));
        }
        let to = match parse_bound(&filter.to_block)? {
            Some(b) => self.resolve(b).await?,
            None => self.adapter.head().await?,
        };
        let from = match parse_bound(&filter.from_block)? {
            Some(b) => self.resolve(b).await?,
            None => to,
        };
        if from > to {
            return Ok(Vec::new());
        }
        if to - from + 1 > self.max_log_range {
            return Err(Error::Unsupported(format!(
                "log range of {} blocks exceeds max_log_range ({})",
                to - from + 1,
                self.max_log_range
            )));
        }
        let chain = self.adapter.chain();
        let mut out = Vec::new();
        for height in from..=to {
            let Some(raw) = self.adapter.block(height).await? else {
                continue;
            };
            let block = self.normalizer.normalize_block(chain, raw)?;
            let mut index = 0u64;
            for tx in &block.transactions {
                let logs = self.transformer.logs(chain, tx, index)?;
                index += logs.len() as u64;
                out.extend(logs.into_iter().filter(|l| filter.matches(l)));
            }
        }
        Ok(out)
    }
}
