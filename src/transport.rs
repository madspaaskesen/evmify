//! How JSON-RPC requests reach a node.
//!
//! [`Transport`] is the only thing clients and adapters depend on, so you can plug in
//! WebSockets, IPC, a rate limiter, a cache or a recorder without touching the rest.

use std::collections::{HashMap, VecDeque};
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, Result};

/// Sends one JSON-RPC call and returns its `result`.
#[async_trait]
pub trait Transport: Send + Sync + Debug {
    async fn request(&self, method: &str, params: Value) -> Result<Value>;
}

#[async_trait]
impl<T: Transport + ?Sized> Transport for Arc<T> {
    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        (**self).request(method, params).await
    }
}

#[async_trait]
impl<T: Transport + ?Sized> Transport for Box<T> {
    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        (**self).request(method, params).await
    }
}

/// JSON-RPC 2.0 request envelope.
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcRequest<'a> {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: &'a str,
    pub params: Value,
}

/// JSON-RPC 2.0 response envelope.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcResponse {
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    /// Turn the envelope into `Ok(result)` or [`Error::Rpc`].
    pub fn into_result(self) -> Result<Value> {
        match (self.error, self.result) {
            (Some(e), _) => Err(Error::Rpc {
                code: e.code,
                message: e.message,
                data: e.data,
            }),
            (None, Some(v)) => Ok(v),
            (None, None) => Ok(Value::Null),
        }
    }
}

#[cfg(feature = "http")]
pub use http::HttpTransport;

#[cfg(feature = "http")]
mod http {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use super::*;

    /// JSON-RPC over HTTP(S), via reqwest + rustls.
    #[derive(Debug)]
    pub struct HttpTransport {
        client: reqwest::Client,
        url: String,
        auth: Option<Auth>,
        next_id: AtomicU64,
        max_retries: u32,
        min_interval: Option<Duration>,
        next_slot: Mutex<Option<Instant>>,
    }

    /// Longest pause between retries, whatever `Retry-After` asks for.
    const MAX_BACKOFF: Duration = Duration::from_secs(30);

    #[derive(Debug, Clone)]
    enum Auth {
        Bearer(String),
        Basic(String, Option<String>),
    }

    impl HttpTransport {
        pub fn new(url: impl Into<String>) -> Self {
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("reqwest client builds with static config");
            Self::with_client(client, url)
        }

        /// Bring your own client (custom timeouts, proxies, headers).
        pub fn with_client(client: reqwest::Client, url: impl Into<String>) -> Self {
            Self {
                client,
                url: url.into(),
                auth: None,
                next_id: AtomicU64::new(1),
                max_retries: 3,
                min_interval: None,
                next_slot: Mutex::new(None),
            }
        }

        /// Send `Authorization: Bearer <token>` with every request.
        pub fn bearer(mut self, token: impl Into<String>) -> Self {
            self.auth = Some(Auth::Bearer(token.into()));
            self
        }

        /// HTTP basic auth (e.g. Bitcoin Core `rpcuser` / `rpcpassword`).
        pub fn basic_auth(mut self, user: impl Into<String>, password: Option<String>) -> Self {
            self.auth = Some(Auth::Basic(user.into(), password));
            self
        }

        /// How often to retry when the server says it is overloaded (HTTP 429 or 503).
        /// Each retry waits for the server's `Retry-After`, or else 0.5 s, 1 s, 2 s, …
        /// (capped at 30 s). Default 3; `0` disables retries.
        pub fn max_retries(mut self, retries: u32) -> Self {
            self.max_retries = retries;
            self
        }

        /// Send at most `requests_per_second` requests, spaced evenly, across every call
        /// made through this transport (also concurrent ones). Keeps a client under a
        /// provider's limit instead of running into HTTP 429. `0` disables it (default).
        pub fn max_rps(mut self, requests_per_second: f64) -> Self {
            self.min_interval = (requests_per_second > 0.0)
                .then(|| Duration::from_secs_f64(1.0 / requests_per_second));
            self
        }

        pub fn url(&self) -> &str {
            &self.url
        }

        /// Reserve the next send slot and wait for it.
        async fn pace(&self) {
            let Some(interval) = self.min_interval else {
                return;
            };
            let wait = {
                let mut slot = self.next_slot.lock().unwrap();
                let now = Instant::now();
                let start = slot.map_or(now, |s| s.max(now));
                *slot = Some(start + interval);
                start.saturating_duration_since(now)
            };
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
        }

        fn backoff(attempt: u32, retry_after: Option<&reqwest::header::HeaderValue>) -> Duration {
            retry_after
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(Duration::from_secs)
                .unwrap_or_else(|| Duration::from_millis(500) * 2u32.saturating_pow(attempt))
                .min(MAX_BACKOFF)
        }
    }

