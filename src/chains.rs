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
    /// Stable key, e.g. `"ethereum"`, `"solana-mainnet"`.
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
        Self::new("solana-mainnet", "Solana", ChainKind::Solana, "SOL", 9)
    }

    pub fn solana_devnet() -> Self {
        Self::new(
            "solana-devnet",
            "Solana Devnet",
            ChainKind::Solana,
            "SOL",
            9,
        )
    }

    pub fn bitcoin_mainnet() -> Self {
        Self::new("bitcoin", "Bitcoin", ChainKind::Bitcoin, "BTC", 8)
    }

    pub fn bitcoin_testnet() -> Self {
        Self::new(
            "bitcoin-testnet",
            "Bitcoin Testnet",
            ChainKind::Bitcoin,
            "tBTC",
            8,
        )
    }
}

/// `(chain id, key, name, native symbol)` for well-known EVM networks.
pub const KNOWN_EVM_CHAINS: &[(u64, &str, &str, &str)] = &[
    (1, "ethereum", "Ethereum", "ETH"),
    (10, "optimism", "OP Mainnet", "ETH"),
    (56, "bsc", "BNB Smart Chain", "BNB"),
    (100, "gnosis", "Gnosis", "xDAI"),
    (137, "polygon", "Polygon PoS", "POL"),
    (324, "zksync", "zkSync Era", "ETH"),
    (5000, "mantle", "Mantle", "MNT"),
    (8453, "base", "Base", "ETH"),
    (42161, "arbitrum", "Arbitrum One", "ETH"),
    (42220, "celo", "Celo", "CELO"),
    (43114, "avalanche", "Avalanche C-Chain", "AVAX"),
    (59144, "linea", "Linea", "ETH"),
    (81457, "blast", "Blast", "ETH"),
    (534352, "scroll", "Scroll", "ETH"),
    (11155111, "sepolia", "Sepolia", "ETH"),
    (84532, "base-sepolia", "Base Sepolia", "ETH"),
];

/// Look up a well-known EVM chain by EIP-155 id.
pub fn by_id(chain_id: u64) -> Option<ChainInfo> {
    KNOWN_EVM_CHAINS
        .iter()
        .find(|(id, ..)| *id == chain_id)
        .map(|(id, key, name, sym)| {
            ChainInfo::new(key, name, ChainKind::Evm, sym, 18).with_evm_chain_id(*id)
        })
}

/// Look up a well-known EVM chain by key (`"base"`, `"arbitrum"`, ...).
pub fn by_key(key: &str) -> Option<ChainInfo> {
    KNOWN_EVM_CHAINS
        .iter()
        .find(|(_, k, ..)| *k == key)
        .and_then(|(id, ..)| by_id(*id))
}
