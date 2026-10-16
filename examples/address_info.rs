//! Inspect an address on any EVM chain: native balance, nonce, wallet or contract, and
//! (with `--transfers`) its latest ERC-20 / ERC-721 transfers, newest first.
//!
//! ```sh
//! cargo run --example address_info -- ronin 0xYourAddress
//! cargo run --example address_info -- ronin 0xYourAddress --transfers
//! cargo run --example address_info -- ronin 0xYourAddress --transfers --since 24h
//! cargo run --example address_info -- ethereum 0xYourAddress --transfers --limit 50 --blocks 20000
//! cargo run --example address_info -- ronin 0xYourAddress --transfers --from 2026-10-01 --to 2026-10-08
//! cargo run --example address_info -- ronin 0xYourAddress --transfers --from-block 62300000 --oldest-first
//! cargo run --example address_info -- --help
//! ```
//!
//! `CHAIN` is a key from `cargo run --example list_chains`, or any RPC URL.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::io::{IsTerminal, Write};
use std::time::Duration;

use clap::Parser;
use evmify::chains::RpcSource;
use evmify::{
    Address, BlockId, Bytes, CallRequest, EvmClient, Filter, HttpTransport, Log, Transport, U256,
};

const TRANSFER: &str = "Transfer(address,address,uint256)";
const SYMBOL: [u8; 4] = [0x95, 0xd8, 0x9b, 0x41]; // symbol()
const DECIMALS: [u8; 4] = [0x31, 0x3c, 0xe5, 0x67]; // decimals()
/// Ask before scans that may need more calls than this.
const CONFIRM_ABOVE: u64 = 100;

#[derive(Parser)]
#[command(about = "Balance, nonce and recent token transfers of an address on any EVM chain")]
struct Args {
    /// Chain key (see `--example list_chains`) or RPC URL.
    chain: String,
    /// The 0x address to inspect.
    address: Address,

    /// Also list recent ERC-20 / ERC-721 transfers, newest first.
    #[arg(short, long)]
    transfers: bool,
    /// Stop after this many transfers.
    #[arg(long, default_value_t = 20)]
    limit: usize,
    /// Look back at most this many blocks from the end of the range.
    #[arg(long, default_value_t = evmify::scan::DEFAULT_MAX_BLOCKS, conflicts_with_all = ["since", "from", "from_block"])]
    blocks: u64,
    /// Look back this far in time instead, e.g. 30m, 24h, 7d.
    #[arg(long, value_parser = parse_duration, conflicts_with_all = ["from", "from_block"])]
    since: Option<Duration>,
    /// Start at this date/time (UTC), e.g. 2026-10-01 or 2026-10-01T12:00:00Z.
    #[arg(long, value_parser = parse_datetime, conflicts_with = "from_block")]
    from: Option<u64>,
    /// End at this date/time (UTC) [default: now].
    #[arg(long, value_parser = parse_datetime, conflicts_with = "to_block")]
    to: Option<u64>,
    /// Start at this block, e.g. to resume an earlier scan.
    #[arg(long)]
    from_block: Option<u64>,
    /// End at this block [default: the latest].
    #[arg(long)]
    to_block: Option<u64>,
    /// Walk forwards from the start of the range instead of backwards from the end.
    #[arg(long)]
    oldest_first: bool,
    /// Blocks per eth_getLogs call (halved automatically if the RPC refuses).
    #[arg(long, default_value_t = evmify::scan::DEFAULT_CHUNK)]
    chunk: u64,
    /// Max requests per second [default: 5 on public RPCs, unlimited otherwise; 0 = unlimited].
    #[arg(long)]
    rps: Option<f64>,
    /// Don't ask before long scans.
    #[arg(short, long)]
    yes: bool,
}

