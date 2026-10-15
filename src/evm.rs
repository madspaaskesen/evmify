//! Typed client for any EVM-compatible JSON-RPC endpoint.

use alloy_primitives::{Address, B256, Bytes, U64, U256};
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::chains::{self, ChainInfo};
use crate::error::{Error, Result};
use crate::provider::ChainProvider;
use crate::transport::Transport;
use crate::types::{Block, BlockId, CallRequest, Filter, Log, Receipt, Transaction};

/// Works against Ethereum, L2s and EVM sidechains alike: it only uses the standard
/// `eth_*` namespace, and unknown response fields are kept in each type's `other` map.
#[derive(Debug, Clone)]
pub struct EvmClient<T> {
    transport: T,
    chain: ChainInfo,
}

#[cfg(feature = "http")]
impl EvmClient<crate::transport::HttpTransport> {
    /// HTTP client for `url`, asking the node which chain it serves.
    pub async fn connect(url: impl Into<String>) -> Result<Self> {
        Self::detect(crate::transport::HttpTransport::new(url)).await
    }
}

impl<T: Transport> EvmClient<T> {
    /// Use a transport for a chain you already know.
    pub fn new(transport: T, chain: ChainInfo) -> Self {
        Self { transport, chain }
    }

    /// Call `eth_chainId` and fill in chain metadata from the built-in registry.
    pub async fn detect(transport: T) -> Result<Self> {
        let id: U64 = decode(
            "eth_chainId",
            transport.request("eth_chainId", json!([])).await?,
        )?;
        let id = id.to::<u64>();
        let chain = chains::by_id(id).unwrap_or_else(|| ChainInfo::evm_unknown(id));
        Ok(Self { transport, chain })
    }

    pub fn transport(&self) -> &T {
        &self.transport
    }

    pub fn chain_info(&self) -> &ChainInfo {
        &self.chain
    }

    /// Escape hatch: any method, typed result.
    pub async fn request<R: DeserializeOwned>(&self, method: &str, params: Value) -> Result<R> {
        decode(method, self.transport.request(method, params).await?)
    }

    pub async fn chain_id(&self) -> Result<u64> {
        Ok(self.request::<U64>("eth_chainId", json!([])).await?.to())
    }

    pub async fn block_number(&self) -> Result<u64> {
        Ok(self
            .request::<U64>("eth_blockNumber", json!([]))
            .await?
            .to())
    }

    pub async fn gas_price(&self) -> Result<U256> {
        self.request("eth_gasPrice", json!([])).await
    }

    pub async fn get_balance(&self, address: Address, at: BlockId) -> Result<U256> {
        self.request("eth_getBalance", json!([address, at.to_param()]))
            .await
    }

    pub async fn get_transaction_count(&self, address: Address, at: BlockId) -> Result<u64> {
        Ok(self
            .request::<U64>("eth_getTransactionCount", json!([address, at.to_param()]))
            .await?
            .to())
    }

    pub async fn get_code(&self, address: Address, at: BlockId) -> Result<Bytes> {
        self.request("eth_getCode", json!([address, at.to_param()]))
            .await
    }

    pub async fn get_storage_at(&self, address: Address, slot: U256, at: BlockId) -> Result<B256> {
        self.request("eth_getStorageAt", json!([address, slot, at.to_param()]))
            .await
    }

    /// `full = true` returns transaction objects, otherwise only hashes.
    pub async fn get_block(&self, at: BlockId, full: bool) -> Result<Option<Block>> {
        self.request("eth_getBlockByNumber", json!([at.to_param(), full]))
            .await
    }

    pub async fn get_block_by_hash(&self, hash: B256, full: bool) -> Result<Option<Block>> {
        self.request("eth_getBlockByHash", json!([hash, full]))
            .await
    }

    pub async fn get_transaction(&self, hash: B256) -> Result<Option<Transaction>> {
        self.request("eth_getTransactionByHash", json!([hash]))
            .await
    }

    pub async fn get_transaction_receipt(&self, hash: B256) -> Result<Option<Receipt>> {
        self.request("eth_getTransactionReceipt", json!([hash]))
            .await
    }

    pub async fn get_logs(&self, filter: &Filter) -> Result<Vec<Log>> {
        self.request("eth_getLogs", json!([filter])).await
    }

    pub async fn call(&self, call: &CallRequest, at: BlockId) -> Result<Bytes> {
        self.request("eth_call", json!([call, at.to_param()])).await
    }

    pub async fn estimate_gas(&self, call: &CallRequest) -> Result<U256> {
        self.request("eth_estimateGas", json!([call])).await
    }

    /// Broadcast an already-signed transaction. Signing is out of scope for this crate.
    pub async fn send_raw_transaction(&self, raw: Bytes) -> Result<B256> {
        self.request("eth_sendRawTransaction", json!([raw])).await
    }
}

fn decode<R: DeserializeOwned>(method: &str, v: Value) -> Result<R> {
    serde_json::from_value(v).map_err(|e| Error::decode(method, e))
}

#[async_trait]
impl<T: Transport> ChainProvider for EvmClient<T> {
    fn chain(&self) -> &ChainInfo {
        &self.chain
    }

    async fn block_number(&self) -> Result<u64> {
        EvmClient::block_number(self).await
    }

    async fn block(&self, at: BlockId, full: bool) -> Result<Option<Block>> {
        self.get_block(at, full).await
    }

    async fn transaction(&self, hash: B256) -> Result<Option<Transaction>> {
        self.get_transaction(hash).await
    }

    async fn receipt(&self, hash: B256) -> Result<Option<Receipt>> {
        self.get_transaction_receipt(hash).await
    }

    async fn logs(&self, filter: &Filter) -> Result<Vec<Log>> {
        self.get_logs(filter).await
    }
}
