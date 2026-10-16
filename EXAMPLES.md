# Examples

Copy-paste commands for trying evmify against real networks. All examples live in
[`examples/`](examples/) and run with `cargo run --example <name> -- <args>`.

Wherever an example takes a chain, you can pass a **shorthand key** (`ronin`,
`ethereum`, `base`, `solana`, …) or any **RPC URL**. Keys use the chain's public RPC,
unless `EVMIFY_RPC_<KEY>` points them somewhere else.

Public RPC endpoints are free but rate limited, and many cap how many blocks one
`eth_getLogs` call may cover. For anything beyond experiments, use your own node or a
provider (Alchemy, QuickNode, Chainstack, dRPC, …).

## List the built-in chains: `list_chains`

Every shorthand key with its chain id, name, symbol and the RPC it resolves to.

```sh
cargo run --example list_chains

# see an override take effect
EVMIFY_RPC_RONIN=https://my-node.example cargo run --example list_chains
```

## Inspect an address on any EVM chain: `address_info`

Native balance, number of sent transactions and wallet or contract. With `--transfers`,
also its latest ERC-20 / ERC-721 transfers, **newest first**, with token symbols looked
up on-chain.

```sh
# basic info only (4 quick calls)
cargo run --example address_info -- ronin 0xYourRoninAddress

# + the latest 20 transfers, searching back at most 10 000 blocks
cargo run --example address_info -- ronin 0xYourRoninAddress --transfers

# search by time instead of blocks (block time is measured on the chain)
cargo run --example address_info -- ronin 0xYourRoninAddress --transfers --since 24h

# more results, further back, faster own node, no questions
EVMIFY_RPC_RONIN=https://my-node.example cargo run --example address_info -- \
  ronin 0xYourRoninAddress --transfers --limit 100 --blocks 200000 --rps 50 --yes

# an exact period (UTC dates or times; resolved to exact blocks)
cargo run --example address_info -- ronin 0xYourRoninAddress --transfers --from 2026-10-01 --to 2026-10-08

# walk a period forwards, 50 at a time, and resume where it stopped
cargo run --example address_info -- ronin 0xYourRoninAddress --transfers --from 2026-10-01 --oldest-first --limit 50
cargo run --example address_info -- ronin 0xYourRoninAddress --transfers --from-block 62300000 --oldest-first --limit 50

# all options
cargo run --example address_info -- --help
```

| Option | Default | Meaning |
|---|---|---|
| `-t, --transfers` | off | also list recent token transfers |
| `--limit N` | 20 | stop after N transfers |
| `--blocks N` | 10 000 | look back at most N blocks |
| `--since 24h` | | look back by time instead (`s`, `m`, `h`, `d`, `w`); estimated from the measured block time |
| `--from 2026-10-01` / `--to …` | now | exact period in UTC (`YYYY-MM-DD` or `YYYY-MM-DDTHH:MM[:SS]`), resolved to blocks by binary search on block timestamps |
| `--from-block N` / `--to-block N` | latest | exact block range, e.g. to resume a scan |
| `--oldest-first` | | walk forwards from the start; prints the `--from-block` to continue from |
| `--chunk N` | 200 | blocks per `eth_getLogs` call; halved automatically if the RPC refuses |
| `--rps N` | 5 on public RPCs, else unlimited | max requests per second (`0` = unlimited) |
| `-y, --yes` | | don't ask before long scans |

The search walks **backwards from the newest block** and stops as soon as it has
`--limit` transfers, so active addresses finish in a few calls. Scans that may need more
than 100 requests show an estimate and ask before starting; in scripts (no terminal) they
stop instead of hanging, unless you pass `--yes`.

Example output:

```text
chain     Ronin (chain id 2020)
rpc       https://api.roninchain.com/rpc (Public)
address   0x…
kind      wallet (EOA)
balance   12.5 RON
sent      341 transactions

transfers in blocks 48113511..=48123510 (newest first): 2
  2026-10-16 18:40 UTC  block 48123510   OUT              NFT #1234 AXIE     to   0x…   tx 0x1b7e9a33…
  2026-10-16 18:38 UTC  block 48123456   IN                  250 SLP      from 0x…   tx 0x8f2c41d0…
```

> Ronin addresses are sometimes shown as `ronin:abc…`. Replace `ronin:` with `0x`; it is
> the same address.

Public RPCs are rate limited. evmify waits and retries on HTTP 429, but a large scan can
still be refused; for those, point the key at your own node or a provider with
`EVMIFY_RPC_<KEY>`.

## Latest block from several chains: `multi_chain`

Ethereum, Base, Ronin, Arbitrum and Solana through one `ChainProvider` trait, printed
as EVM JSON.

```sh
cargo run --example multi_chain

# also include Bitcoin (needs your own Bitcoin Core 25+ node)
EVMIFY_RPC_BITCOIN=http://user:pass@127.0.0.1:8332 cargo run --example multi_chain
```

## A Solana transaction as an EVM receipt: `solana_tx`

Look up a transaction by its signature (from Solscan, a wallet, …). SOL and SPL token
movements come back as ERC-20 style `Transfer` / `BalanceChange` logs, and the original
signature, instructions and program logs are kept under `_native`.

```sh
cargo run --example solana_tx -- <SIGNATURE>

# devnet, or your own RPC by URL or by override
cargo run --example solana_tx -- <SIGNATURE> solana-devnet
cargo run --example solana_tx -- <SIGNATURE> https://your-solana-rpc.example
EVMIFY_RPC_SOLANA=https://your-solana-rpc.example cargo run --example solana_tx -- <SIGNATURE>
```

The public Solana RPC only keeps recent history; older transactions need an archive
provider.

## Raw JSON-RPC from the shell

Handy for checking what a node returns before evmify transforms it:

```sh
# Which chain is this endpoint? (0x7e4 = 2020 = Ronin)
curl -s -X POST https://api.roninchain.com/rpc \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}'

# Balance of an address, in wei (hex)
curl -s -X POST https://api.roninchain.com/rpc \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"eth_getBalance","params":["0xYourAddress","latest"]}'
```