#[tokio::main]
async fn main() -> evmify::Result<()> {
    let args = Args::parse();
    let resolved = evmify::chains::resolve_rpc(&args.chain)?;
    // One request budget for everything this run does (dates, scan, token names).
    let rps = args.rps.unwrap_or(if resolved.source == RpcSource::Public {
        5.0
    } else {
        0.0
    });
    let transport = HttpTransport::new(resolved.url.clone()).max_rps(rps);
    let client = match resolved.chain {
        Some(chain) => match chain.evm_chain_id {
            Some(id) => EvmClient::detect_expecting(transport, id).await?,
            None => {
                eprintln!("`{}` is not an EVM chain", chain.key);
                std::process::exit(2);
            }
        },
        None => EvmClient::detect(transport).await?,
    };
    let chain = client.chain_info().clone();

    let balance = client.get_balance(args.address, BlockId::Latest).await?;
    let nonce = client
        .get_transaction_count(args.address, BlockId::Latest)
        .await?;
    let code = client.get_code(args.address, BlockId::Latest).await?;

    println!(
        "chain     {} (chain id {})",
        chain.name,
        chain.evm_chain_id.unwrap_or_default()
    );
    println!("rpc       {} ({:?})", resolved.url, resolved.source);
    println!("address   {}", args.address);
    println!(
        "kind      {}",
        if code.is_empty() {
            "wallet (EOA)"
        } else {
            "contract"
        }
    );
    println!(
        "balance   {} {}",
        format_units(balance, chain.native_decimals),
        chain.native_symbol
    );
    println!("sent      {nonce} transactions");

    if !args.transfers {
        println!("\nadd --transfers to list recent token transfers (see --help)");
        return Ok(());
    }

    // Transfer(from, to, value): topic 1 = from, topic 2 = to.
    let me = args.address.into_word();
    let transfer = Filter::new().event(TRANSFER);
    // Resolve the range: dates become exact blocks via binary search on block timestamps.
    if args.from.is_some() || args.to.is_some() {
        eprintln!(
            "finding the blocks for {} … {}{}",
            args.from
                .map(format_utc)
                .unwrap_or_else(|| "(start)".into()),
            args.to.map(format_utc).unwrap_or_else(|| "now".into()),
            if rps > 0.0 {
                format!(" (≈15-30 lookups each at {rps} req/s)…")
            } else {
                "…".into()
            }
        );
    }
    let from_block = match (args.from_block, args.from) {
        (Some(b), _) => Some(b),
        (None, Some(t)) => Some(client.block_at(t).await?),
        (None, None) => None,
    };
    let to_block = match (args.to_block, args.to) {
        (Some(b), _) => Some(b),
        // Last block at or before `t`.
        (None, Some(t)) => Some(client.block_at(t + 1).await?.saturating_sub(1)),
        (None, None) => None,
    };
    if args.from.is_some() || args.to.is_some() {
        eprintln!(
            "→ blocks {} ..= {}",
            from_block.map_or("…".into(), |b| b.to_string()),
            to_block.map_or("latest".into(), |b| b.to_string())
        );
    }
    let max_blocks = match args.since {
        Some(since) => client.blocks_in(since).await?,
        None => args.blocks,
    };

    let mut scan = client
        .scan_logs(transfer.clone().topic(1, me))
        .or_filter(transfer.topic(2, me))
        .limit(args.limit)
        .max_blocks(max_blocks)
        .chunk(args.chunk)
        .rate_limit(rps);
    scan = if args.oldest_first {
        scan.oldest_first()
    } else {
        scan.newest_first()
    };
    if let Some(b) = from_block {
        scan = scan.from_block(b);
    }
    if let Some(b) = to_block {
        scan = scan.to_block(b);
    }

    let plan = scan.plan().await?;
    if plan.max_requests > CONFIRM_ABOVE && !args.yes && !confirm(&plan, rps) {
        return Ok(());
    }

    let interactive = std::io::stderr().is_terminal();
    let logs = scan
        .on_progress(move |p| {
            if interactive {
                eprint!(
                    "\rscanning… {:>3}%  {} / {} blocks  {} found",
                    p.scanned * 100 / p.total.max(1),
                    p.scanned,
                    p.total,
                    p.found
                );
            }
        })
        .run()
        .await;
    if interactive {
        eprint!("\r\x1b[2K");
    }
    let logs = match logs {
        Ok(logs) => logs,
        Err(e) => {
            eprintln!("could not fetch transfers: {e}");
            return Ok(());
        }
    };

    println!(
        "\ntransfers in blocks {}..={} ({}): {}",
        plan.from,
        plan.to,
        if args.oldest_first {
            "oldest first"
        } else {
            "newest first"
        },
        logs.len()
    );
    if logs.is_empty() {
        println!(
            "none found; look further back, e.g. --blocks {} or --since 7d",
            plan.blocks.saturating_mul(4)
        );
        return Ok(());
    }
    let mut tokens: HashMap<Address, (String, u8)> = HashMap::new();
    let mut times: HashMap<u64, u64> = HashMap::new();
    for log in &logs {
        if let Entry::Vacant(slot) = tokens.entry(log.address) {
            slot.insert(token_meta(&client, log.address).await);
        }
        let block: u64 = log.block_number.map(|b| b.to()).unwrap_or_default();
        if let Entry::Vacant(slot) = times.entry(block) {
            let ts = client
                .get_block(BlockId::Number(block), false)
                .await
                .ok()
                .flatten()
                .map(|b| b.timestamp.to::<u64>())
                .unwrap_or_default();
            slot.insert(ts);
        }
        let (symbol, decimals) = &tokens[&log.address];
        print_transfer(log, args.address, symbol, *decimals, times[&block]);
    }
    // The last block covered, so a later run can continue with --from-block.
    if args.oldest_first && logs.len() >= args.limit {
        let last: u64 = logs
            .last()
            .and_then(|l| l.block_number)
            .map(|b| b.to())
            .unwrap_or(plan.from);
        println!(
            "\nmore may follow: continue with --from-block {} --oldest-first",
            last + 1
        );
    }
    Ok(())
}

