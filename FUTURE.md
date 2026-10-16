# Future

Where evmify could go next, and why. Nothing here is promised or scheduled; it is a
place to collect ideas, the reasoning behind them, and what they would take. Want to
work on one? Open an issue or a discussion first so we can agree on the shape.

## Full address history: scanning vs. indexing

`EvmClient::scan_logs` answers "what happened recently?" well: the last hours, days or
weeks of an address. It cannot answer "everything this address ever did" on a
long-lived chain in reasonable time, and that is a property of `eth_getLogs`, not a bug:

- Ronin makes a block every ~2 seconds, ~15.8 million a year. At 200 blocks per call and
  two queries per window (transfers in and out) a full year is ~158 000 calls, about
  9 hours at 5 requests per second, and public RPCs refuse long before that.
- Gaps of inactivity don't help: the node can only tell a block is empty by looking at
  it (each block's bloom filter lets it skip quickly, but still block by block).

**How block explorers are instant.** They do the work once, ahead of time:

1. They run their own (archive) nodes, so no public rate limits apply.
2. They read every block exactly once, as it is produced, and decode every transaction
   and log: who sent what to whom, which token, how much.
3. They write the result to a database (typically PostgreSQL) **indexed by address**.
4. Looking up an address is then an index lookup, milliseconds whether the history is
   one year or ten.

The cost moves from read time (per query) to write time (once per block, for all
addresses), paid with servers, disk and 24/7 operation. Open-source examples:
[Blockscout](https://www.blockscout.com/) (a full explorer) and indexing frameworks such
as [The Graph](https://thegraph.com/), [Ponder](https://ponder.sh/),
[Envio](https://envio.dev/) and [Subsquid](https://www.sqd.dev/).

That points to three directions for evmify, from small to large:

### 1. Indexer adapters (full history from an existing index)

An adapter that asks an explorer or indexer API (the chain's own explorer API, Moralis,
Covalent/GoldRush, …) for an address's full history and returns it as evmify `Log`s /
`Transaction`s. One call instead of 100 000, in the same EVM shape as everything else.
Needs an API key per provider; fits the existing adapter → normalizer → transformer
pipeline.

### 2. Finding outgoing activity by nonce

An account's nonce only grows when it sends a transaction. Binary-searching
`eth_getTransactionCount(address, block)` over block numbers finds the exact block of
each sent transaction in ~25 calls, instead of scanning every block. Limits: needs an
**archive node** (historical state), and only finds transactions the address *sent*, not
tokens it received.

### 3. evmify-indexer (your own mini explorer)

The pipeline already reads blocks from many chains and produces one common format, which
is the first half of an indexer. The plan is a **separate crate in the same repository**
(a Cargo workspace, like tokio or alloy), so `evmify` itself stays light and stateless:

```text
evmify/                  the repository
├── Cargo.toml           [workspace]
├── src/ …               evmify: the library, unchanged
└── evmify-indexer/
    ├── src/lib.rs       the indexing stream + optional sinks
    └── src/main.rs      CLI: evmify-indexer run ronin --db ronin.sqlite
```

**The core is a stream, not a database.** The indexer follows a chain and yields events;
the host decides where they go (its own Postgres, Kafka, files, …):

```rust
let mut stream = Indexer::new(client).from_block(62_000_000).follow();

while let Some(event) = stream.next().await {
    match event? {
        IndexEvent::Block { block, transfers } => db.insert(block, transfers).await?,
        IndexEvent::Reorg { from_block } => db.delete_from(from_block).await?,
    }
}
```

What the stream takes care of, because hosts most often get these wrong:

- **Reorgs.** When the chain replaces its latest blocks, the stream emits
  `Reorg { from_block }` so the host can delete what is no longer canonical.
- **Resuming.** The host stores "last processed block" in the same database transaction
  as the data, and passes it to `from_block` on restart: no gaps and no duplicates, even
  after a crash.
- **Backfill and live** in one API: catch up on history in parallel windows, then follow
  new blocks.

**Optional ready-made sinks** behind features, for those who want one command instead of
writing the storage themselves:

- `sqlite` (via `rusqlite` with `bundled`, so SQLite is compiled in and the result is one
  file): indexes on address, one transaction per block, reorg = one `DELETE … WHERE
  block >= N`. The default for the CLI.
- `postgres` for larger installations.
- A plain JSON Lines / CSV export for raw data. Files alone are easy to write but slow to
  query by address, which is the problem an index solves, so they are an export format,
  not the store.

With that, "all transfers of address X across Ronin, Ethereum, Solana and Bitcoin" is
one query against one table, in the same EVM shape evmify uses everywhere.

## Serve non-EVM chains as a JSON-RPC server

Run the pipeline behind an `eth_*` JSON-RPC endpoint, so existing EVM tooling (viem,
ethers, wallets, indexers) can read Solana or Bitcoin with **no code changes**: point it
at `http://localhost:8545` and call `eth_getBlockByNumber`. Probably the most visible
feature evmify could have. Building blocks: `ChainProvider` already exposes blocks,
transactions, receipts and logs in EVM shape; it needs an HTTP server (e.g. axum), method
routing and chain-id assignment for non-EVM chains.

## More chains

- **NEO N3.** A great fit: addresses are 20-byte script hashes (map 1:1 to EVM addresses,
  reversibly), tx and block hashes are 32 bytes, and NEP-17 tokens emit
  `Transfer(from, to, amount)` notifications that become real ERC-20 `Transfer` logs.
  RPC: `getblockcount`, `getblock`, `getrawtransaction`, `getapplicationlog`.
- **Ontology.** Similar to NEO (20-byte addresses, `getsmartcodeevent`); it also has an EVM
  layer, so check what `EvmClient` can already read before designing the adapter.
- **Tron, Sui, Aptos, Cosmos SDK, TON, Near**: open a
  [chain request](https://github.com/madspaaskesen/evmify/issues/new?template=chain-request.yml)
  or 👍 an existing one.
- **Cargo features per chain** (`solana`, `bitcoin`, `neo`, …) once there are more adapters,
  so users only compile what they use.

## Smaller improvements

- **Skip Solana vote transactions** (`SolanaAdapter::skip_vote_transactions`): validator votes
  are most of every block and noise for most users.
- **Balances for non-EVM chains**: `getBalance` on Solana; for Bitcoin this needs an address
  index (Esplora / Electrum), since Bitcoin Core has none.
- **WebSocket transport** and `eth_subscribe("newHeads" | "logs")` for following chains live.
- **Batched JSON-RPC** (several calls in one HTTP request) to cut round trips in scans.
- **A caching `Transport` wrapper** for hosts that want it, kept outside the core so evmify
  stays stateless.
- **A chain configurator page**: pick chains in a browser and get the `Cargo.toml`
  features, `EVMIFY_RPC_*` variables or `docker-compose.yml` to copy.

## Non-goals (for now)

- **Signing and sending** non-EVM transactions. evmify is read-side;
  `send_raw_transaction` only broadcasts already-signed EVM transactions.
- **Storing data inside the library.** Caching and persistence belong to the host
  application or a separate indexer crate.
