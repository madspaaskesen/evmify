use serde_json::Value;

/// Result alias used throughout the crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything that can go wrong between a caller and a chain.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The request never got a JSON-RPC answer (network, TLS, HTTP status, timeout).
    /// [`crate::transport::FallbackTransport`] retries the next endpoint on this variant.
    #[error("transport error: {0}")]
    Transport(String),

    /// The node answered with a JSON-RPC error object.
    #[error("rpc error {code}: {message}")]
    Rpc {
        code: i64,
        message: String,
        data: Option<Value>,
    },

    /// The node answered, but the payload did not have the expected shape.
    #[error("failed to decode `{context}`: {source}")]
    Decode {
        context: String,
        #[source]
        source: serde_json::Error,
    },

    /// A normalizer could not make sense of a native payload.
    #[error("normalize error: {0}")]
    Normalize(String),

    /// The operation has no meaning (or no implementation) for this chain.
    #[error("unsupported: {0}")]
    Unsupported(String),
}

impl Error {
    pub(crate) fn decode(context: impl Into<String>, source: serde_json::Error) -> Self {
        Error::Decode {
            context: context.into(),
            source,
        }
    }

    /// True when retrying against another endpoint could help.
    pub fn is_transport(&self) -> bool {
        matches!(self, Error::Transport(_))
    }
}
