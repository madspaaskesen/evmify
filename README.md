# evmify

[![CI](https://github.com/madspaaskesen/evmify/actions/workflows/ci.yml/badge.svg)](https://github.com/madspaaskesen/evmify/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/evmify.svg)](https://crates.io/crates/evmify)
[![docs.rs](https://docs.rs/evmify/badge.svg)](https://docs.rs/evmify)
[![downloads](https://img.shields.io/crates/d/evmify.svg)](https://crates.io/crates/evmify)
[![license](https://img.shields.io/crates/l/evmify.svg)](#license)
[![MSRV](https://img.shields.io/badge/MSRV-1.90-blue.svg)](https://blog.rust-lang.org/)

**[Docs](https://docs.rs/evmify) · [Examples](EXAMPLES.md) · [Project page](https://mp-it.dk/projects/evmify) · [Roadmap](FUTURE.md) · [Changelog](CHANGELOG.md)**

**EVM-ify any chain.** One read API for EVM **and** non-EVM chains, always answered in EVM JSON-RPC shape: query Solana or Bitcoin and get back the same blocks, transactions, receipts and logs your Ethereum code already understands.

- **`EvmClient`** — a typed `eth_*` client that works against any EVM chain (Ethereum, OP-stack, Arbitrum, Polygon, BNB, Avalanche, zkSync, …). Fields the crate does not model (`l1BlockNumber`, deposit fields, …) are kept in `other`, never dropped.
- **A modular pipeline** — `Adapter → Normalizer → Transformer` — that serves non-EVM chains as EVM blocks, transactions, receipts and logs. Solana and Bitcoin ship as reference adapters.
- **`ChainProvider`** — the trait both implement, so `Vec<Box<dyn ChainProvider>>` can hold Ethereum, Base, Solana and Bitcoin side by side.

```rust
use evmify::{BlockId, ChainProvider, EvmClient, adapters::solana};

let chains: Vec<Box<dyn ChainProvider>> = vec![
    Box::new(EvmClient::connect("ethereum").await?),
    Box::new(EvmClient::connect("ronin").await?),
    Box::new(solana::connect("solana")?),
];

for chain in &chains {
    let block = chain.block(BlockId::Latest, true).await?.unwrap();
    // Same struct, same JSON, whichever chain it came from.
    println!("{} #{} {}", chain.chain().name, block.number, serde_json::to_string(&block)?);
}
```

Run the full example: `cargo run --example multi_chain`, and see every built-in chain with `cargo run --example list_chains`. More copy-paste commands (inspect any address on Ronin, Ethereum, Base…, look up a Solana transaction) are in [EXAMPLES.md](EXAMPLES.md).

## Chain shorthands and RPC endpoints

Everywhere an RPC URL is accepted you can pass a **shorthand key** instead: `ethereum`, `base`, `arbitrum`, `ronin`, `polygon`, `solana`, … (`cargo run --example list_chains` prints them all). A key resolves in this order:

1. **A URL wins.** Anything containing `://` is used as is: `EvmClient::connect("https://my-node.example:8545")`.
2. **`EVMIFY_RPC_<KEY>`**, if set: `EVMIFY_RPC_RONIN=https://my-node.example evmify-app` points `ronin` at your own node without code changes (upper case, `-` becomes `_`, e.g. `EVMIFY_RPC_BASE_SEPOLIA`).
3. **The chain's public RPC**, taken from the chain's own documentation.

For a key, `EvmClient::connect` also checks the node's `eth_chainId`, so an override that points at the wrong network fails with `Error::ChainMismatch` instead of returning the wrong chain's data. `chains::resolve_rpc` exposes the same resolution if you want the URL itself.

Public endpoints are free but **rate limited and meant for trying things out**. Use your own node or a provider (Alchemy, QuickNode, Chainstack, dRPC, …) in production. Bitcoin has no public JSON-RPC, so `bitcoin` needs `EVMIFY_RPC_BITCOIN=http://user:pass@host:8332`.

## Scanning logs over long ranges

Providers cap how many blocks one `eth_getLogs` call may cover (Ronin: 200) and how many calls you may make per second. `EvmClient::scan_logs` handles both:

```rust
let transfers = Filter::new().event("Transfer(address,address,uint256)");
let latest = client
    .scan_logs(transfers.clone().topic(1, me))  // from me …
    .or_filter(transfers.topic(2, me))          // … or to me
    .newest_first()                              // walk backwards from the head
    .limit(20)                                   // stop once 20 are found
    .max_blocks(10_000)
    .rate_limit(5.0)                             // requests per second
    .on_progress(|p| eprint!("\r{}/{} blocks", p.scanned, p.total))
    .run()
    .await?;
```

Windows that the node rejects as too large are halved and retried, and `scan.plan()` estimates the number of calls up front (e.g. to ask the user). `client.blocks_in(Duration::from_secs(86_400))` turns "the last 24 h" into a block count using the chain's measured block time, and `client.block_at(unix_secs)` finds the exact block for a date by binary search on block timestamps. evmify keeps no state between calls: caching and storing results is up to your application.

## The pipeline

```text
 native RPC ──► Adapter ──► RawBlock/RawTx ──► Normalizer ──► NormalizedBlock/Tx ──► Transformer ──► EVM Block/Tx/Receipt/Log
               (fetch)                         (understand)                        (present)
```

| Stage | Trait | Job | Swap it when… |
|---|---|---|---|
| Adapter | `Adapter` | Talk to the native node, return raw serde structs | you use a different provider/API (Helius, Esplora, …) |
| Normalizer | `Normalizer` | Raw payload → chain-agnostic `NormalizedBlock` / `NormalizedTx` / `NormalizedEvent` | you want to decode programs/instructions differently |
| Transformer | `Transformer` | Normalized → output format (`EvmTransformer` by default) | you want a different output (your own schema, Parquet rows, …) |

Normalizers are pure and synchronous, so they are tested against JSON fixtures with no network. Every stage is generic, so swapping one never touches the others:

```rust
let btc = bitcoin::pipeline(bitcoin::BitcoinAdapter::mainnet(transport))
    .with_transformer(EvmTransformer::new(DefaultMapper::new(true)).with_scale(ValueScale::Native));
```

## How non-EVM data maps onto EVM

| EVM field | Solana | Bitcoin |
|---|---|---|
| `block.number` | slot (skipped slots → `None`) | height |
| `block.hash` | `keccak256(blockhash)` | block hash (unchanged) |
| `block.miner` | leader (fee reward recipient) | first coinbase output address |
| `tx.hash` | `keccak256(signature)` | txid (unchanged, reversible) |
| `tx.from` | fee payer | first input's address (`0x0` for coinbase) |
| `tx.to` / `value` | recipient + amount for plain SOL transfers, otherwise first invoked program + 0 | first non-change output + total paid excluding change |
| `gasUsed × effectiveGasPrice` | fee | fee |
| `status` | `meta.err == null` | `1` once in a block |
| logs | `BalanceChange(address,int256)` per SOL/SPL balance change, plus ERC-20 `Transfer` when a mint has one sender and one receiver | ERC-20 `Transfer(from, output, sats)` per output |
| `_native` | signature, raw accounts, instructions, compute units, program logs | inputs, outputs, coinbase flag |

Details worth knowing:

- **Addresses.** Non-hex native addresses become `keccak256(utf8(native))[12..]`. The original is always in `_native`. Plug in your own `IdMapper` for a different scheme.
- **Amounts.** By default native amounts are rescaled to 18 decimals (`ValueScale::Decimals(18)`) so EVM tooling formats 1 SOL / 1 BTC like 1 ETH. Token amounts are never rescaled. Use `ValueScale::Native` for lamports/satoshis.
- **Native coin logs** are emitted from `0xEeeeeEeeeEeEeeEeEeEeeEEEeeeeEeeeeeeeEEeE`, the usual native-token placeholder.
- **Hash lookups.** Bitcoin hashes are reversible. Solana signatures hash one-way, so `transaction(hash)` works for anything the pipeline has seen in a block (`IndexedMapper`, last 100k); otherwise use `transaction_by_native_id(signature)`. For an indexer, back `IdMapper` with your database.
- **`logs()`** on non-EVM chains scans blocks one by one and is capped by `max_log_range` (default 50).
- **Bitcoin** needs Core 25+ (prevouts in `getblock` verbosity 3) and `-txindex=1` for tx lookups. Auth: `HttpTransport::new(url).basic_auth(user, Some(pass))`.

## Transports

`Transport` is one async method, so anything can sit behind it.

- `HttpTransport` (feature `http`, on by default) — reqwest + rustls, bearer or basic auth, a shared request budget (`.max_rps(5.0)`), and automatic retries with backoff when a provider answers HTTP 429 / 503 (`.max_retries(n)`).
- `FallbackTransport` — several endpoints, moves on only on network failures (an RPC error like `execution reverted` is returned straight away).
- `MockTransport` — canned responses + call log, for your own tests.

Build with `default-features = false` to bring your own HTTP stack.

## Request a chain

Missing a chain? [Open a chain request](https://github.com/madspaaskesen/evmify/issues/new?template=chain-request.yml), or 👍 an [existing one](https://github.com/madspaaskesen/evmify/issues?q=is%3Aissue+label%3Achain-request). The most-wanted chains are built first. Questions and ideas go in [Discussions](https://github.com/madspaaskesen/evmify/discussions).

## Adding a chain

**EVM chains** already work with `EvmClient` and a URL, with no code needed. To give one a shorthand key, a proper name and (optionally) its public RPC, add one line with the chain's [EIP-155 chain ID](https://chainlist.org) to `KNOWN_CHAINS` in `src/chains.rs`; see [CONTRIBUTING.md](CONTRIBUTING.md#adding-an-evm-chain).

**Non-EVM chains** need an adapter; see [CONTRIBUTING.md](CONTRIBUTING.md). In short: raw serde structs, an `Adapter` that fetches them, a `Normalizer` that maps them, a `pipeline()` helper, and fixture tests. The EVM output comes for free.

## Scope

Read-side only for now. `EvmClient::send_raw_transaction` broadcasts pre-signed EVM transactions; signing and non-EVM writes are out of scope.

## Roadmap ideas

Full address history via indexers, a JSON-RPC server that lets viem/ethers read Solana and Bitcoin unchanged, NEO and more chains, and smaller improvements: see [FUTURE.md](FUTURE.md), including why block explorers are instant while `eth_getLogs` scans are not.

## Author

Built and maintained by [Mads Paaskesen](https://mp-it.dk/projects/evmify), Hardcore Websmith at [MP-IT](https://mp-it.dk). Need a chain adapter, an indexer, payment monitoring or a Rust / Node.js backend around evmify? I take on freelance work: [get in touch](https://mp-it.dk/contact).

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
