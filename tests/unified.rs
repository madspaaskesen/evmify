//! EVM and non-EVM chains behind one trait object.

use evmify::adapters::{bitcoin, solana};
use evmify::{BlockId, ChainKind, ChainProvider, EvmClient, MockTransport};
use serde_json::json;

#[tokio::test]
async fn heterogeneous_chains_share_one_interface() {
    let eth = MockTransport::new().on("eth_blockNumber", json!("0x1406f40"));
    let sol = MockTransport::new().on("getSlot", json!(300_000_000u64));
    let btc = MockTransport::new().on("getblockcount", json!(870_000));

    let chains: Vec<Box<dyn ChainProvider>> = vec![
        Box::new(EvmClient::new(
            eth,
            evmify::chains::by_key("ethereum").unwrap(),
        )),
        Box::new(solana::pipeline(solana::SolanaAdapter::mainnet(sol))),
        Box::new(bitcoin::pipeline(bitcoin::BitcoinAdapter::mainnet(btc))),
    ];

    let mut heads = Vec::new();
    for c in &chains {
        heads.push((c.chain().kind, c.block_number().await.unwrap()));
    }
    assert_eq!(
        heads,
        vec![
            (ChainKind::Evm, 21_000_000),
            (ChainKind::Solana, 300_000_000),
            (ChainKind::Bitcoin, 870_000),
        ]
    );

    // Bitcoin "finalized" = 6 confirmations deep.
    let btc = MockTransport::new()
        .on("getblockcount", json!(870_000))
        .on("getblockhash", json!("00".repeat(32)))
        .on(
            "getblock",
            json!({ "hash": "00".repeat(32), "height": 869_994, "time": 0, "tx": [] }),
        );
    let p = bitcoin::pipeline(bitcoin::BitcoinAdapter::mainnet(btc.clone()));
    let b = p.block(BlockId::Finalized, false).await.unwrap().unwrap();
    assert_eq!(b.number.to::<u64>(), 869_994);
    assert_eq!(btc.calls()[1].1, json!([869_994]));
}
