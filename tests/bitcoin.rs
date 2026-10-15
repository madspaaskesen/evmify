use omnirpc::adapters::bitcoin::{self, BitcoinAdapter, btc_to_sats};
use omnirpc::pipeline::{IdMapper, NATIVE_TOKEN_ADDRESS, ValueScale};
use omnirpc::{
    Address, B256, BlockId, BlockTransactions, ChainProvider, EvmTransformer, MockTransport, U256,
};
use serde_json::{Value, json};

const BLOCK_HASH: &str = "00000000000000000002a7c4c1e48d76c5a37902165a270156b7a8d72728a054";
const PREV_HASH: &str = "000000000000000000012c3d0e4c5b8f6dc2a0f7b8e2a7a9a6a9f4e1d3c2b1a0";
const COINBASE_TXID: &str = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b";
const PAY_TXID: &str = "f4184fc596403b9d638783cf57adfe4c75c605f6356fbc91338530e9831e9e16";
const MINER: &str = "bc1qminer0000000000000000000000000000000";
const ALICE: &str = "bc1qalice000000000000000000000000000000";
const BOB: &str = "bc1qbob00000000000000000000000000000000";

fn payment_tx() -> Value {
    json!({
        "txid": PAY_TXID,
        "hash": PAY_TXID,
        "vin": [{
            "txid": "0437cd7f8525ceed2324359c2d0ba26006d92d856a9c20fa0241106ee5a597c9",
            "vout": 0,
            "prevout": { "generated": false, "height": 799_000, "value": 1.0, "scriptPubKey": { "address": ALICE, "type": "witness_v0_keyhash" } }
        }],
        "vout": [
            { "value": 0.3,    "n": 0, "scriptPubKey": { "address": BOB,   "type": "witness_v0_keyhash" } },
            { "value": 0.6999, "n": 1, "scriptPubKey": { "address": ALICE, "type": "witness_v0_keyhash" } }
        ],
        "fee": 0.0001
    })
}

fn block() -> Value {
    json!({
        "hash": BLOCK_HASH,
        "height": 800_000,
        "previousblockhash": PREV_HASH,
        "time": 1_690_168_629,
        "tx": [
            {
                "txid": COINBASE_TXID,
                "vin": [{ "coinbase": "0300350c0101", "sequence": 4294967295u64 }],
                "vout": [
                    { "value": 3.1251, "n": 0, "scriptPubKey": { "address": MINER, "type": "witness_v0_keyhash" } },
                    { "value": 0.0,    "n": 1, "scriptPubKey": { "type": "nulldata" } }
                ]
            },
            payment_tx()
        ]
    })
}

#[test]
fn sats_conversion_is_exact() {
    assert_eq!(btc_to_sats(0.6999), 69_990_000);
    assert_eq!(btc_to_sats(20_999_999.976_9), 2_099_999_997_690_000);
    assert_eq!(btc_to_sats(0.00000001), 1);
}

#[tokio::test]
async fn block_in_evm_shape() {
    let mock = MockTransport::new()
        .on("getblockhash", json!(BLOCK_HASH))
        .on("getblock", block());
    let btc = bitcoin::pipeline(BitcoinAdapter::mainnet(mock.clone()));
    let b = btc
        .block(BlockId::Number(800_000), true)
        .await
        .unwrap()
        .unwrap();

    // Hex ids keep their value.
    assert_eq!(b.hash, BLOCK_HASH.parse::<B256>().unwrap());
    assert_eq!(b.parent_hash, PREV_HASH.parse::<B256>().unwrap());
    assert_eq!(
        b.miner,
        omnirpc::pipeline::DefaultMapper::default().address(MINER)
    );

    let BlockTransactions::Full(txs) = &b.transactions else {
        panic!()
    };
    let coinbase = &txs[0];
    assert_eq!(coinbase.from, Address::ZERO);

    let pay = &txs[1];
    assert_eq!(pay.hash, PAY_TXID.parse::<B256>().unwrap());
    // 0.3 BTC to Bob; change back to Alice is not counted as value.
    assert_eq!(
        pay.value,
        U256::from(30_000_000u64) * U256::from(10_000_000_000u64)
    );
    assert_eq!(pay.native.as_ref().unwrap().to.as_deref(), Some(BOB));
    assert_eq!(
        pay.native.as_ref().unwrap().extra["outputs"][1]["sats"],
        json!(69_990_000)
    );

    // getblock was asked for verbosity 3 (prevouts).
    let (_, params) = mock
        .calls()
        .iter()
        .find(|(m, _)| m == "getblock")
        .cloned()
        .unwrap();
    assert_eq!(params, json!([BLOCK_HASH, 3]));
}

#[tokio::test]
async fn lookup_by_evm_hash_round_trips_to_txid() {
    let mut tx = payment_tx();
    tx["blockhash"] = json!(BLOCK_HASH);
    let mock = MockTransport::new().on("getrawtransaction", tx).on(
        "getblockheader",
        json!({ "hash": BLOCK_HASH, "height": 800_000 }),
    );
    let btc = bitcoin::pipeline(BitcoinAdapter::mainnet(mock.clone()));

    let hash: B256 = PAY_TXID.parse().unwrap();
    let r = btc.receipt(hash).await.unwrap().unwrap();
    assert_eq!(mock.calls()[0].1, json!([PAY_TXID, 2]));
    assert_eq!(r.succeeded(), Some(true));
    assert_eq!(r.block_number.unwrap().to::<u64>(), 800_000);
    // fee = gasUsed * effectiveGasPrice = 10_000 sats, scaled to 18 decimals
    assert_eq!(
        r.gas_used * r.effective_gas_price.unwrap(),
        U256::from(10_000u64) * U256::from(10_000_000_000u64)
    );
    // One ERC-20 style Transfer per output, from the native-token address.
    assert_eq!(r.logs.len(), 2);
    assert!(r.logs.iter().all(|l| l.address == NATIVE_TOKEN_ADDRESS));
}

#[tokio::test]
async fn native_units_when_asked() {
    let mock = MockTransport::new()
        .on("getblockhash", json!(BLOCK_HASH))
        .on("getblock", block());
    let btc = bitcoin::pipeline(BitcoinAdapter::mainnet(mock)).with_transformer(
        EvmTransformer::new(omnirpc::pipeline::DefaultMapper::new(true))
            .with_scale(ValueScale::Native),
    );
    let b = btc
        .block(BlockId::Number(800_000), true)
        .await
        .unwrap()
        .unwrap();
    let BlockTransactions::Full(txs) = &b.transactions else {
        panic!()
    };
    assert_eq!(txs[1].value, U256::from(30_000_000u64));
}

#[tokio::test]
async fn unknown_height_and_txid_are_none() {
    let mock = MockTransport::new()
        .on_error("getblockhash", -8, "Block height out of range")
        .on_error(
            "getrawtransaction",
            -5,
            "No such mempool or blockchain transaction",
        );
    let btc = bitcoin::pipeline(BitcoinAdapter::mainnet(mock));
    assert!(
        btc.block(BlockId::Number(99_999_999), false)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        btc.transaction_by_native_id(PAY_TXID)
            .await
            .unwrap()
            .is_none()
    );
}