    #[async_trait]
    impl Transport for HttpTransport {
        async fn request(&self, method: &str, params: Value) -> Result<Value> {
            let body = JsonRpcRequest {
                jsonrpc: "2.0",
                id: self.next_id.fetch_add(1, Ordering::Relaxed),
                method,
                params,
            };
            let mut attempt = 0;
            let resp = loop {
                self.pace().await;
                let mut req = self.client.post(&self.url).json(&body);
                match &self.auth {
                    Some(Auth::Bearer(token)) => req = req.bearer_auth(token),
                    Some(Auth::Basic(user, pass)) => req = req.basic_auth(user, pass.as_ref()),
                    None => {}
                }
                let resp = req
                    .send()
                    .await
                    .map_err(|e| Error::Transport(format!("{}: {e}", self.url)))?;
                let overloaded = matches!(resp.status().as_u16(), 429 | 503);
                if !overloaded || attempt >= self.max_retries {
                    break resp;
                }
                let wait = Self::backoff(attempt, resp.headers().get(reqwest::header::RETRY_AFTER));
                tokio::time::sleep(wait).await;
                attempt += 1;
            };
            let status = resp.status();
            let text = resp
                .text()
                .await
                .map_err(|e| Error::Transport(format!("{}: {e}", self.url)))?;
            // Many providers return a JSON-RPC error body together with a 4xx/5xx status;
            // prefer that structured error when it is there.
            match serde_json::from_str::<JsonRpcResponse>(&text) {
                Ok(envelope) if envelope.error.is_some() || status.is_success() => {
                    envelope.into_result()
                }
                _ if !status.is_success() => Err(Error::Transport(format!(
                    "{}: HTTP {status}: {}",
                    self.url,
                    text.chars().take(200).collect::<String>()
                ))),
                Err(e) => Err(Error::decode(format!("{method} response"), e)),
                Ok(envelope) => envelope.into_result(),
            }
        }
    }
}

/// Tries each endpoint in order, moving on only on transport failures.
///
/// RPC errors (e.g. "execution reverted") are returned immediately, since another node
/// would answer the same way.
#[derive(Debug, Clone)]
pub struct FallbackTransport {
    endpoints: Vec<Arc<dyn Transport>>,
}

impl FallbackTransport {
    pub fn new(endpoints: Vec<Arc<dyn Transport>>) -> Self {
        assert!(
            !endpoints.is_empty(),
            "FallbackTransport needs at least one endpoint"
        );
        Self { endpoints }
    }
}

#[async_trait]
impl Transport for FallbackTransport {
    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let mut failures = Vec::new();
        for endpoint in &self.endpoints {
            match endpoint.request(method, params.clone()).await {
                Err(e) if e.is_transport() => failures.push(e.to_string()),
                other => return other,
            }
        }
        Err(Error::Transport(format!(
            "all {} endpoints failed: {}",
            self.endpoints.len(),
            failures.join("; ")
        )))
    }
}

/// In-memory transport for tests: canned responses per method, plus a call log.
///
/// ```
/// use evmify::transport::MockTransport;
/// use serde_json::json;
///
/// let mock = MockTransport::new().on("eth_chainId", json!("0x1"));
/// ```
#[derive(Debug, Default, Clone)]
pub struct MockTransport {
    inner: Arc<Mutex<MockState>>,
}

type MockResponse = Result<Value, (i64, String)>;
type MockHandler = Arc<dyn Fn(&Value) -> Value + Send + Sync>;

#[derive(Default)]
struct MockState {
    /// Responses for a method; the last one repeats once the queue is down to one.
    responses: HashMap<String, VecDeque<MockResponse>>,
    /// Computed responses, checked before `responses`.
    handlers: HashMap<String, MockHandler>,
    calls: Vec<(String, Value)>,
}

impl Debug for MockState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockState")
            .field("responses", &self.responses)
            .field("handlers", &self.handlers.keys().collect::<Vec<_>>())
            .field("calls", &self.calls)
            .finish()
    }
}

impl MockTransport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue a successful result for `method`.
    pub fn on(self, method: &str, result: Value) -> Self {
        self.push(method, Ok(result));
        self
    }

    /// Queue a JSON-RPC error for `method`.
    pub fn on_error(self, method: &str, code: i64, message: &str) -> Self {
        self.push(method, Err((code, message.to_string())));
        self
    }

    /// Answer `method` by computing the result from its params, e.g. return block `n` for
    /// `eth_getBlockByNumber(n)`. Takes precedence over queued responses.
    pub fn on_fn(self, method: &str, f: impl Fn(&Value) -> Value + Send + Sync + 'static) -> Self {
        self.inner
            .lock()
            .unwrap()
            .handlers
            .insert(method.to_string(), Arc::new(f));
        self
    }

    fn push(&self, method: &str, r: MockResponse) {
        self.inner
            .lock()
            .unwrap()
            .responses
            .entry(method.to_string())
            .or_default()
            .push_back(r);
    }

    /// Every `(method, params)` seen so far, in order.
    pub fn calls(&self) -> Vec<(String, Value)> {
        self.inner.lock().unwrap().calls.clone()
    }
}

#[async_trait]
impl Transport for MockTransport {
    async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let mut state = self.inner.lock().unwrap();
        if let Some(handler) = state.handlers.get(method).cloned() {
            let result = handler(&params);
            state.calls.push((method.to_string(), params));
            return Ok(result);
        }
        state.calls.push((method.to_string(), params));
        let queue = state
            .responses
            .get_mut(method)
            .ok_or_else(|| Error::Transport(format!("mock: no response for `{method}`")))?;
        let next = if queue.len() > 1 {
            queue.pop_front().unwrap()
        } else {
            queue.front().cloned().unwrap()
        };
        next.map_err(|(code, message)| Error::Rpc {
            code,
            message,
            data: None,
        })
    }
}
