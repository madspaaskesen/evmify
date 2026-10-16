use evmify::adapters::solana::{self, SolanaAdapter};
use evmify::pipeline::{IdMapper, NATIVE_TOKEN_ADDRESS};
use evmify::{
    B256, BlockId, BlockTransactions, ChainProvider, Filter, MockTransport, U256,
    primitives::keccak256,
};
use serde_json::{Value, json};

const PAYER: &str = "7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU";
const ALICE_TOKEN_ACC: &str = "3emsAVdmGKERbHjmGfQ6oZ1e35dkf5iYcS6U4CPKFVaa";
const BOB: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
const BOB_TOKEN_ACC: &str = "HN7cABqLq46Es1jh92dQQisAq662SmxELLLsHHe4YWrH";
const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const COMPUTE_BUDGET: &str = "ComputeBudget111111111111111111111111111111";
const SYSTEM: &str = "11111111111111111111111111111111";
const USDC: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const LEADER: &str = "DRpbCBMxVnDK7maPM5tGv6MvB3v1sRMC86PZ8okm21hy";
const SIG_SOL: &str =
    "5VERv8NMvzbJMEkV8xnrLkEaWRtSz9CosKDYjCJjBRnbJLgp8uirBgmQpjKhoR4tjF3ZpRzrFmBV6UjKdiSZkQUW";
const SIG_USDC: &str =
    "4hXTCkRzt9WyecNzV1XPgCDfGAZzQKNxLXgynz5QDuWWPSAZBZSHptvWRL3BjCvzUXRdKvHL2b8yXrNPBR3e4Gm4";

fn tx_sol_transfer() -> Value {
    json!({
        "transaction": {
            "signatures": [SIG_SOL],
            "message": {
                "accountKeys": [PAYER, BOB, SYSTEM],
                "instructions": [{ "programIdIndex": 2, "accounts": [0, 1], "data": "3Bxs4h24hBtQy9rw" }],
                "recentBlockhash": "EkSnNWid2cvwEVnVx9aBqawnmiCNiDgp3gUdkDPTKN1N"
            }
        },
        "meta": {
            "err": null,
            "fee": 5000,
            "preBalances": [10_000_000_000u64, 0, 1],
            "postBalances": [8_999_995_000u64, 1_000_000_000u64, 1],
            "preTokenBalances": [],
            "postTokenBalances": [],
            "logMessages": ["Program 11111111111111111111111111111111 invoke [1]", "Program 11111111111111111111111111111111 success"],
            "computeUnitsConsumed": 150
        },
        "version": "legacy"
    })
}

/// v0 transaction: Bob's token account comes from an address lookup table.
fn tx_usdc_transfer() -> Value {
    json!({
        "transaction": {
            "signatures": [SIG_USDC],
            "message": {
                "accountKeys": [PAYER, ALICE_TOKEN_ACC, TOKEN_PROGRAM, COMPUTE_BUDGET],
                "instructions": [
                    { "programIdIndex": 3, "accounts": [], "data": "3DdGGhkhJbjm" },
                    { "programIdIndex": 2, "accounts": [1, 4, 0], "data": "3Bxs4ThwQbE4vyj5" }
                ]
            }
        },
        "meta": {
            "err": null,
            "fee": 5000,
            "preBalances": [5_000_000_000u64, 2_039_280, 1, 1, 2_039_280],
            "postBalances": [4_999_995_000u64, 2_039_280, 1, 1, 2_039_280],
            "preTokenBalances": [
                { "accountIndex": 1, "mint": USDC, "owner": PAYER, "uiTokenAmount": { "amount": "5000000", "decimals": 6 } },
                { "accountIndex": 4, "mint": USDC, "owner": BOB,   "uiTokenAmount": { "amount": "0", "decimals": 6 } }
            ],
            "postTokenBalances": [
                { "accountIndex": 1, "mint": USDC, "owner": PAYER, "uiTokenAmount": { "amount": "3000000", "decimals": 6 } },
                { "accountIndex": 4, "mint": USDC, "owner": BOB,   "uiTokenAmount": { "amount": "2000000", "decimals": 6 } }
            ],
            "loadedAddresses": { "writable": [BOB_TOKEN_ACC], "readonly": [] },
            "computeUnitsConsumed": 4645
        },
        "version": 0
    })
}

fn block() -> Value {
    json!({
        "blockhash": "8Hr2zBZDoKgX6fPu9XBsPK7a4ZuiUsDCsCUMmqLrSvTt",
        "previousBlockhash": "EkSnNWid2cvwEVnVx9aBqawnmiCNiDgp3gUdkDPTKN1N",
        "parentSlot": 299_999_999u64,
        "blockTime": 1_729_000_000,
        "blockHeight": 278_000_000u64,
        "transactions": [tx_sol_transfer(), tx_usdc_transfer()],
        "rewards": [{ "pubkey": LEADER, "lamports": 5000, "postBalance": 1, "rewardType": "Fee", "commission": null }]
    })
}

