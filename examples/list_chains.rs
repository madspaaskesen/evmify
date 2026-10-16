//! List the built-in chains, their shorthand keys and which RPC each key resolves to.
//!
//! ```sh
//! cargo run --example list_chains
//! EVMIFY_RPC_RONIN=https://my-node.example cargo run --example list_chains
//! ```

use evmify::chains::{KNOWN_CHAINS, RpcSource, resolve_rpc};

fn main() {
    println!(
        "{:<16} {:>9}  {:<20} {:<6} RPC",
        "KEY", "CHAIN ID", "NAME", "SYMBOL"
    );
    for chain in KNOWN_CHAINS {
        let id = chain
            .evm_chain_id
            .map(|i| i.to_string())
            .unwrap_or_else(|| "-".into());
        let rpc = match resolve_rpc(chain.key) {
            Ok(r) => match r.source {
                RpcSource::Env(var) => format!("{}  (from {var})", r.url),
                _ => r.url,
            },
            Err(_) => format!("(none: set {})", chain.env_var()),
        };
        println!(
            "{:<16} {:>9}  {:<20} {:<6} {rpc}",
            chain.key, id, chain.name, chain.symbol
        );
    }
    println!();
    println!("Use a key anywhere an RPC URL is expected, e.g.");
    println!("  cargo run --example address_info -- ronin 0xYourAddress");
    println!("Override a key's RPC with EVMIFY_RPC_<KEY> (upper case, - as _), or pass a URL.");
    println!("Public RPCs are rate limited: fine for trying things, not for production.");
}
