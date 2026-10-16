# Contributing

Thanks for helping! Issues and PRs are welcome.

## Before you open a PR

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo clippy --all-targets --no-default-features -- -D warnings
cargo test
```

CI runs the same commands.

## Adding a non-EVM chain

Use `src/adapters/solana.rs` or `src/adapters/bitcoin.rs` as a template. A new chain is one file with four parts:

1. **Raw types** — `#[derive(Deserialize)]` structs for the node's responses. Model only the fields you use; serde ignores the rest, which keeps you robust to API additions.
2. **Adapter** — implement `pipeline::Adapter`: `chain()`, `head()`, `block(height)`, `transaction(native_id)`, and optionally `finalized_head()`. Return `Ok(None)` (not an error) for heights/ids that do not exist; map the node's "not found" error codes to `None`.
3. **Normalizer** — implement `pipeline::Normalizer`: raw → `NormalizedBlock` / `NormalizedTx`. Keep it pure (no I/O). Rules of thumb:
   - amounts in the chain's base unit, as `u128`
   - identifiers stay native strings; the transformer maps them
   - use `NormalizedEvent::transfer` / `balance_change` so the output looks like ERC-20 logs
   - put anything else useful in `extra` (it ends up in `_native.extra`)
4. **`pipeline()` helper** — wire adapter + normalizer + `EvmTransformer` with the right `IdMapper` (`DefaultMapper::new(true)` if native ids are 32-byte hex, otherwise wrap in `IndexedMapper`).

Then add `ChainInfo::<chain>_mainnet()` in `src/chains.rs`, register the module in `src/adapters/mod.rs`, and write tests in `tests/<chain>.rs` using `MockTransport` with realistic JSON fixtures (copy real responses and trim them). Document the field mapping in the module docs and the README table.

## Adding an EVM chain

Nothing to do: `EvmClient` works with any EVM endpoint. To give it a name and symbol, add a line to `KNOWN_EVM_CHAINS` in `src/chains.rs`:

```rust
// (chain id, key, display name, native symbol)
(2020, "ronin", "Ronin", "RON"),
```

The number is the chain's **EIP-155 chain ID**: a global identifier, not something local to this crate. It is the value the node returns for `eth_chainId`, which is how `EvmClient::connect` recognizes the chain. Look it up on [chainlist.org](https://chainlist.org) (backed by [ethereum-lists/chains](https://github.com/ethereum-lists/chains)), and double-check it against the chain's own docs or by calling `eth_chainId` on its official RPC. Use a short lowercase `key` (`-testnet` / `-sepolia` suffix for test networks) and the native coin's symbol.

Chains missing from the list still work; they are just named `EVM chain <id>`.