fn mapper_addr(s: &str) -> evmify::Address {
    evmify::pipeline::DefaultMapper::default().address(s)
}

#[tokio::test]
async fn block_in_evm_shape() {
    let mock = MockTransport::new().on("getBlock", block());
    let sol = solana::pipeline(SolanaAdapter::mainnet(mock.clone()));

    let b = sol
        .block(BlockId::Number(300_000_000), true)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(b.number.to::<u64>(), 300_000_000);
    assert_eq!(b.timestamp.to::<u64>(), 1_729_000_000);
    assert_eq!(b.miner, mapper_addr(LEADER));
    assert_eq!(
        b.hash,
        keccak256("8Hr2zBZDoKgX6fPu9XBsPK7a4ZuiUsDCsCUMmqLrSvTt")
    );
    assert_eq!(
        b.native.as_ref().unwrap().id,
        "8Hr2zBZDoKgX6fPu9XBsPK7a4ZuiUsDCsCUMmqLrSvTt"
    );

    let BlockTransactions::Full(txs) = &b.transactions else {
        panic!("expected full txs")
    };
    // Plain SOL transfer: recipient and amount, scaled to 18 decimals.
    let t = &txs[0];
    assert_eq!(t.hash, keccak256(SIG_SOL));
    assert_eq!(t.from, mapper_addr(PAYER));
    assert_eq!(t.to, Some(mapper_addr(BOB)));
    assert_eq!(t.value, U256::from(10u64).pow(U256::from(18))); // 1 SOL
    assert_eq!(
        t.gas * t.gas_price.unwrap(),
        U256::from(5000u64) * U256::from(1_000_000_000u64)
    );
    assert_eq!(t.block_number.unwrap().to::<u64>(), 300_000_000);
    assert_eq!(
        t.native.as_ref().unwrap().value.as_deref(),
        Some("1000000000")
    );

    // Token transfer: `to` is the program (compute budget skipped), value 0.
    let u = &txs[1];
    assert_eq!(u.to, Some(mapper_addr(TOKEN_PROGRAM)));
    assert_eq!(u.value, U256::ZERO);
    assert_eq!(u.transaction_index.unwrap().to::<u64>(), 1);
    // Lookup-table key resolved.
    let ixs = &u.native.as_ref().unwrap().extra["instructions"];
    assert_eq!(ixs[1]["accounts"][1], json!(BOB_TOKEN_ACC));

    // Request asks for v0 transactions.
    let (_, params) = &mock.calls()[0];
    assert_eq!(params[1]["maxSupportedTransactionVersion"], json!(1));
}

#[tokio::test]
async fn receipts_carry_erc20_style_logs() {
    let mut tx = tx_usdc_transfer();
    tx["slot"] = json!(300_000_000u64);
    tx["blockTime"] = json!(1_729_000_000);
    let mock = MockTransport::new().on("getTransaction", tx);
    let sol = solana::pipeline(SolanaAdapter::mainnet(mock));

    let r = sol.receipt_by_native_id(SIG_USDC).await.unwrap().unwrap();
    assert_eq!(r.succeeded(), Some(true));
    assert_eq!(r.block_number.unwrap().to::<u64>(), 300_000_000);

    let transfer = keccak256("Transfer(address,address,uint256)");
    let t = r
        .logs
        .iter()
        .find(|l| l.topics[0] == transfer)
        .expect("Transfer log");
    assert_eq!(t.address, mapper_addr(USDC)); // mint is the "token contract"
    assert_eq!(t.topics[1], mapper_addr(PAYER).into_word());
    assert_eq!(t.topics[2], mapper_addr(BOB).into_word());
    // Token amounts are not rescaled: 2 USDC with 6 decimals.
    assert_eq!(U256::from_be_slice(&t.data), U256::from(2_000_000u64));

    let changes: Vec<_> = r
        .logs
        .iter()
        .filter(|l| l.topics[0] == keccak256("BalanceChange(address,int256)"))
        .collect();
    assert_eq!(changes.len(), 2);
    let payer_change = changes
        .iter()
        .find(|l| l.topics[1] == mapper_addr(PAYER).into_word())
        .unwrap();
    let raw = U256::from_be_slice(&payer_change.data);
    assert_eq!(raw.wrapping_neg(), U256::from(2_000_000u64)); // int256(-2_000_000)
}

#[tokio::test]
async fn failed_tx_and_native_events() {
    let mut tx = tx_sol_transfer();
    tx["meta"]["err"] = json!({ "InstructionError": [0, { "Custom": 1 }] });
    tx["meta"]["postBalances"] = json!([9_999_995_000u64, 0, 1]);
    tx["slot"] = json!(1);
    let sol = solana::pipeline(SolanaAdapter::mainnet(
        MockTransport::new().on("getTransaction", tx),
    ));

    let r = sol.receipt_by_native_id(SIG_SOL).await.unwrap().unwrap();
    assert_eq!(r.succeeded(), Some(false));
    assert!(
        r.logs.is_empty(),
        "only the fee moved, so no balance-change logs"
    );
    assert_eq!(
        r.native.unwrap().extra["error"]["InstructionError"][0],
        json!(0)
    );
}

