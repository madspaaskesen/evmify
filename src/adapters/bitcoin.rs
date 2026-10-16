//! Bitcoin Core JSON-RPC (`getblockcount` / `getblockhash` / `getblock` / `getrawtransaction`).
//!
//! Needs Bitcoin Core 25+ (or a compatible provider) for `getblock` verbosity 3 and
//! `getrawtransaction` verbosity 2, which include each input's `prevout`. Transaction lookup
//! by id needs `-txindex=1` on the node.
//!
//! Mapping:
//! * txids and block hashes are 32 bytes already, so EVM hashes equal them and lookups by
//!   hash work in both directions
//! * `from` = first input's address (`None` for coinbase), `to` = first output not paying
//!   back to an input address, `value` = total paid to such outputs (i.e. excluding change)
//! * every output becomes an ERC-20 style `Transfer(from, outputAddress, sats)` log
//!   emitted by the native-token address; coinbase outputs come "from" `0x0`
//! * block `miner` = first coinbase output address

use std::collections::HashSet;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::chains::ChainInfo;
use crate::error::{Error, Result};
use crate::pipeline::{
    Adapter, DefaultMapper, EvmTransformer, NULL_ACCOUNT, NormalizedBlock, NormalizedEvent,
    NormalizedTx, Normalizer, Pipeline,
};
use crate::transport::Transport;

/// Block height out of range.
const RPC_INVALID_PARAMETER: i64 = -8;
/// No such transaction / block.
const RPC_INVALID_ADDRESS_OR_KEY: i64 = -5;

