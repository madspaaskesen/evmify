//! Chain metadata.

/// What protocol family a chain speaks natively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChainKind {
    Evm,
    Solana,
    Bitcoin,
    /// Anything else you add an adapter for.
    Other(&'static str),
}

/// Static description of a chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainInfo {
    /// Stable key, e.g. `"ethereum"`, `"solana"`.
    pub key: String,
    pub name: String,
    pub kind: ChainKind,
    /// EIP-155 chain id. `None` for non-EVM chains unless you assign one.
    pub evm_chain_id: Option<u64>,
    pub native_symbol: String,
    /// Decimals of the native coin's base unit (18 for ETH, 9 for SOL, 8 for BTC).
    pub native_decimals: u8,
}

impl ChainInfo {
    pub fn new(key: &str, name: &str, kind: ChainKind, symbol: &str, decimals: u8) -> Self {
        Self {
            key: key.into(),
            name: name.into(),
            kind,
            evm_chain_id: None,
            native_symbol: symbol.into(),
            native_decimals: decimals,
        }
    }

    pub fn with_evm_chain_id(mut self, id: u64) -> Self {
        self.evm_chain_id = Some(id);
        self
    }

    /// A generic EVM chain known only by id (used when [`by_id`] has no entry).
    pub fn evm_unknown(chain_id: u64) -> Self {
        Self::new(
            &format!("evm-{chain_id}"),
            &format!("EVM chain {chain_id}"),
            ChainKind::Evm,
            "ETH",
            18,
        )
        .with_evm_chain_id(chain_id)
    }

    pub fn solana_mainnet() -> Self {
        known_chain("solana").expect("built-in").info()
    }

    pub fn solana_devnet() -> Self {
        known_chain("solana-devnet").expect("built-in").info()
    }

    pub fn bitcoin_mainnet() -> Self {
        known_chain("bitcoin").expect("built-in").info()
    }

    pub fn bitcoin_testnet() -> Self {
        known_chain("bitcoin-testnet").expect("built-in").info()
    }
}

/// A built-in chain: metadata plus, where the chain publishes one, a free public RPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownChain {
    /// Shorthand used by [`resolve_rpc`] and `EvmClient::connect` (`"ronin"`, `"base"`, ...).
    pub key: &'static str,
    pub name: &'static str,
    pub kind: ChainKind,
    /// EIP-155 chain id (see <https://chainlist.org>); `None` for non-EVM chains.
    pub evm_chain_id: Option<u64>,
    pub symbol: &'static str,
    pub decimals: u8,
    /// Free public endpoint from the chain's own docs. Rate limited and meant for trying
    /// things out; use your own node or a provider in production.
    pub public_rpc: Option<&'static str>,
}

impl KnownChain {
    pub fn info(&self) -> ChainInfo {
        ChainInfo {
            key: self.key.into(),
            name: self.name.into(),
            kind: self.kind,
            evm_chain_id: self.evm_chain_id,
            native_symbol: self.symbol.into(),
            native_decimals: self.decimals,
        }
    }

    /// Environment variable that overrides this chain's RPC: `EVMIFY_RPC_<KEY>`, upper
    /// case with `-` as `_`, e.g. `EVMIFY_RPC_RONIN`, `EVMIFY_RPC_BASE_SEPOLIA`.
    pub fn env_var(&self) -> String {
        format!("EVMIFY_RPC_{}", self.key.to_uppercase().replace('-', "_"))
    }
}

const fn evm(
    id: u64,
    key: &'static str,
    name: &'static str,
    symbol: &'static str,
    public_rpc: Option<&'static str>,
) -> KnownChain {
    KnownChain {
        key,
        name,
        kind: ChainKind::Evm,
        evm_chain_id: Some(id),
        symbol,
        decimals: 18,
        public_rpc,
    }
}