/// Ask on a terminal; refuse (instead of hanging) when there is nobody to ask.
fn confirm(plan: &evmify::ScanPlan, rps: f64) -> bool {
    let time = match plan.min_duration {
        Some(d) if rps > 0.0 => format!(", at least {} at {rps} req/s", human(d)),
        _ => String::new(),
    };
    let question = format!(
        "This scan covers {} blocks and may take up to {} requests{time}.",
        plan.blocks, plan.max_requests
    );
    if !std::io::stdin().is_terminal() {
        eprintln!(
            "{question}\nNot a terminal: re-run with --yes to proceed, or narrow it with --blocks / --since / --from."
        );
        return false;
    }
    eprint!("{question} Continue? [y/N] ");
    std::io::stderr().flush().ok();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer).ok();
    matches!(answer.trim(), "y" | "Y" | "yes" | "Yes")
}

fn print_transfer(log: &Log, me: Address, symbol: &str, decimals: u8, timestamp: u64) {
    let (Some(from), Some(to)) = (log.topics.get(1), log.topics.get(2)) else {
        return;
    };
    let (from, to) = (Address::from_word(*from), Address::from_word(*to));
    let (dir, other) = if from == me {
        ("OUT", to)
    } else {
        ("IN ", from)
    };
    // ERC-721 indexes the token id as topic 3; ERC-20 puts the amount in data.
    let amount = match log.topics.get(3) {
        Some(id) => format!("NFT #{}", U256::from_be_bytes(id.0)),
        None if log.data.len() >= 32 => {
            format_units(U256::from_be_slice(&log.data[..32]), decimals)
        }
        None => "?".into(),
    };
    let block: u64 = log.block_number.map(|b| b.to()).unwrap_or_default();
    let tx = log
        .transaction_hash
        .map(|h| h.to_string())
        .unwrap_or_default();
    println!(
        "  {}  block {block:<10} {dir} {amount:>22} {symbol:<8} {} {other}   tx {}…",
        format_utc(timestamp),
        if dir == "OUT" { "to  " } else { "from" },
        &tx[..tx.len().min(12)]
    );
}

/// `symbol()` and `decimals()` via `eth_call`, falling back to a short address.
async fn token_meta<T: Transport>(client: &EvmClient<T>, token: Address) -> (String, u8) {
    let call = |selector: [u8; 4]| CallRequest::new(token, selector.to_vec());
    let symbol = client
        .call(&call(SYMBOL), BlockId::Latest)
        .await
        .ok()
        .and_then(|b| decode_string(&b))
        .unwrap_or_else(|| {
            let s = token.to_string();
            format!("{}…", &s[..8])
        });
    let decimals = client
        .call(&call(DECIMALS), BlockId::Latest)
        .await
        .ok()
        .filter(|b| b.len() >= 32)
        .map(|b| b[31])
        .unwrap_or(0);
    (symbol, decimals)
}

