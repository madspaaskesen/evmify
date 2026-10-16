//! Look up one Solana transaction by signature and print it as an EVM receipt,
//! with SOL / SPL token movements as ERC-20 style logs.
//!
//! ```sh
//! cargo run --example solana_tx -- <SIGNATURE> [CHAIN|RPC_URL]
//! cargo run --example solana_tx -- <SIGNATURE>                  # mainnet public RPC
//! cargo run --example solana_tx -- <SIGNATURE> solana-devnet
//! EVMIFY_RPC_SOLANA=https://my-rpc.example cargo run --example solana_tx -- <SIGNATURE>
//! ```

use evmify::ChainProvider;
use evmify::adapters::solana;

#[tokio::main]
async fn main() -> evmify::Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(signature) = args.next() else {
        eprintln!("usage: cargo run --example solana_tx -- <SIGNATURE> [CHAIN|RPC_URL]");
        std::process::exit(2);
    };
    let target = args.next().unwrap_or_else(|| "solana".into());

    let sol = solana::connect(&target)?;
    match sol.receipt_by_native_id(&signature).await? {
        Some(receipt) => println!("{}", serde_json::to_string_pretty(&receipt).unwrap()),
        None => eprintln!("transaction not found (too old for this RPC, or not confirmed yet)"),
    }
    Ok(())
}
