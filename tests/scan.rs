use std::sync::{Arc, Mutex};
use std::time::Duration;

use evmify::{EvmClient, Filter, MockTransport};
use serde_json::{Value, json};

const TRANSFER: &str = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";

fn log(block: u64, index: u64) -> Value {
    json!({
        "address": "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913",
        "topics": [TRANSFER],
        "data": "0x",
        "blockNumber": format!("{block:#x}"),
        "transactionHash": format!("0x{:064x}", block * 1000 + index),
        "logIndex": format!("{index:#x}")
    })
}

fn ranges(mock: &MockTransport) -> Vec<(u64, u64)> {
    let hex =
        |v: &Value| u64::from_str_radix(v.as_str().unwrap().trim_start_matches("0x"), 16).unwrap();
    mock.calls()
        .iter()
        .filter(|(m, _)| m == "eth_getLogs")
        .map(|(_, p)| (hex(&p[0]["fromBlock"]), hex(&p[0]["toBlock"])))
        .collect()
}

fn client(mock: &MockTransport) -> EvmClient<MockTransport> {
    EvmClient::new(mock.clone(), evmify::chains::by_key("ronin").unwrap())
}

#[tokio::test]
async fn newest_first_stops_at_the_limit() {
    let mock = MockTransport::new()
        .on("eth_blockNumber", json!("0x3e8")) // head = 1000
        .on(
            "eth_getLogs",
            json!([log(990, 0), log(995, 1), log(999, 0)]),
        );
    let c = client(&mock);

    let seen = Arc::new(Mutex::new(Vec::new()));
    let progress = seen.clone();
    let logs = c
        .scan_logs(Filter::new().event("Transfer(address,address,uint256)"))
        .newest_first()
        .limit(5)
        .max_blocks(1000)
        .chunk(100)
        .on_progress(move |p| progress.lock().unwrap().push(*p))
        .run()
        .await
        .unwrap();

    // Two windows of 3 logs each are enough for 5: the scan stops early.
    assert_eq!(ranges(&mock), vec![(901, 1000), (801, 900)]);
    assert_eq!(logs.len(), 5);
    // Newest first within each window.
    let blocks: Vec<u64> = logs.iter().map(|l| l.block_number.unwrap().to()).collect();
    assert_eq!(blocks, vec![999, 995, 990, 999, 995]);

    let seen = seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    assert_eq!(
        (seen[1].scanned, seen[1].total, seen[1].found),
        (200, 1000, 6)
    );
}

#[tokio::test]
async fn or_filters_are_merged_and_deduplicated() {
    // Both filters return the same self-transfer plus one log of their own.
    let mock = MockTransport::new()
        .on("eth_getLogs", json!([log(10, 0), log(12, 3)]))
        .on("eth_getLogs", json!([log(10, 0), log(11, 1)]));
    let c = client(&mock);
    let logs = c
        .scan_logs(Filter::new())
        .or_filter(Filter::new())
        .from_block(0)
        .to_block(99)
        .run()
        .await
        .unwrap();
    let keys: Vec<(u64, u64)> = logs
        .iter()
        .map(|l| (l.block_number.unwrap().to(), l.log_index.unwrap().to()))
        .collect();
    assert_eq!(keys, vec![(10, 0), (11, 1), (12, 3)]);
}

#[tokio::test]
async fn newest_first_halves_rejected_windows() {
    let mock = MockTransport::new()
        .on_error("eth_getLogs", -32602, "Invalid params")
        .on("eth_getLogs", json!([]));
    let c = client(&mock);
    let logs = c
        .scan_logs(Filter::new())
        .newest_first()
        .from_block(500)
        .to_block(899)
        .chunk(400)
        .run()
        .await
        .unwrap();
    assert!(logs.is_empty());
    assert_eq!(
        ranges(&mock),
        vec![(500, 899), (700, 899), (500, 699)] // 400 rejected -> 200
    );
}

#[tokio::test]
async fn plan_estimates_requests_without_scanning() {
    let mock = MockTransport::new().on("eth_blockNumber", json!("0x2710")); // 10_000
    let c = client(&mock);
    let mut scan = c
        .scan_logs(Filter::new())
        .or_filter(Filter::new())
        .max_blocks(50_000)
        .chunk(200)
        .rate_limit(5.0);
    let plan = scan.plan().await.unwrap();
    assert_eq!((plan.from, plan.to, plan.blocks), (0, 10_000, 10_001));
    assert_eq!(plan.max_requests, 51 * 2);
    assert_eq!(plan.min_duration, Some(Duration::from_millis(200) * 101));
    assert!(ranges(&mock).is_empty(), "plan makes no eth_getLogs calls");
}

