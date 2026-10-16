//! Latest block from several chains, all through one trait and one output shape.
//!
//! ```sh
//! cargo run --example multi_chain
//! # also Bitcoin, from your own Bitcoin Core 25+ node:
//! EVMIFY_RPC_BITCOIN=http://user:pass@127.0.0.1:8332 cargo run --example multi_chain
//! ```

use std::sync::Arc;

use evmify::adapters::{bitcoin, solana};
use evmify::{BlockId, ChainProvider, EvmClient, FallbackTransport, HttpTransport, Transport};

#[tokio::main]
async fn main() -> evmify::Result<()> {
    let mut chains: Vec<Box<dyn ChainProvider>> = Vec::new();

    // EVM chains by shorthand key (see `--example list_chains`), or by URL.
    for key in ["ethereum", "base", "ronin"] {
        match EvmClient::connect(key).await {
            Ok(c) => chains.push(Box::new(c)),
            Err(e) => eprintln!("skip {key}: {e}"),
        }
    }

    // Several endpoints for one chain, tried in order on network failure.
    let arbitrum = FallbackTransport::new(vec![
        Arc::new(HttpTransport::new("https://arb1.arbitrum.io/rpc")) as Arc<dyn Transport>,
        Arc::new(HttpTransport::new(
            "https://arbitrum-one-rpc.publicnode.com",
        )),
    ]);
    match EvmClient::detect_expecting(arbitrum, 42161).await {
        Ok(c) => chains.push(Box::new(c)),
        Err(e) => eprintln!("skip arbitrum: {e}"),
    }

    // Solana, served as EVM.
    chains.push(Box::new(solana::connect("solana")?));

    // Bitcoin, served as EVM (only when EVMIFY_RPC_BITCOIN points at your node).
    if std::env::var("EVMIFY_RPC_BITCOIN").is_ok() {
        chains.push(Box::new(bitcoin::connect("bitcoin")?));
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
