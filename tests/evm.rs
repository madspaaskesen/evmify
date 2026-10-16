use std::sync::Arc;

use evmify::{
    Address, B256, BlockId, BlockTransactions, ChainProvider, Error, EvmClient, FallbackTransport,
    Filter, MockTransport, Transport, U256,
};
use serde_json::json;

const TX: &str = "0x88df016429689c079f3b2f6ad39fa052532c56795b733da78a91ebe6a713944b";

fn base_block() -> serde_json::Value {
    json!({
        "number": "0x1b4",
        "hash": "0xdc0818cf78f21a8e70579cb46a43643f78291264dda342ae31049421c82d21ae",
        "parentHash": "0xe99e022112df268087ea7eafaf4790497fd21dbeeb6bd7a1721df161a6657a54",
        "timestamp": "0x55ba467c",
        "miner": "0x4200000000000000000000000000000000000011",
        "gasUsed": "0x9f759",
        "gasLimit": "0x1c9c380",
        "baseFeePerGas": "0x7",
        "transactions": [TX],
        // OP-stack / L2 specific: must survive untouched.
        "l1BlockNumber": "0x12a05f2",
        "withdrawalsRoot": "0x56e81f171bcc55a6ff8345e692c0f86e5b48e01b996cadc001622fb5e363b421"
    })
}

#[tokio::test]
async fn detects_chain_and_reads_block_with_unknown_fields() {
    let mock = MockTransport::new()
        .on("eth_chainId", json!("0x2105"))
        .on("eth_getBlockByNumber", base_block());
    let client = EvmClient::detect(mock.clone()).await.unwrap();
    assert_eq!(client.chain_info().name, "Base");
    assert_eq!(client.chain_info().evm_chain_id, Some(8453));

    let block = client
        .get_block(BlockId::Number(436), false)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(block.number.to::<u64>(), 436);
    assert_eq!(block.base_fee_per_gas, Some(U256::from(7)));
    assert!(matches!(&block.transactions, BlockTransactions::Hashes(h) if h.len() == 1));
    assert_eq!(block.other["l1BlockNumber"], json!("0x12a05f2"));

    // Round-trip keeps the extra fields and the hex-quantity encoding.
    let back = serde_json::to_value(&block).unwrap();
    assert_eq!(back["l1BlockNumber"], json!("0x12a05f2"));
    assert_eq!(back["number"], json!("0x1b4"));
    assert!(back.get("_native").is_none());

    // Params sent as the spec wants them.
    let (method, params) = mock.calls().last().cloned().unwrap();
    assert_eq!(method, "eth_getBlockByNumber");
    assert_eq!(params, json!(["0x1b4", false]));
}

#[tokio::test]
async fn receipt_and_logs() {
    let receipt = json!({
        "transactionHash": TX,
        "transactionIndex": "0x0",
        "blockHash": "0xdc0818cf78f21a8e70579cb46a43643f78291264dda342ae31049421c82d21ae",
        "blockNumber": "0x1b4",
        "from": "0xa7d9ddbe1f17865597fbd27ec712455208b6b76d",
        "to": "0xf02c1c8e6114b1dbe8937a39260b5b0a374432bb",
        "contractAddress": null,
        "cumulativeGasUsed": "0x5208",
        "gasUsed": "0x5208",
        "effectiveGasPrice": "0x3b9aca00",
        "status": "0x1",
        "type": "0x2",
        "logsBloom": "0x00",
        "logs": [{
            "address": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
            "topics": [
                "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",
                "0x000000000000000000000000a7d9ddbe1f17865597fbd27ec712455208b6b76d",
                "0x000000000000000000000000f02c1c8e6114b1dbe8937a39260b5b0a374432bb"
            ],
            "data": "0x00000000000000000000000000000000000000000000000000000000000f4240",
            "blockNumber": "0x1b4",
            "transactionHash": TX,
            "logIndex": "0x0",
            "removed": false
        }]
    });
    let mock = MockTransport::new()
        .on("eth_getTransactionReceipt", receipt.clone())
        .on("eth_getLogs", json!([receipt["logs"][0]]));
    let client = EvmClient::new(mock.clone(), evmify::chains::by_id(8453).unwrap());

    let r = client
        .get_transaction_receipt(TX.parse().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(r.succeeded(), Some(true));
    assert_eq!(r.logs.len(), 1);
    assert_eq!(r.other["type"], json!("0x2"));

    let filter = Filter::new()
        .from_block(436)
        .to_block(BlockId::Latest)
        .address(
            "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"
                .parse()
                .unwrap(),
        )
        .event("Transfer(address,address,uint256)");
    let logs = client.get_logs(&filter).await.unwrap();
    assert!(filter.matches(&logs[0]));
    assert_eq!(
        serde_json::to_value(&filter).unwrap(),
        json!({
            "fromBlock": "0x1b4",
            "toBlock": "latest",
            "address": ["0x833589fcd6edb6e08f4c7c32d4f71b54bda02913"],
            "topics": [["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"]]
        })
    );
}

#[tokio::test]
async fn rpc_errors_are_structured() {
    let mock = MockTransport::new().on_error("eth_call", 3, "execution reverted");
    let client = EvmClient::new(mock, evmify::ChainInfo::evm_unknown(1337));
    let err = client
        .call(
            &evmify::CallRequest::new(Address::ZERO, vec![0u8; 4]),
            BlockId::Latest,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Rpc { code: 3, .. }));
}