#[tokio::test]
async fn rate_limit_spaces_requests() {
    let mock = MockTransport::new().on("eth_getLogs", json!([]));
    let c = client(&mock);
    let started = std::time::Instant::now();
    c.scan_logs(Filter::new())
        .from_block(0)
        .to_block(299)
        .chunk(100)
        .rate_limit(20.0) // 50 ms apart
        .run()
        .await
        .unwrap();
    assert_eq!(ranges(&mock).len(), 3);
    assert!(started.elapsed() >= Duration::from_millis(100));
}

#[tokio::test]
async fn blocks_in_uses_measured_block_time() {
    let block = |n: u64, ts: u64| {
        json!({
            "number": format!("{n:#x}"),
            "hash": format!("0x{n:064x}"),
            "parentHash": format!("0x{:064x}", n - 1),
            "timestamp": format!("{ts:#x}"),
            "transactions": []
        })
    };
    // Head 50_000 at t=1_000_000, block 49_000 at t=997_000: 3 s per block (Ronin).
    let mock = MockTransport::new()
        .on("eth_getBlockByNumber", block(50_000, 1_000_000))
        .on("eth_getBlockByNumber", block(49_000, 997_000));
    let c = client(&mock);
    let day = c.blocks_in(Duration::from_secs(24 * 3600)).await.unwrap();
    assert_eq!(day, 28_800);
    let calls = mock.calls();
    assert_eq!(calls[0].1, json!(["latest", false]));
    assert_eq!(calls[1].1, json!([format!("{:#x}", 49_000), false]));
}

#[tokio::test]
async fn block_at_finds_the_exact_block() {
    // Block n has timestamp 1_000_000 + 2n (a 2-second chain), head = 62_000_000.
    let head: u64 = 62_000_000;
    let ts = |n: u64| 1_000_000 + 2 * n;
    let block = move |n: u64| {
        json!({
            "number": format!("{n:#x}"),
            "hash": format!("0x{n:064x}"),
            "parentHash": format!("0x{:064x}", n.saturating_sub(1)),
            "timestamp": format!("{:#x}", ts(n)),
            "transactions": []
        })
    };
    let mock = MockTransport::new().on_fn("eth_getBlockByNumber", move |params| {
        let n = match params[0].as_str().unwrap() {
            "latest" => head,
            hex => u64::from_str_radix(hex.trim_start_matches("0x"), 16).unwrap(),
        };
        block(n)
    });
    let c = client(&mock);

    // Exactly on a block, between two blocks, before genesis, after the head.
    assert_eq!(c.block_at(ts(48_123_456)).await.unwrap(), 48_123_456);
    assert_eq!(c.block_at(ts(48_123_456) + 1).await.unwrap(), 48_123_457);
    assert_eq!(c.block_at(0).await.unwrap(), 0);
    assert_eq!(c.block_at(u64::MAX).await.unwrap(), head);

    let calls = mock.calls().len();
    c.block_at(ts(1_234_567)).await.unwrap();
    let used = mock.calls().len() - calls;
    assert!(used <= 28, "binary search used {used} calls");
}

/// A 2-second chain whose node only serves the last `kept` blocks, like most public RPCs.
fn pruned_chain(head: u64, kept: u64) -> MockTransport {
    MockTransport::new().on_fn("eth_getBlockByNumber", move |params| {
        let n = match params[0].as_str().unwrap() {
            "latest" => head,
            hex => u64::from_str_radix(hex.trim_start_matches("0x"), 16).unwrap(),
        };
        if n + kept <= head {
            return Value::Null; // pruned
        }
        json!({
            "number": format!("{n:#x}"),
            "hash": format!("0x{n:064x}"),
            "parentHash": format!("0x{:064x}", n.saturating_sub(1)),
            "timestamp": format!("{:#x}", 1_000_000 + 2 * n),
            "transactions": []
        })
    })
}

#[tokio::test]
async fn block_at_works_on_a_non_archive_node() {
    // Head 62M, node keeps ~2 weeks (604 800 blocks at 2 s).
    let head = 62_355_777;
    let mock = pruned_chain(head, 604_800);
    let c = client(&mock);
    let two_days_ago = 1_000_000 + 2 * head - 2 * 86_400;
    let n = c.block_at(two_days_ago).await.unwrap();
    assert_eq!(n, head - 86_400);
    // Never asked for anything the node doesn't have, and stayed cheap.
    assert!(mock.calls().len() <= 30, "{} calls", mock.calls().len());

    // A date older than the node's history fails with a helpful message.
    let mock = pruned_chain(head, 604_800);
    let err = client(&mock)
        .block_at(1_000_000 + 2 * (head - 1_000_000))
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("archive"), "{err}");
}