#[derive(Debug, Clone, Deserialize)]
pub struct BitcoinBlock {
    pub hash: String,
    pub height: u64,
    #[serde(default)]
    pub previousblockhash: Option<String>,
    pub time: u64,
    #[serde(default)]
    pub tx: Vec<BitcoinTx>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BitcoinTx {
    pub txid: String,
    #[serde(default)]
    pub vin: Vec<BitcoinVin>,
    #[serde(default)]
    pub vout: Vec<BitcoinVout>,
    /// BTC, present when prevouts are known (not for coinbase).
    #[serde(default)]
    pub fee: Option<f64>,
    #[serde(default)]
    pub blockhash: Option<String>,
    #[serde(default)]
    pub blocktime: Option<u64>,
    /// Not in the RPC payload; filled in by the adapter from `getblockheader`.
    #[serde(default)]
    pub block_height: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BitcoinVin {
    #[serde(default)]
    pub coinbase: Option<String>,
    #[serde(default)]
    pub txid: Option<String>,
    #[serde(default)]
    pub vout: Option<u32>,
    #[serde(default)]
    pub prevout: Option<BitcoinPrevout>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BitcoinPrevout {
    pub value: f64,
    #[serde(rename = "scriptPubKey")]
    pub script_pub_key: BitcoinScript,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BitcoinVout {
    pub value: f64,
    pub n: u32,
    #[serde(rename = "scriptPubKey")]
    pub script_pub_key: BitcoinScript,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BitcoinScript {
    #[serde(default)]
    pub address: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
}

/// BTC float from RPC -> satoshis. Exact for every real amount (< 2^53 sats).
pub fn btc_to_sats(btc: f64) -> u64 {
    (btc * 100_000_000.0).round() as u64
}

#[derive(Debug, Clone)]
pub struct BitcoinAdapter<T> {
    transport: T,
    chain: ChainInfo,
    /// Blocks this deep are reported as finalized.
    pub finality_depth: u64,
}

impl<T: Transport> BitcoinAdapter<T> {
    pub fn new(transport: T, chain: ChainInfo) -> Self {
        Self {
            transport,
            chain,
            finality_depth: 6,
        }
    }

    pub fn mainnet(transport: T) -> Self {
        Self::new(transport, ChainInfo::bitcoin_mainnet())
    }

    async fn call<R: serde::de::DeserializeOwned>(&self, method: &str, params: Value) -> Result<R> {
        let v = self.transport.request(method, params).await?;
        serde_json::from_value(v).map_err(|e| Error::decode(method, e))
    }
}

fn not_found<R>(r: Result<R>) -> Result<Option<R>> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::Rpc { code, .. })
            if code == RPC_INVALID_PARAMETER || code == RPC_INVALID_ADDRESS_OR_KEY =>
        {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

#[async_trait]
impl<T: Transport> Adapter for BitcoinAdapter<T> {
    type RawBlock = BitcoinBlock;
    type RawTx = BitcoinTx;

    fn chain(&self) -> &ChainInfo {
        &self.chain
    }

    async fn head(&self) -> Result<u64> {
        self.call("getblockcount", json!([])).await
    }

    async fn finalized_head(&self) -> Result<u64> {
        Ok(self.head().await?.saturating_sub(self.finality_depth))
    }

    async fn block(&self, height: u64) -> Result<Option<BitcoinBlock>> {
        let Some(hash) = not_found(self.call::<String>("getblockhash", json!([height])).await)?
        else {
            return Ok(None);
        };
        not_found(self.call("getblock", json!([hash, 3])).await)
    }

    async fn transaction(&self, txid: &str) -> Result<Option<BitcoinTx>> {
        let Some(mut tx) = not_found(
            self.call::<BitcoinTx>("getrawtransaction", json!([txid, 2]))
                .await,
        )?
        else {
            return Ok(None);
        };
        if let Some(hash) = &tx.blockhash {
            let header: Value = self.call("getblockheader", json!([hash])).await?;
            tx.block_height = header.get("height").and_then(Value::as_u64);
        }
        Ok(Some(tx))
    }
}

#[derive(Debug, Clone, Default)]
pub struct BitcoinNormalizer;

impl BitcoinNormalizer {
    fn tx(
        &self,
        raw: BitcoinTx,
        height: Option<u64>,
        block_hash: Option<String>,
        index: Option<u64>,
    ) -> NormalizedTx {
        let coinbase = raw.vin.iter().any(|v| v.coinbase.is_some());
        let input_addrs: Vec<String> = raw
            .vin
            .iter()
            .filter_map(|v| v.prevout.as_ref()?.script_pub_key.address.clone())
            .collect();
        let own: HashSet<&str> = input_addrs.iter().map(String::as_str).collect();
        let from = input_addrs.first().cloned();

        let sum_out: u64 = raw.vout.iter().map(|o| btc_to_sats(o.value)).sum();
        let all_prevouts = raw.vin.iter().all(|v| v.prevout.is_some());
        let fee = match raw.fee {
            Some(f) => Some(btc_to_sats(f) as u128),
            None if coinbase => Some(0),
            None if all_prevouts => {
                let sum_in: u64 = raw
                    .vin
                    .iter()
                    .filter_map(|v| v.prevout.as_ref())
                    .map(|p| btc_to_sats(p.value))
                    .sum();
                Some(sum_in.saturating_sub(sum_out) as u128)
            }
            None => None,
        };

        let payments: Vec<(&str, u64)> = raw
            .vout
            .iter()
            .filter_map(|o| Some((o.script_pub_key.address.as_deref()?, btc_to_sats(o.value))))
            .collect();
        let external: Vec<&(&str, u64)> =
            payments.iter().filter(|(a, _)| !own.contains(a)).collect();
        let to = external
            .first()
            .or(payments.first().as_ref())
            .map(|(a, _)| a.to_string());
        let value: u128 = external.iter().map(|(_, v)| *v as u128).sum();

        let sender = from.clone().unwrap_or_else(|| NULL_ACCOUNT.to_string());
        let events = payments
            .iter()
            .map(|(addr, sats)| {
                NormalizedEvent::transfer(None, sender.clone(), addr.to_string(), *sats as u128)
            })
            .collect();

        let outputs: Vec<Value> = raw
            .vout
            .iter()
            .map(|o| {
                json!({
                    "n": o.n,
                    "address": o.script_pub_key.address,
                    "type": o.script_pub_key.kind,
                    "sats": btc_to_sats(o.value),
                })
            })
            .collect();
        let inputs: Vec<Value> = raw
            .vin
            .iter()
            .map(|v| {
                json!({
                    "txid": v.txid,
                    "vout": v.vout,
                    "address": v.prevout.as_ref().and_then(|p| p.script_pub_key.address.clone()),
                    "sats": v.prevout.as_ref().map(|p| btc_to_sats(p.value)),
                })
            })
            .collect();

        // A transaction in a block is valid by definition; mempool txs are not final.
        let confirmed = raw.blockhash.is_some() || height.is_some() || raw.block_height.is_some();
        NormalizedTx {
            id: raw.txid,
            block_height: height.or(raw.block_height),
            block_hash: block_hash.or(raw.blockhash),
            index,
            from,
            to,
            value,
            fee,
            success: confirmed.then_some(true),
            input: Vec::new(),
            events,
            extra: json!({ "coinbase": coinbase, "inputs": inputs, "outputs": outputs }),
        }
    }
}

impl Normalizer for BitcoinNormalizer {
    type RawBlock = BitcoinBlock;
    type RawTx = BitcoinTx;

    fn normalize_block(&self, _chain: &ChainInfo, raw: BitcoinBlock) -> Result<NormalizedBlock> {
        let producer = raw
            .tx
            .first()
            .filter(|t| t.vin.iter().any(|v| v.coinbase.is_some()))
            .and_then(|t| t.vout.iter().find_map(|o| o.script_pub_key.address.clone()));
        let tx_count = raw.tx.len();
        let transactions = raw
            .tx
            .into_iter()
            .enumerate()
            .map(|(i, tx)| self.tx(tx, Some(raw.height), Some(raw.hash.clone()), Some(i as u64)))
            .collect();
        Ok(NormalizedBlock {
            height: raw.height,
            hash: raw.hash,
            parent_hash: raw.previousblockhash.unwrap_or_default(),
            timestamp: raw.time,
            producer,
            transactions,
            extra: json!({ "txCount": tx_count }),
        })
    }

    fn normalize_tx(&self, _chain: &ChainInfo, raw: BitcoinTx) -> Result<NormalizedTx> {
        Ok(self.tx(raw, None, None, None))
    }
}

/// The ready-made Bitcoin pipeline type.
pub type BitcoinPipeline<T> = Pipeline<BitcoinAdapter<T>, BitcoinNormalizer, EvmTransformer>;

/// Bitcoin ids are hex already, so lookups by EVM hash map straight back to txids.
pub fn pipeline<T: Transport>(adapter: BitcoinAdapter<T>) -> BitcoinPipeline<T> {
    Pipeline::new(
        adapter,
        BitcoinNormalizer,
        EvmTransformer::new(DefaultMapper::new(true)),
    )
}

/// The ready-made pipeline over HTTP, from a URL or a key (`"bitcoin"`,
/// `"bitcoin-testnet"`). There is no public Bitcoin JSON-RPC, so a key needs
/// `EVMIFY_RPC_BITCOIN` (or `_TESTNET`) pointing at your node. Credentials may be
/// embedded: `http://user:password@127.0.0.1:8332`.
#[cfg(feature = "http")]
pub fn connect(url_or_key: &str) -> Result<BitcoinPipeline<crate::transport::HttpTransport>> {
    let rpc = crate::chains::resolve_rpc(url_or_key)?;
    let chain = match rpc.chain {
        Some(c) if c.kind == crate::chains::ChainKind::Bitcoin => c.info(),
        Some(c) => {
            return Err(Error::Unsupported(format!(
                "`{}` is not a Bitcoin network",
                c.key
            )));
        }
        None => ChainInfo::bitcoin_mainnet(),
    };
    let (url, credentials) = split_credentials(&rpc.url);
    let mut transport = crate::transport::HttpTransport::new(url);
    if let Some((user, password)) = credentials {
        transport = transport.basic_auth(user, password);
    }
    Ok(pipeline(BitcoinAdapter::new(transport, chain)))
}

/// `http://user:pw@host/` -> (`http://host/`, Some(("user", Some("pw")))).
#[cfg(feature = "http")]
fn split_credentials(url: &str) -> (String, Option<(String, Option<String>)>) {
    let Some((scheme, rest)) = url.split_once("://") else {
        return (url.to_string(), None);
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    match rest[..authority_end].rsplit_once('@') {
        Some((userinfo, host)) => {
            let (user, password) = match userinfo.split_once(':') {
                Some((u, p)) => (u.to_string(), Some(p.to_string())),
                None => (userinfo.to_string(), None),
            };
            (
                format!("{scheme}://{host}{}", &rest[authority_end..]),
                Some((user, password)),
            )
        }
        None => (url.to_string(), None),
    }
}

#[cfg(all(test, feature = "http"))]
mod tests {
    use super::split_credentials;

    #[test]
    fn credentials_are_split_from_the_url() {
        assert_eq!(
            split_credentials("http://alice:s3cret@127.0.0.1:8332/wallet/x"),
            (
                "http://127.0.0.1:8332/wallet/x".to_string(),
                Some(("alice".to_string(), Some("s3cret".to_string())))
            )
        );
        assert_eq!(
            split_credentials("https://node.example"),
            ("https://node.example".to_string(), None)
        );
    }
}
