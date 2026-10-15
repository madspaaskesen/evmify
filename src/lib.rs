//! # evmify
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
//! use evmify::{BlockId, ChainProvider, EvmClient, HttpTransport, adapters::solana};
//!
//! let chains: Vec<Box<dyn ChainProvider>> = vec![
//!     Box::new(EvmClient::connect("https://eth.llamarpc.com").await?),
//!     Box::new(solana::pipeline(solana::SolanaAdapter::mainnet(
//!         HttpTransport::new("https://api.mainnet-beta.solana.com"),
//!     ))),
//! ];
//! for chain in &chains {
//!     let block = chain.block(BlockId::Latest, false).await?;
//!     println!("{}: {:?}", chain.chain().name, block.map(|b| b.number));
//! }
//! # Ok(()) }
//! ```

pub mod adapters;
pub mod chains;
mod error;
pub mod evm;
pub mod pipeline;
mod provider;
pub mod transport;
pub mod types;

pub use alloy_primitives::{self as primitives, Address, B256, Bytes, U64, U256};
pub use chains::{ChainInfo, ChainKind};
pub use error::{Error, Result};
pub use evm::EvmClient;
pub use pipeline::{Adapter, EvmTransformer, Normalizer, Pipeline, Transformer};
pub use provider::ChainProvider;
#[cfg(feature = "http")]
pub use transport::HttpTransport;
pub use transport::{FallbackTransport, MockTransport, Transport};
pub use types::*;