/// `30m`, `24h`, `7d`, `90s`, `2w`.
fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    let split = s
        .find(|c: char| !c.is_ascii_digit())
        .ok_or("add a unit: s, m, h, d or w (e.g. 24h)")?;
    let (n, unit) = s.split_at(split);
    let n: u64 = n
        .parse()
        .map_err(|_| format!("`{s}` does not start with a number"))?;
    let secs = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        "w" => 604_800,
        other => return Err(format!("unknown unit `{other}`: use s, m, h, d or w")),
    };
    Ok(Duration::from_secs(n * secs))
}

/// `2026-10-01`, `2026-10-01T12:00`, `2026-10-01T12:00:00Z` or `2026-10-01 12:00:00` (UTC)
/// -> Unix seconds.
fn parse_datetime(s: &str) -> Result<u64, String> {
    let bad = || format!("`{s}`: use YYYY-MM-DD or YYYY-MM-DDTHH:MM[:SS] (UTC)");
    let s = s.trim().trim_end_matches('Z');
    let (date, time) = match s.split_once(['T', ' ']) {
        Some((d, t)) => (d, t),
        None => (s, "00:00:00"),
    };
    let d: Vec<i64> = date
        .split('-')
        .map(|p| p.parse().map_err(|_| bad()))
        .collect::<Result<_, _>>()?;
    let t: Vec<i64> = time
        .split(':')
        .map(|p| p.parse().map_err(|_| bad()))
        .collect::<Result<_, _>>()?;
    let ([y, m, day], [hh, mm, ..]) = (d.as_slice(), t.as_slice()) else {
        return Err(bad());
    };
    let ss = t.get(2).copied().unwrap_or(0);
    if !(1..=12).contains(m) || !(1..=31).contains(day) || *hh > 23 || *mm > 59 || ss > 60 {
        return Err(bad());
    }
    let secs = days_from_civil(*y, *m, *day) * 86_400 + hh * 3_600 + mm * 60 + ss;
    u64::try_from(secs).map_err(|_| format!("`{s}` is before 1970"))
}

/// Unix seconds -> `2026-10-01 12:00 UTC`.
fn format_utc(secs: u64) -> String {
    if secs == 0 {
        return "????-??-?? ??:??    ".into();
    }
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        rem / 3_600,
        rem % 3_600 / 60
    )
}

// Days since 1970-01-01 <-> proleptic Gregorian date (Howard Hinnant's algorithms).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn human(d: Duration) -> String {
    match d.as_secs() {
        s if s < 90 => format!("{s} s"),
        s if s < 5_400 => format!("{} min", s.div_ceil(60)),
        s => format!("{:.1} h", s as f64 / 3_600.0),
    }
}

/// ABI-decode a `string` return value (or a legacy `bytes32` one).
fn decode_string(b: &Bytes) -> Option<String> {
    let raw: &[u8] = if b.len() >= 64 {
        let offset = usize::try_from(U256::from_be_slice(&b[..32])).ok()?;
        let len = usize::try_from(U256::from_be_slice(b.get(offset..offset + 32)?)).ok()?;
        b.get(offset + 32..offset + 32 + len)?
    } else if b.len() == 32 {
        let end = b.iter().position(|&c| c == 0).unwrap_or(32);
        &b[..end]
    } else {
        return None;
    };
    let s: String = String::from_utf8_lossy(raw)
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    (!s.is_empty()).then_some(s)
}

/// `1234500000000000000` with 18 decimals -> `1.2345`.
fn format_units(value: U256, decimals: u8) -> String {
    if decimals == 0 {
        return value.to_string();
    }
    let unit = U256::from(10u64).pow(U256::from(decimals));
    let whole = value / unit;
    let frac = format!(
        "{:0>width$}",
        (value % unit).to_string(),
        width = decimals as usize
    );
    let frac = frac[..frac.len().min(6)].trim_end_matches('0');
    if frac.is_empty() {
        whole.to_string()
    } else {
        format!("{whole}.{frac}")
    }
}
