//! Latest block from several chains, all through one trait and one output shape.
//!
//! ```sh
//! cargo run --example multi_chain
//! # optional: a Bitcoin Core node (getblock verbosity 3 needs Core 25+)
//! BTC_RPC_URL=http://127.0.0.1:8332 BTC_RPC_USER=user BTC_RPC_PASS=pass cargo run --example multi_chain
//! ```

use std::sync::Arc;

use evmify::adapters::{bitcoin, solana};
use evmify::{BlockId, ChainProvider, EvmClient, FallbackTransport, HttpTransport, Transport};

#[tokio::main]
async fn main() -> evmify::Result<()> {
    let mut chains: Vec<Box<dyn ChainProvider>> = Vec::new();

    // Any EVM chain: just a URL. The chain is identified via eth_chainId.
    for url in [
        "https://ethereum-rpc.publicnode.com",
        "https://mainnet.base.org",
    ] {
        match EvmClient::connect(url).await {
            Ok(c) => chains.push(Box::new(c)),
            Err(e) => eprintln!("skip {url}: {e}"),
        }
    }

    // Several endpoints for one chain, tried in order on network failure.
    let arbitrum = FallbackTransport::new(vec![
        Arc::new(HttpTransport::new("https://arb1.arbitrum.io/rpc")) as Arc<dyn Transport>,
        Arc::new(HttpTransport::new(
            "https://arbitrum-one-rpc.publicnode.com",
        )),
    ]);
    match EvmClient::detect(arbitrum).await {
        Ok(c) => chains.push(Box::new(c)),
        Err(e) => eprintln!("skip arbitrum: {e}"),
    }

    // Solana, served as EVM.
    chains.push(Box::new(solana::pipeline(solana::SolanaAdapter::mainnet(
        HttpTransport::new("https://api.mainnet-beta.solana.com"),
    ))));

    // Bitcoin, served as EVM (only if you have a node).
    if let Ok(url) = std::env::var("BTC_RPC_URL") {
        let mut t = HttpTransport::new(url);
        if let Ok(user) = std::env::var("BTC_RPC_USER") {
            t = t.basic_auth(user, std::env::var("BTC_RPC_PASS").ok());
        }
        chains.push(Box::new(bitcoin::pipeline(
            bitcoin::BitcoinAdapter::mainnet(t),
        )));
    }

    for chain in &chains {
        let info = chain.chain();
        match chain.block(BlockId::Latest, true).await {
            Ok(Some(block)) => {
                let n: u64 = block.number.to();
                println!(
                    "{:<12} block {:>11}  {} txs  hash {}",
                    info.name,
                    n,
                    block.transactions.len(),
                    block.hash
                );
                if let evmify::BlockTransactions::Full(txs) = &block.transactions
                    && let Some(tx) = txs.first()
                {
                    println!("{:<12} first tx as EVM JSON:", "");
                    println!("{}", serde_json::to_string_pretty(tx).unwrap());
                }
            }
            Ok(None) => println!("{:<12} latest block not available yet", info.name),
            Err(e) => println!("{:<12} error: {e}", info.name),
        }
    }
    Ok(())
}