/// Every built-in chain. Add EVM chains with their EIP-155 id from
/// <https://chainlist.org>, and only list a `public_rpc` the chain's own docs publish.
pub const KNOWN_CHAINS: &[KnownChain] = &[
    // EVM mainnets
    evm(
        1,
        "ethereum",
        "Ethereum",
        "ETH",
        Some("https://ethereum-rpc.publicnode.com"),
    ),
    evm(
        10,
        "optimism",
        "OP Mainnet",
        "ETH",
        Some("https://mainnet.optimism.io"),
    ),
    evm(
        56,
        "bsc",
        "BNB Smart Chain",
        "BNB",
        Some("https://bsc-dataseed.bnbchain.org"),
    ),
    evm(
        100,
        "gnosis",
        "Gnosis",
        "xDAI",
        Some("https://rpc.gnosischain.com"),
    ),
    evm(
        137,
        "polygon",
        "Polygon PoS",
        "POL",
        Some("https://polygon.drpc.org"),
    ),
    evm(
        324,
        "zksync",
        "zkSync Era",
        "ETH",
        Some("https://mainnet.era.zksync.io"),
    ),
    evm(
        2020,
        "ronin",
        "Ronin",
        "RON",
        Some("https://api.roninchain.com/rpc"),
    ),
    evm(5000, "mantle", "Mantle", "MNT", None),
    evm(
        8453,
        "base",
        "Base",
        "ETH",
        Some("https://mainnet.base.org"),
    ),
    evm(
        42161,
        "arbitrum",
        "Arbitrum One",
        "ETH",
        Some("https://arb1.arbitrum.io/rpc"),
    ),
    evm(
        42220,
        "celo",
        "Celo",
        "CELO",
        Some("https://forno.celo.org"),
    ),
    evm(
        43114,
        "avalanche",
        "Avalanche C-Chain",
        "AVAX",
        Some("https://api.avax.network/ext/bc/C/rpc"),
    ),
    evm(
        59144,
        "linea",
        "Linea",
        "ETH",
        Some("https://rpc.linea.build"),
    ),
    evm(81457, "blast", "Blast", "ETH", None),
    evm(
        534352,
        "scroll",
        "Scroll",
        "ETH",
        Some("https://rpc.scroll.io"),
    ),
    // EVM testnets
    evm(
        11155111,
        "sepolia",
        "Sepolia",
        "ETH",
        Some("https://ethereum-sepolia-rpc.publicnode.com"),
    ),
    evm(
        84532,
        "base-sepolia",
        "Base Sepolia",
        "ETH",
        Some("https://sepolia.base.org"),
    ),
    evm(
        202601,
        "ronin-saigon",
        "Ronin Saigon",
        "RON",
        Some("https://saigon-testnet.roninchain.com/rpc"),
    ),
    // Non-EVM
    KnownChain {
        key: "solana",
        name: "Solana",
        kind: ChainKind::Solana,
        evm_chain_id: None,
        symbol: "SOL",
        decimals: 9,
        public_rpc: Some("https://api.mainnet-beta.solana.com"),
    },
    KnownChain {
        key: "solana-devnet",
        name: "Solana Devnet",
        kind: ChainKind::Solana,
        evm_chain_id: None,
        symbol: "SOL",
        decimals: 9,
        public_rpc: Some("https://api.devnet.solana.com"),
    },
    // Bitcoin has no public JSON-RPC: point EVMIFY_RPC_BITCOIN at your own node.
    KnownChain {
        key: "bitcoin",
        name: "Bitcoin",
        kind: ChainKind::Bitcoin,
        evm_chain_id: None,
        symbol: "BTC",
        decimals: 8,
        public_rpc: None,
    },
    KnownChain {
        key: "bitcoin-testnet",
        name: "Bitcoin Testnet",
        kind: ChainKind::Bitcoin,
        evm_chain_id: None,
        symbol: "tBTC",
        decimals: 8,
        public_rpc: None,
    },
];

/// A built-in chain by shorthand key (case-insensitive).
pub fn known_chain(key: &str) -> Option<&'static KnownChain> {
    KNOWN_CHAINS
        .iter()
        .find(|c| c.key.eq_ignore_ascii_case(key))
}

/// A built-in EVM chain by EIP-155 id.
pub fn by_id(chain_id: u64) -> Option<ChainInfo> {
    KNOWN_CHAINS
        .iter()
        .find(|c| c.evm_chain_id == Some(chain_id))
        .map(KnownChain::info)
}

/// A built-in chain by key (`"base"`, `"arbitrum"`, `"solana"`, ...).
pub fn by_key(key: &str) -> Option<ChainInfo> {
    known_chain(key).map(KnownChain::info)
}

