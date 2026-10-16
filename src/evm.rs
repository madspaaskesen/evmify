//! Typed client for any EVM-compatible JSON-RPC endpoint.

use alloy_primitives::{Address, B256, Bytes, U64, U256};
use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::chains::{self, ChainInfo};
use crate::error::{Error, Result};
use crate::provider::ChainProvider;
use crate::scan::LogScan;
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
    /// HTTP client for a URL or a shorthand key from [`chains::KNOWN_CHAINS`]:
    ///
    /// ```no_run
    /// # async fn demo() -> evmify::Result<()> {
    /// use evmify::EvmClient;
    /// let ronin = EvmClient::connect("ronin").await?;                     // public RPC
    /// let mine = EvmClient::connect("https://my-node.example:8545").await?; // any URL
    /// # Ok(()) }
    /// ```
    ///
    /// A key uses `EVMIFY_RPC_<KEY>` when set (e.g. `EVMIFY_RPC_RONIN`), otherwise the
    /// chain's public RPC; see [`chains::resolve_rpc`]. The node is asked for its chain id,
    /// and for a key it must match, so a misconfigured URL fails early.
    pub async fn connect(url_or_key: impl AsRef<str>) -> Result<Self> {
        let rpc = chains::resolve_rpc(url_or_key.as_ref())?;
        let transport = crate::transport::HttpTransport::new(rpc.url);
        match rpc.chain.and_then(|c| c.evm_chain_id) {
            Some(expected) => Self::detect_expecting(transport, expected).await,
            None if rpc.chain.is_some() => Err(Error::Unsupported(format!(
                "`{}` is not an EVM chain; use its adapter (e.g. adapters::solana::connect)",
                url_or_key.as_ref()
            ))),
            None => Self::detect(transport).await,
        }
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

    /// Like [`EvmClient::detect`], but fails with [`Error::ChainMismatch`] when the node
    /// serves a different chain than `expected_chain_id`.
    pub async fn detect_expecting(transport: T, expected_chain_id: u64) -> Result<Self> {
        let client = Self::detect(transport).await?;
        match client.chain.evm_chain_id {
            Some(id) if id == expected_chain_id => Ok(client),
            actual => Err(Error::ChainMismatch {
                expected: expected_chain_id,
                actual: actual.unwrap_or_default(),
            }),
        }
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

    /// `eth_getLogs` over blocks `from..=to`, split into windows of at most `max_range`
    /// blocks, since most providers cap the range per call (Ronin: 200, many others:
    /// 500-10 000). If the node still rejects a window as too large, the window is
    /// halved and retried, so this works without knowing the provider's limit.
    ///
    /// The filter's own `fromBlock` / `toBlock` / `blockHash` are ignored. For newest-first
    /// scans, limits, rate limiting and progress, use [`EvmClient::scan_logs`].
    pub async fn get_logs_chunked(
        &self,
        filter: &Filter,
        from: u64,
        to: u64,
        max_range: u64,
    ) -> Result<Vec<Log>> {
        self.scan_logs(filter.clone())
            .from_block(from)
            .to_block(to)
            .chunk(max_range)
            .run()
            .await
    }

    /// Scan `eth_getLogs` over a long range: in windows, newest or oldest first, with an
    /// optional result limit, rate limit and progress callback. See [`LogScan`].
    pub fn scan_logs(&self, filter: Filter) -> LogScan<'_, T> {
        LogScan::new(self, filter)
    }

    /// The first block whose timestamp is at or after `unix_secs` (or the head, if that
    /// moment hasn't been reached). Exact, unlike [`blocks_in`](Self::blocks_in).
    ///
    /// It first guesses the block from the chain's measured block time, then widens a
    /// bracket around the guess and binary-searches inside it. That keeps every lookup
    /// close to the target, which matters because most public RPCs are not archive
    /// nodes: they only serve recent blocks. Typically 15-30 `eth_getBlockByNumber` calls.
    ///
    /// The last block *at or before* `t` is `block_at(t + 1) - 1`.
    pub async fn block_at(&self, unix_secs: u64) -> Result<u64> {
        let head = self
            .get_block(BlockId::Latest, false)
            .await?
            .ok_or_else(|| Error::Unsupported("the node did not return the latest block".into()))?;
        let (head_n, head_ts): (u64, u64) = (head.number.to(), head.timestamp.to());
        if unix_secs >= head_ts || head_n == 0 {
            return Ok(head_n);
        }

        // Guess from the average block time of the last (up to) 1 000 blocks.
        let sample = head_n.min(1_000);
        let sample_ts = self.timestamp_of(head_n - sample).await?;
        let secs_per_block = (head_ts.saturating_sub(sample_ts) as f64 / sample as f64).max(0.001);
        let back = ((head_ts - unix_secs) as f64 / secs_per_block).ceil() as u64;
        let guess = head_n.saturating_sub(back);

        // Bracket: timestamp(lo) < t <= timestamp(hi), widening from the guess.
        let (mut lo, mut hi);
        let mut step = (back / 20).max(100);
        if self.timestamp_of(guess).await? >= unix_secs {
            hi = guess;
            loop {
                if hi == 0 {
                    return Ok(0);
                }
                let candidate = hi.saturating_sub(step);
                if self.timestamp_of(candidate).await? < unix_secs {
                    lo = candidate;
                    break;
                }
                hi = candidate;
                step = step.saturating_mul(2);
            }
        } else {
            lo = guess;
            loop {
                let candidate = lo.saturating_add(step).min(head_n);
                if candidate == head_n || self.timestamp_of(candidate).await? >= unix_secs {
                    hi = candidate;
                    break;
                }
                lo = candidate;
                step = step.saturating_mul(2);
            }
        }

        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if self.timestamp_of(mid).await? >= unix_secs {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        Ok(hi)
    }

    async fn timestamp_of(&self, number: u64) -> Result<u64> {
        match self.get_block(BlockId::Number(number), false).await? {
            Some(block) => Ok(block.timestamp.to()),
            None => Err(Error::Unsupported(format!(
                "this RPC does not serve block {number}: it is probably not an archive node \
                 and only keeps recent history. Pick a more recent date, or use an archive RPC \
                 (e.g. via EVMIFY_RPC_<KEY>)"
            ))),
        }
    }

    /// How many blocks cover `duration`, from the chain's average block time over its
    /// last 1 000 blocks (two `eth_getBlockByNumber` calls). Handy for "the last 24 h".
    pub async fn blocks_in(&self, duration: std::time::Duration) -> Result<u64> {
        let missing = || Error::Unsupported("the node did not return the block".into());
        let head = self
            .get_block(BlockId::Latest, false)
            .await?
            .ok_or_else(missing)?;
        let head_number: u64 = head.number.to();
        let sample = head_number.min(1_000);
        if sample == 0 {
            return Ok(0);
        }
        let old = self
            .get_block(BlockId::Number(head_number - sample), false)
            .await?
            .ok_or_else(missing)?;
        let elapsed = head
            .timestamp
            .to::<u64>()
            .saturating_sub(old.timestamp.to::<u64>());
        if elapsed == 0 {
            return Err(Error::Unsupported(
                "cannot measure block time: timestamps are equal".into(),
            ));
        }
        Ok((duration.as_secs_f64() * sample as f64 / elapsed as f64).ceil() as u64)
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
