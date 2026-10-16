//! # evmify
//!
//! [Project page](https://mp-it.dk/projects/evmify) ·
//! [GitHub](https://github.com/madspaaskesen/evmify) ·
//! [Examples](https://github.com/madspaaskesen/evmify/blob/main/EXAMPLES.md) ·
//! [Roadmap](https://github.com/madspaaskesen/evmify/blob/main/FUTURE.md)
//!
//! **EVM-ify any chain.** One read API for EVM and non-EVM chains, always answered in EVM JSON-RPC shape.
//!
//! * [`EvmClient`] — typed `eth_*` client that works on any EVM chain (Ethereum, L2s,
//!   sidechains). Unknown response fields are preserved, never dropped.
//! * [`pipeline`] — a modular **adapter → normalizer → transformer** pipeline that serves
//!   non-EVM chains (Solana and Bitcoin included) as EVM blocks, transactions, receipts
//!   and logs.
//! * [`ChainProvider`] — the trait both implement, so you can hold them side by side.
//!
//! ```no_run
//! # #[cfg(feature = "http")]
//! # async fn demo() -> evmify::Result<()> {
//! use evmify::{BlockId, ChainProvider, EvmClient, adapters::solana};
//!
//! // Shorthand keys use the chain's public RPC (or EVMIFY_RPC_<KEY>); URLs work too.
//! let chains: Vec<Box<dyn ChainProvider>> = vec![
//!     Box::new(EvmClient::connect("ethereum").await?),
//!     Box::new(EvmClient::connect("https://mainnet.base.org").await?),
//!     Box::new(solana::connect("solana")?),
//! ];
//! for chain in &chains {
//!     let block = chain.block(BlockId::Latest, false).await?;
//!     println!("{}: {:?}", chain.chain().name, block.map(|b| b.number));
//! }
//! # Ok(()) }
//! ```
//!
//! Built and maintained by Mads Paaskesen, Hardcore Websmith at [MP-IT](https://mp-it.dk/projects/evmify).

pub mod adapters;
pub mod chains;
mod error;
pub mod evm;
pub mod pipeline;
mod provider;
pub mod scan;
pub mod transport;
pub mod types;

pub use alloy_primitives::{self as primitives, Address, B256, Bytes, U64, U256};
pub use chains::{ChainInfo, ChainKind, KnownChain, resolve_rpc};
pub use error::{Error, Result};
pub use evm::EvmClient;
pub use pipeline::{Adapter, EvmTransformer, Normalizer, Pipeline, Transformer};
pub use provider::ChainProvider;
pub use scan::{LogScan, ScanPlan, ScanProgress};
#[cfg(feature = "http")]
pub use transport::HttpTransport;
pub use transport::{FallbackTransport, MockTransport, Transport};
pub use types::*;
