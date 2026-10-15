# evmify

**EVM-ify any chain.** One read API for EVM **and** non-EVM chains, always answered in EVM JSON-RPC shape: query Solana or Bitcoin and get back the same blocks, transactions, receipts and logs your Ethereum code already understands.

- **`EvmClient`** — a typed `eth_*` client that works against any EVM chain (Ethereum, OP-stack, Arbitrum, Polygon, BNB, Avalanche, zkSync, …). Fields the crate does not model (`l1BlockNumber`, deposit fields, …) are kept in `other`, never dropped.
- **A modular pipeline** — `Adapter → Normalizer → Transformer` — that serves non-EVM chains as EVM blocks, transactions, receipts and logs. Solana and Bitcoin ship as reference adapters.
- **`ChainProvider`** — the trait both implement, so `Vec<Box<dyn ChainProvider>>` can hold Ethereum, Base, Solana and Bitcoin side by side.

```rust
use evmify::{BlockId, ChainProvider, EvmClient, HttpTransport, adapters::solana};

let chains: Vec<Box<dyn ChainProvider>> = vec![
    Box::new(EvmClient::connect("https://ethereum-rpc.publicnode.com").await?),
    Box::new(solana::pipeline(solana::SolanaAdapter::mainnet(
        HttpTransport::new("https://api.mainnet-beta.solana.com"),
    ))),
];

for chain in &chains {
    let block = chain.block(BlockId::Latest, true).await?.unwrap();
    // Same struct, same JSON, whichever chain it came from.
    println!("{} #{} {}", chain.chain().name, block.number, serde_json::to_string(&block)?);
}
```

Run the full example: `cargo run --example multi_chain`.

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

- `HttpTransport` (feature `http`, on by default) — reqwest + rustls, bearer or basic auth.
- `FallbackTransport` — several endpoints, moves on only on network failures (an RPC error like `execution reverted` is returned straight away).
- `MockTransport` — canned responses + call log, for your own tests.

Build with `default-features = false` to bring your own HTTP stack.

## Request a chain

Missing a chain? [Open a chain request](https://github.com/madspaaskesen/evmify/issues/new?template=chain-request.yml), or 👍 an [existing one](https://github.com/madspaaskesen/evmify/issues?q=is%3Aissue+label%3Achain-request). The most-wanted chains are built first. Questions and ideas go in [Discussions](https://github.com/madspaaskesen/evmify/discussions).

## Adding a chain

See [CONTRIBUTING.md](CONTRIBUTING.md). In short: raw serde structs, an `Adapter` that fetches them, a `Normalizer` that maps them, a `pipeline()` helper, and fixture tests. The EVM output comes for free.

## Scope

Read-side only for now. `EvmClient::send_raw_transaction` broadcasts pre-signed EVM transactions; signing and non-EVM writes are out of scope.

## Roadmap ideas

- More adapters: Tron, Cosmos SDK, Sui, Aptos, TON, Near
- Serving the pipeline *as* a JSON-RPC server (point ethers/viem at Solana)
- WebSocket transport + `eth_subscribe("newHeads")`
- Batched JSON-RPC requests, retry/backoff and rate-limit middleware

## License

Dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
