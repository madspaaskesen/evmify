//! One read interface for every chain.

use alloy_primitives::B256;
use async_trait::async_trait;

use crate::chains::ChainInfo;
use crate::error::{Error, Result};
use crate::types::{Block, BlockId, Filter, Log, Receipt, Transaction};

/// Read access to a chain, always answered in EVM shape.
///
/// Implemented by [`crate::EvmClient`] (pass-through) and by [`crate::Pipeline`]
/// (non-EVM chains translated on the fly), so `Vec<Box<dyn ChainProvider>>` can hold
/// Ethereum, Base, Solana and Bitcoin side by side.
#[async_trait]
pub trait ChainProvider: Send + Sync {
    fn chain(&self) -> &ChainInfo;

    async fn block_number(&self) -> Result<u64>;

    async fn block(&self, at: BlockId, full: bool) -> Result<Option<Block>>;

    async fn transaction(&self, hash: B256) -> Result<Option<Transaction>>;

    async fn receipt(&self, hash: B256) -> Result<Option<Receipt>>;

    /// Look a transaction up by its chain-native id (Solana signature, Bitcoin txid,
    /// or an EVM `0x` hash). The default only understands 32-byte hex ids.
    async fn transaction_by_native_id(&self, id: &str) -> Result<Option<Transaction>> {
        let hash: B256 = id
            .parse()
            .map_err(|_| Error::Unsupported(format!("`{id}` is not a 32-byte hex hash")))?;
        self.transaction(hash).await
    }

    /// Receipt counterpart of [`ChainProvider::transaction_by_native_id`].
    async fn receipt_by_native_id(&self, id: &str) -> Result<Option<Receipt>> {
        let hash: B256 = id
            .parse()
            .map_err(|_| Error::Unsupported(format!("`{id}` is not a 32-byte hex hash")))?;
        self.receipt(hash).await
    }

    async fn logs(&self, filter: &Filter) -> Result<Vec<Log>>;
}