/// Where an RPC URL came from, as decided by [`resolve_rpc`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpcSource {
    /// The caller passed a URL.
    Url,
    /// `EVMIFY_RPC_<KEY>` was set.
    Env(String),
    /// The chain's built-in public RPC.
    Public,
}

/// A resolved endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRpc {
    pub url: String,
    pub source: RpcSource,
    /// The built-in chain, when `target` was a shorthand key.
    pub chain: Option<&'static KnownChain>,
}

/// Turn a URL or a shorthand key into an RPC URL.
///
/// * anything containing `://` is used as is;
/// * otherwise it is a key from [`KNOWN_CHAINS`], and the URL comes from the
///   `EVMIFY_RPC_<KEY>` environment variable if set, else the chain's public RPC.
pub fn resolve_rpc(target: &str) -> crate::Result<ResolvedRpc> {
    resolve_rpc_with(target, |name| std::env::var(name).ok())
}

/// [`resolve_rpc`] with a custom environment lookup (handy in tests or for config files).
pub fn resolve_rpc_with(
    target: &str,
    env: impl Fn(&str) -> Option<String>,
) -> crate::Result<ResolvedRpc> {
    if target.contains("://") {
        return Ok(ResolvedRpc {
            url: target.to_string(),
            source: RpcSource::Url,
            chain: None,
        });
    }
    let chain = known_chain(target).ok_or_else(|| {
        crate::Error::Unsupported(format!(
            "unknown chain `{target}`: pass an RPC URL, or one of: {}",
            KNOWN_CHAINS
                .iter()
                .map(|c| c.key)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;
    let var = chain.env_var();
    if let Some(url) = env(&var).filter(|u| !u.trim().is_empty()) {
        return Ok(ResolvedRpc {
            url,
            source: RpcSource::Env(var),
            chain: Some(chain),
        });
    }
    match chain.public_rpc {
        Some(url) => Ok(ResolvedRpc {
            url: url.to_string(),
            source: RpcSource::Public,
            chain: Some(chain),
        }),
        None => Err(crate::Error::Unsupported(format!(
            "{} has no built-in public RPC: set {var} or pass a URL",
            chain.name
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_ids_and_env_names_are_unique() {
        let mut keys: Vec<_> = KNOWN_CHAINS.iter().map(|c| c.key).collect();
        let mut ids: Vec<_> = KNOWN_CHAINS.iter().filter_map(|c| c.evm_chain_id).collect();
        let (k, i) = (keys.len(), ids.len());
        keys.sort();
        keys.dedup();
        ids.sort();
        ids.dedup();
        assert_eq!((keys.len(), ids.len()), (k, i));
        for c in KNOWN_CHAINS {
            assert_eq!(c.key, c.key.to_lowercase(), "keys are lowercase");
            assert!(c.public_rpc.is_none_or(|u| u.starts_with("https://")));
        }
    }

    #[test]
    fn resolution_order() {
        let no_env = |_: &str| None;
        let r = resolve_rpc_with("ronin", no_env).unwrap();
        assert_eq!(r.url, "https://api.roninchain.com/rpc");
        assert_eq!(r.source, RpcSource::Public);
        assert_eq!(r.chain.unwrap().evm_chain_id, Some(2020));

        let r = resolve_rpc_with("Ronin", |v| {
            (v == "EVMIFY_RPC_RONIN").then(|| "https://my-node:8545".to_string())
        })
        .unwrap();
        assert_eq!(r.url, "https://my-node:8545");
        assert_eq!(r.source, RpcSource::Env("EVMIFY_RPC_RONIN".into()));

        let r = resolve_rpc_with("http://127.0.0.1:8545", no_env).unwrap();
        assert_eq!((r.source, r.chain), (RpcSource::Url, None));

        let e = resolve_rpc_with("bitcoin", no_env).unwrap_err().to_string();
        assert!(e.contains("EVMIFY_RPC_BITCOIN"), "{e}");
        let e = resolve_rpc_with("nope", no_env).unwrap_err().to_string();
        assert!(e.contains("ronin") && e.contains("solana"), "{e}");
        assert_eq!(
            known_chain("base-sepolia").unwrap().env_var(),
            "EVMIFY_RPC_BASE_SEPOLIA"
        );
    }
}
