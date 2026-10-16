# Changelog

All notable changes to evmify. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [SemVer](https://semver.org/) (in `0.x`, a minor bump may break the API).

## [0.2.0]

### Added
- **Chain shorthands.** `EvmClient::connect("ronin")`, `solana::connect("solana")` and `bitcoin::connect("bitcoin")` accept a key from `chains::KNOWN_CHAINS` or any RPC URL.
- **RPC overrides** per key via `EVMIFY_RPC_<KEY>` (e.g. `EVMIFY_RPC_RONIN`), resolved by `chains::resolve_rpc`.
- Built-in **public RPCs** for chains whose own docs publish one (Ethereum, OP, BNB, Gnosis, Polygon, zkSync, Ronin, Base, Arbitrum, Celo, Avalanche, Linea, Scroll, Sepolia, Base Sepolia, Ronin Saigon, Solana, Solana Devnet).
- `EvmClient::detect_expecting` and `Error::ChainMismatch`: connecting by key verifies the node's `eth_chainId`.
- `EvmClient::scan_logs` → `LogScan` builder: newest-first or oldest-first windows, `limit`, `max_blocks`, `or_filter`, `rate_limit` (requests/s), `on_progress`, and `plan()` to estimate calls before running.
- `EvmClient::block_at(unix_secs)`: the exact first block at or after a moment, by binary search on block timestamps (~26 calls on a 60M-block chain).
- `MockTransport::on_fn`: answer a method from its params, for tests that need different responses per call.
- `EvmClient::blocks_in(duration)`: block count for a time span, from the chain's measured block time.
- `EvmClient::get_logs_chunked`: `eth_getLogs` over long block ranges, split into windows and halved automatically when a provider rejects the range (Ronin allows 200 blocks).
- `HttpTransport::max_rps(n)`: a request budget shared by every call through the transport (also concurrent ones), so a client stays under a provider's limit instead of running into 429s.
- `HttpTransport` retries HTTP 429 / 503 ("rate limit exceeded", "overloaded"), honouring `Retry-After` or backing off 0.5 s, 1 s, 2 s, … (`.max_retries(n)`, default 3).
- `bitcoin::connect` reads credentials from the URL (`http://user:pass@host:8332`).
- Examples: `list_chains`, `address_info` (balance, nonce and, with `--transfers`, the latest ERC-20/721 transfers newest first; `--since 24h`, `--from`/`--to` dates, `--from-block`/`--to-block` to resume, `--oldest-first`, `--limit`, `--rps`, confirmation before long scans, and the date of every transfer), `solana_tx` (a Solana transaction as an EVM receipt). Copy-paste commands in `EXAMPLES.md`.
- Ronin (2020) and Ronin Saigon (202601) in the chain registry.

### Changed
- **Breaking:** `chains::KNOWN_EVM_CHAINS` (tuples) is replaced by `chains::KNOWN_CHAINS` (`KnownChain` structs, including Solana and Bitcoin).
- **Breaking:** chain keys for non-EVM networks are now `solana`, `solana-devnet`, `bitcoin`, `bitcoin-testnet`; `_native.chain` uses them (was `solana-mainnet`).
- **Breaking:** `EvmClient::connect` takes `impl AsRef<str>` (a URL or a key).
- `Error` is `#[non_exhaustive]`, and RPC errors include the node's `data` field in their message (often the actual reason, e.g. a block-range limit).

## [0.1.2]

### Fixed
- Solana: request `maxSupportedTransactionVersion: 1`, so blocks containing the new v1 transactions no longer fail with `-32015`. Transaction version and v1 `transactionConfig` are kept in `_native.extra`.

## [0.1.1] - yanked

Published from the wrong commit (version bump only). Use 0.1.2.

## [0.1.0]

- Initial release: `EvmClient`, the adapter → normalizer → transformer pipeline, Solana and Bitcoin adapters, HTTP / fallback / mock transports.