#[tokio::test]
async fn fallback_moves_on_only_for_transport_errors() {
    let dead = MockTransport::new(); // no responses -> transport error
    let alive = MockTransport::new()
        .on("eth_blockNumber", json!("0x10"))
        .on_error("eth_call", 3, "execution reverted");
    let fallback = FallbackTransport::new(vec![
        Arc::new(dead.clone()) as Arc<dyn Transport>,
        Arc::new(alive.clone()),
    ]);
    let client = EvmClient::new(fallback, evmify::chains::by_id(1).unwrap());
    assert_eq!(ChainProvider::block_number(&client).await.unwrap(), 16);

    // RPC error from the live node is returned, not retried elsewhere.
    let err = client
        .call(
            &evmify::CallRequest::new(Address::ZERO, vec![]),
            BlockId::Latest,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Rpc { .. }));
    assert_eq!(dead.calls().len(), 2);
    assert_eq!(alive.calls().len(), 2);
}

#[tokio::test]
async fn missing_block_is_none() {
    let mock = MockTransport::new().on("eth_getBlockByHash", json!(null));
    let client = EvmClient::new(mock, evmify::chains::by_id(1).unwrap());
    assert!(
        client
            .get_block_by_hash(B256::ZERO, true)
            .await
            .unwrap()
            .is_none()
    );
}

#[test]
fn rpc_error_message_includes_data() {
    // Ronin's actual answer to a 501-block eth_getLogs.
    let err = evmify::transport::JsonRpcResponse {
        result: None,
        error: Some(evmify::transport::JsonRpcError {
            code: -32602,
            message: "Invalid params".into(),
            data: Some(json!(
                "requested block range 501 exceeds the limit of 200; narrow your fromBlock/toBlock"
            )),
        }),
    }
    .into_result()
    .unwrap_err();
    assert_eq!(
        err.to_string(),
        "rpc error -32602: Invalid params (requested block range 501 exceeds the limit of 200; narrow your fromBlock/toBlock)"
    );
}

#[tokio::test]
async fn get_logs_chunked_splits_and_halves_on_range_errors() {
    let log = json!({
        "address": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
        "topics": ["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"],
        "data": "0x",
        "blockNumber": "0x1",
        "logIndex": "0x0"
    });
    // First window is rejected as too large, every later call succeeds.
    let mock = MockTransport::new()
        .on_error("eth_getLogs", -32602, "Invalid params")
        .on("eth_getLogs", json!([log]));
    let client = EvmClient::new(mock.clone(), evmify::chains::by_id(2020).unwrap());

    let filter = Filter::new().event("Transfer(address,address,uint256)");
    let logs = client
        .get_logs_chunked(&filter, 100, 349, 200)
        .await
        .unwrap();

    let ranges: Vec<(String, String)> = mock
        .calls()
        .iter()
        .map(|(_, p)| {
            (
                p[0]["fromBlock"].as_str().unwrap().to_string(),
                p[0]["toBlock"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let hex = |n: u64| format!("{n:#x}");
    assert_eq!(
        ranges,
        vec![
            (hex(100), hex(299)), // 200 blocks: rejected
            (hex(100), hex(199)), // halved to 100
            (hex(200), hex(299)),
            (hex(300), hex(349)), // last, shorter window
        ]
    );
    assert_eq!(logs.len(), 3);
    assert_eq!(client.chain_info().name, "Ronin");
}

#[tokio::test]
async fn get_logs_chunked_gives_up_on_other_errors() {
    let mock = MockTransport::new().on_error("eth_getLogs", -32601, "method not found");
    let client = EvmClient::new(mock.clone(), evmify::chains::by_id(1).unwrap());
    let err = client
        .get_logs_chunked(&Filter::new(), 0, 1000, 100)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Rpc { code: -32601, .. }));
    assert_eq!(mock.calls().len(), 1);
}

#[tokio::test]
async fn detect_expecting_catches_the_wrong_chain() {
    // e.g. EVMIFY_RPC_RONIN accidentally pointing at an Ethereum node
    let mock = MockTransport::new().on("eth_chainId", json!("0x1"));
    let err = EvmClient::detect_expecting(mock, 2020).await.unwrap_err();
    assert!(matches!(
        err,
        Error::ChainMismatch {
            expected: 2020,
            actual: 1
        }
    ));
    assert!(err.to_string().contains("expected chain id 2020"));

    let ok = MockTransport::new().on("eth_chainId", json!("0x7e4"));
    let client = EvmClient::detect_expecting(ok, 2020).await.unwrap();
    assert_eq!(client.chain_info().key, "ronin");
}

#[cfg(feature = "http")]
#[tokio::test]
async fn non_evm_keys_are_refused_by_evm_client() {
    let err = EvmClient::connect("solana").await.unwrap_err().to_string();
    assert!(err.contains("not an EVM chain"), "{err}");
    let err = EvmClient::connect("no-such-chain")
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("unknown chain"), "{err}");
}
