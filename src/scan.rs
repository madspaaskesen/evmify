//! Scanning `eth_getLogs` over long block ranges, politely.
//!
//! Providers cap how many blocks one `eth_getLogs` call may cover (Ronin: 200) and how
//! many calls you may make per second. [`LogScan`] splits a range into windows, can walk
//! **newest first** and stop as soon as it has found enough, halves a window when the
//! node rejects it as too large, and can throttle itself to a request rate.
//!
//! The scan holds no state between runs: caching or storing results is up to the caller.
//!
//! ```no_run
//! # #[cfg(feature = "http")]
//! # async fn demo() -> evmify::Result<()> {
//! use evmify::{EvmClient, Filter};
//!
//! let client = EvmClient::connect("ronin").await?;
//! let transfers = Filter::new().event("Transfer(address,address,uint256)");
//! let latest = client
//!     .scan_logs(transfers)
//!     .newest_first()
//!     .limit(20)
//!     .max_blocks(10_000)
//!     .rate_limit(5.0) // requests per second
//!     .on_progress(|p| eprint!("\r{}/{} blocks", p.scanned, p.total))
//!     .run()
//!     .await?;
//! # Ok(()) }
//! ```

use std::time::{Duration, Instant};

use crate::error::{Error, Result};
use crate::evm::EvmClient;
use crate::transport::Transport;
use crate::types::{Filter, Log};

/// Default number of blocks a scan looks back when no start block is given.
pub const DEFAULT_MAX_BLOCKS: u64 = 10_000;
/// Default window size: Ronin's limit, and safe for most public RPCs.
pub const DEFAULT_CHUNK: u64 = 200;

/// Passed to the [`LogScan::on_progress`] callback after every window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanProgress {
    /// Blocks covered so far.
    pub scanned: u64,
    /// Blocks in the whole range.
    pub total: u64,
    /// `eth_getLogs` calls made so far (including rejected ones).
    pub requests: u64,
    /// Logs found so far.
    pub found: usize,
    /// The window just finished, `(from, to)` inclusive.
    pub window: (u64, u64),
}

/// What a scan will cover; see [`LogScan::plan`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScanPlan {
    pub from: u64,
    pub to: u64,
    pub blocks: u64,
    /// Calls needed to cover the whole range. A `limit` can make it fewer; windows
    /// halved after a "range too large" error make it more.
    pub max_requests: u64,
    /// The least time `max_requests` take under the configured rate limit.
    pub min_duration: Option<Duration>,
}

type ProgressFn<'a> = Box<dyn FnMut(&ScanProgress) + Send + 'a>;

/// Builder returned by [`EvmClient::scan_logs`].
#[must_use = "a LogScan does nothing until `.run()` is awaited"]
pub struct LogScan<'a, T> {
    client: &'a EvmClient<T>,
    filters: Vec<Filter>,
    newest_first: bool,
    limit: Option<usize>,
    from: Option<u64>,
    to: Option<u64>,
    max_blocks: u64,
    chunk: u64,
    min_interval: Option<Duration>,
    progress: Option<ProgressFn<'a>>,
    resolved: Option<(u64, u64)>,
}

impl<'a, T: Transport> LogScan<'a, T> {
    pub(crate) fn new(client: &'a EvmClient<T>, filter: Filter) -> Self {
        Self {
            client,
            filters: vec![filter],
            newest_first: false,
            limit: None,
            from: None,
            to: None,
            max_blocks: DEFAULT_MAX_BLOCKS,
            chunk: DEFAULT_CHUNK,
            min_interval: None,
            progress: None,
            resolved: None,
        }
    }

    /// Also match this filter (logical OR), e.g. transfers *to* as well as *from* an
    /// address. Each window queries every filter; duplicates are removed.
    pub fn or_filter(mut self, filter: Filter) -> Self {
        self.filters.push(filter);
        self
    }

    /// Walk from the newest block backwards; results come newest first.
    pub fn newest_first(mut self) -> Self {
        self.newest_first = true;
        self
    }

    /// Walk from the oldest block forwards (the default); results come oldest first.
    pub fn oldest_first(mut self) -> Self {
        self.newest_first = false;
        self
    }

    /// Stop once this many logs are found.
    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    /// First block of the range. Without it, the range is the last
    /// [`max_blocks`](Self::max_blocks) blocks up to `to_block`.
    pub fn from_block(mut self, n: u64) -> Self {
        self.from = Some(n);
        self.resolved = None;
        self
    }

    /// Last block of the range (default: the chain head at the time of the scan).
    pub fn to_block(mut self, n: u64) -> Self {
        self.to = Some(n);
        self.resolved = None;
        self
    }

    /// How far back to look when no `from_block` is given (default 10 000).
    pub fn max_blocks(mut self, n: u64) -> Self {
        self.max_blocks = n.max(1);
        self.resolved = None;
        self
    }

