//! HttpTransport against a tiny local HTTP server.
#![cfg(feature = "http")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use evmify::{Error, HttpTransport, Transport};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// Serves `responses` in order (the last one repeats), one per connection.
async fn serve(responses: Vec<&'static str>) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = listener.accept().await.unwrap();
            let n = counter.fetch_add(1, Ordering::SeqCst);
            let body = responses[n.min(responses.len() - 1)];
            // Read the request (headers + Content-Length body), then answer.
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let read = sock.read(&mut chunk).await.unwrap();
                if read == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..read]);
                let text = String::from_utf8_lossy(&buf);
                if let Some(end) = text.find("\r\n\r\n") {
                    let len = text
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if buf.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            sock.write_all(body.as_bytes()).await.unwrap();
            sock.shutdown().await.ok();
        }
    });
    (url, hits)
}

const TOO_MANY: &str = "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\ncontent-type: application/json\r\ncontent-length: 37\r\nconnection: close\r\n\r\n{\"message\":\"API rate limit exceeded\"}";
const OK: &str = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 41\r\nconnection: close\r\n\r\n{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":\"0x7e4\"}";

#[tokio::test]
async fn retries_429_then_succeeds() {
    let (url, hits) = serve(vec![TOO_MANY, TOO_MANY, OK]).await;
    let t = HttpTransport::new(url);
    let started = Instant::now();
    let result = t.request("eth_chainId", json!([])).await.unwrap();
    assert_eq!(result, json!("0x7e4"));
    assert_eq!(hits.load(Ordering::SeqCst), 3);
    // Retry-After: 0 is honoured instead of the default backoff.
    assert!(started.elapsed().as_secs() < 2);
}

#[tokio::test]
async fn gives_up_after_max_retries() {
    let (url, hits) = serve(vec![TOO_MANY]).await;
    let t = HttpTransport::new(url).max_retries(1);
    let err = t.request("eth_chainId", json!([])).await.unwrap_err();
    assert!(
        matches!(&err, Error::Transport(m) if m.contains("429")),
        "{err}"
    );
    assert_eq!(hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn no_retries_when_disabled() {
    let (url, hits) = serve(vec![TOO_MANY, OK]).await;
    let t = HttpTransport::new(url).max_retries(0);
    assert!(t.request("eth_chainId", json!([])).await.is_err());
    assert_eq!(hits.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn max_rps_spaces_all_requests() {
    let (url, hits) = serve(vec![OK]).await;
    let t = Arc::new(HttpTransport::new(url).max_rps(20.0)); // 50 ms apart
    let started = Instant::now();
    // Concurrent callers share the same budget.
    let calls: Vec<_> = (0..4)
        .map(|_| {
            let t = t.clone();
            tokio::spawn(async move { t.request("eth_chainId", json!([])).await })
        })
        .collect();
    for c in calls {
        c.await.unwrap().unwrap();
    }
    assert_eq!(hits.load(Ordering::SeqCst), 4);
    assert!(
        started.elapsed().as_millis() >= 150,
        "{:?}",
        started.elapsed()
    );
}