#[tokio::test]
async fn skipped_slot_is_none() {
    let mock = MockTransport::new().on_error(
        "getBlock",
        -32007,
        "Slot 300000001 was skipped, or missing due to ledger jump to recent snapshot",
    );
    let sol = solana::pipeline(SolanaAdapter::mainnet(mock));
    assert!(
        sol.block(BlockId::Number(300_000_001), false)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn hash_lookup_after_block_fetch_and_log_scan() {
    let mut tx = tx_sol_transfer();
    tx["slot"] = json!(300_000_000u64);
    let mock = MockTransport::new()
        .on("getSlot", json!(300_000_000u64))
        .on("getBlock", block())
        .on("getTransaction", tx);
    let sol = solana::pipeline(SolanaAdapter::mainnet(mock.clone()));

    // One-way hashes: unknown until seen.
    let h: B256 = keccak256(SIG_SOL);
    assert!(sol.transaction(h).await.is_err());

    let latest = sol.block(BlockId::Latest, false).await.unwrap().unwrap();
    assert!(latest.transactions.hashes().contains(&h));
    let t = sol.transaction(h).await.unwrap().unwrap();
    assert_eq!(t.hash, h);
    let (method, params) = mock.calls().last().cloned().unwrap();
    assert_eq!(
        (method.as_str(), params[0].as_str()),
        ("getTransaction", Some(SIG_SOL))
    );

    // eth_getLogs-style scan over the latest block: native SOL transfers only.
    let logs = sol
        .logs(
            &Filter::new()
                .address(NATIVE_TOKEN_ADDRESS)
                .event("Transfer(address,address,uint256)"),
        )
        .await
        .unwrap();
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].topics[2], mapper_addr(BOB).into_word());
    // Native amounts in logs are scaled like `value`.
    assert_eq!(
        U256::from_be_slice(&logs[0].data),
        U256::from(10u64).pow(U256::from(18))
    );

    // Log indexes are block-level: the USDC tx's logs continue after the SOL tx's three.
    let all = sol.logs(&Filter::new()).await.unwrap();
    let idx: Vec<u64> = all.iter().map(|l| l.log_index.unwrap().to()).collect();
    assert_eq!(idx, (0..all.len() as u64).collect::<Vec<_>>());

    // Ranges are capped.
    let too_wide = Filter::new().from_block(0).to_block(1_000);
    assert!(sol.logs(&too_wide).await.is_err());
}

/// v1 transaction: no lookup tables, compute budget in `transactionConfig`
/// instead of ComputeBudget instructions.
fn tx_v1_sol_transfer() -> Value {
    json!({
        "version": 1,
        "transaction": {
            "signatures": [SIG_SOL],
            "message": {
                "accountKeys": [PAYER, BOB, SYSTEM],
                "instructions": [{ "programIdIndex": 2, "accounts": [0, 1], "data": "3Bxs4h24hBtQy9rw", "stackHeight": null }],
                "transactionConfig": {
                    "computeUnitLimit": 450,
                    "heapSize": null,
                    "loadedAccountsDataSizeLimit": 149,
                    "priorityFee": 5000
                }
            }
        },
        "meta": {
            "err": null,
            "fee": 10000,
            "preBalances": [10_000_000_000u64, 0, 1],
            "postBalances": [8_999_990_000u64, 1_000_000_000u64, 1],
            "computeUnitsConsumed": 150
        }
    })
}

#[tokio::test]
async fn v1_transactions_in_a_block() {
    let mut b = block();
    b["transactions"] = json!([tx_v1_sol_transfer(), tx_usdc_transfer()]);
    let mock = MockTransport::new().on("getBlock", b);
    let sol = solana::pipeline(SolanaAdapter::mainnet(mock));

    let blk = sol
        .block(BlockId::Number(300_000_000), true)
        .await
        .unwrap()
        .unwrap();
    let BlockTransactions::Full(txs) = &blk.transactions else {
        panic!("expected full txs")
    };
    let t = &txs[0];
    assert_eq!(t.to, Some(mapper_addr(BOB)));
    assert_eq!(t.value, U256::from(10u64).pow(U256::from(18)));
    // fee 10_000 lamports (base + 5_000 priority) -> gasUsed * gasPrice
    assert_eq!(
        t.gas * t.gas_price.unwrap(),
        U256::from(10_000u64) * U256::from(1_000_000_000u64)
    );
    let extra = &t.native.as_ref().unwrap().extra;
    assert_eq!(extra["version"], json!(1));
    assert_eq!(extra["transactionConfig"]["priorityFee"], json!(5000));
    // v0 tx in the same block still resolves its lookup-table keys
    assert_eq!(txs[1].native.as_ref().unwrap().extra["version"], json!(0));
}