    /// Blocks per `eth_getLogs` call (default 200). Halved automatically while the node
    /// keeps rejecting the range.
    pub fn chunk(mut self, blocks: u64) -> Self {
        self.chunk = blocks.max(1);
        self
    }

    /// At most `requests_per_second` calls, spaced evenly. Needs the `http` feature
    /// (the timer comes from tokio).
    #[cfg(feature = "http")]
    pub fn rate_limit(mut self, requests_per_second: f64) -> Self {
        self.min_interval =
            (requests_per_second > 0.0).then(|| Duration::from_secs_f64(1.0 / requests_per_second));
        self
    }

    /// Called after every window, e.g. to draw a progress line.
    pub fn on_progress(mut self, f: impl FnMut(&ScanProgress) + Send + 'a) -> Self {
        self.progress = Some(Box::new(f));
        self
    }

    async fn range(&mut self) -> Result<(u64, u64)> {
        if let Some(r) = self.resolved {
            return Ok(r);
        }
        let to = match self.to {
            Some(to) => to,
            None => self.client.block_number().await?,
        };
        let from = self
            .from
            .unwrap_or_else(|| to.saturating_sub(self.max_blocks - 1));
        self.resolved = Some((from, to));
        Ok((from, to))
    }

    /// Resolve the range (one `eth_blockNumber` call if `to_block` is unset) and estimate
    /// the cost, e.g. to ask the user before a long scan. [`run`](Self::run) reuses it.
    pub async fn plan(&mut self) -> Result<ScanPlan> {
        let (from, to) = self.range().await?;
        let blocks = if from <= to { to - from + 1 } else { 0 };
        let max_requests = blocks.div_ceil(self.chunk) * self.filters.len() as u64;
        let min_duration = self.min_interval.map(|interval| {
            interval
                .saturating_mul(u32::try_from(max_requests.saturating_sub(1)).unwrap_or(u32::MAX))
        });
        Ok(ScanPlan {
            from,
            to,
            blocks,
            max_requests,
            min_duration,
        })
    }

    /// Run the scan.
    pub async fn run(mut self) -> Result<Vec<Log>> {
        let (from, to) = self.range().await?;
        let mut found = Vec::new();
        if from > to {
            return Ok(found);
        }
        let total = to - from + 1;
        let (mut lo, mut hi) = (from, to);
        let mut window = self.chunk;
        let (mut scanned, mut requests) = (0u64, 0u64);
        let mut last_request: Option<Instant> = None;

        loop {
            let (start, end) = if self.newest_first {
                (hi.saturating_sub(window - 1).max(lo), hi)
            } else {
                (lo, lo.saturating_add(window - 1).min(hi))
            };

            let mut batch = Vec::new();
            let mut too_large = false;
            for filter in &self.filters {
                self.throttle(&mut last_request).await;
                let mut query = filter.clone();
                query.block_hash = None;
                let query = query.from_block(start).to_block(end);
                requests += 1;
                match self.client.get_logs(&query).await {
                    Ok(logs) => batch.extend(logs),
                    Err(Error::Rpc { code, .. }) if is_range_error(code) && window > 1 => {
                        too_large = true;
                        break;
                    }
                    Err(e) => return Err(e),
                }
            }
            if too_large {
                window = (window / 2).max(1);
                continue;
            }

            // log_index is unique within a block, so this also drops OR-filter duplicates.
            batch.sort_by_key(|l| (l.block_number, l.log_index));
            batch.dedup_by_key(|l| (l.block_number, l.log_index));
            if self.newest_first {
                batch.reverse();
            }
            found.extend(batch);
            scanned += end - start + 1;

            if let Some(report) = self.progress.as_mut() {
                report(&ScanProgress {
                    scanned,
                    total,
                    requests,
                    found: found.len(),
                    window: (start, end),
                });
            }
            if let Some(limit) = self.limit
                && found.len() >= limit
            {
                found.truncate(limit);
                break;
            }
            if self.newest_first {
                if start == lo {
                    break;
                }
                hi = start - 1;
            } else {
                if end == hi {
                    break;
                }
                lo = end + 1;
            }
        }
        Ok(found)
    }

    #[cfg(feature = "http")]
    async fn throttle(&self, last: &mut Option<Instant>) {
        if let (Some(interval), Some(previous)) = (self.min_interval, *last) {
            let elapsed = previous.elapsed();
            if elapsed < interval {
                tokio::time::sleep(interval - elapsed).await;
            }
        }
        *last = Some(Instant::now());
    }

    #[cfg(not(feature = "http"))]
    async fn throttle(&self, _last: &mut Option<Instant>) {}
}

/// Error codes providers use for "too many blocks / results in one eth_getLogs".
pub(crate) fn is_range_error(code: i64) -> bool {
    matches!(code, -32602 | -32005 | -32600 | -32000)
}
