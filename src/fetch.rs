//! Shared HTTP layer: conditional GETs, content hashing and raw archiving.

use crate::config::Config;
use crate::db::{self, Pool};
use anyhow::{Context, Result, bail};
use reqwest::StatusCode;
use reqwest::header::{ETAG, HeaderMap, HeaderValue, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use rusqlite::{OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

#[derive(Clone)]
pub struct Ctx {
    pub cfg: Arc<Config>,
    pub http: reqwest::Client,
    pub db: Pool,
    /// Bumped whenever any source stores new data; used to invalidate API caches.
    pub data_version: Arc<AtomicU64>,
}

impl Ctx {
    pub fn new(cfg: Arc<Config>, db: Pool) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(cfg.user_agent.clone())
            .connect_timeout(Duration::from_secs(20))
            .timeout(Duration::from_secs(180))
            .build()?;
        Ok(Ctx { cfg, http, db, data_version: Arc::new(AtomicU64::new(1)) })
    }

    pub fn bump(&self) {
        self.data_version.fetch_add(1, Ordering::Relaxed);
    }
}

pub enum Fetched {
    /// Body differs from the last successful fetch of this URL.
    Changed(Vec<u8>),
    Unchanged,
}

/// GET with If-None-Match / If-Modified-Since and a content hash check.
/// The cache row is only committed by `commit`, after the caller parsed and stored
/// the body successfully, so a parse failure is retried on the next run.
pub async fn get(ctx: &Ctx, source_id: &str, url: &str) -> Result<(Fetched, Pending)> {
    let cached: Option<(Option<String>, Option<String>, Option<String>)> = {
        let conn = ctx.db.get()?;
        conn.query_row(
            "SELECT etag, last_modified, content_hash FROM http_cache WHERE url=?1",
            [url],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?
    };
    let reply = send_with_retry(url, || {
        let mut req = ctx.http.get(url);
        if let Some((etag, lm, _)) = &cached {
            if let Some(e) = etag {
                req = req.header(IF_NONE_MATCH, e);
            }
            if let Some(l) = lm {
                req = req.header(IF_MODIFIED_SINCE, l);
            }
        }
        req
    })
    .await?;
    let status = reply.status;
    if status == StatusCode::NOT_MODIFIED {
        return Ok((Fetched::Unchanged, Pending::none()));
    }
    if !status.is_success() {
        bail!("GET {url}: HTTP {status}");
    }
    let header = |h| reply.headers.get(h).and_then(|v: &HeaderValue| v.to_str().ok()).map(String::from);
    let etag = header(ETAG);
    let last_modified = header(LAST_MODIFIED);
    let body = reply.body;
    let hash = hex::encode(Sha256::digest(&body));
    let pending = Pending {
        url: Some(url.to_string()),
        etag,
        last_modified,
        hash: hash.clone(),
    };
    if cached.as_ref().and_then(|c| c.2.as_deref()) == Some(hash.as_str()) {
        return Ok((Fetched::Unchanged, pending));
    }
    archive(ctx, source_id, url, &hash, &body);
    Ok((Fetched::Changed(body), pending))
}

/// GET without any caching (for endpoints whose query string changes every run).
pub async fn get_plain(ctx: &Ctx, url: &str) -> Result<Vec<u8>> {
    let reply = send_with_retry(url, || ctx.http.get(url)).await?;
    if !reply.status.is_success() {
        bail!("GET {url}: HTTP {}", reply.status);
    }
    Ok(reply.body)
}

/// HEAD request without caching, for probing whether a URL currently exists.
pub async fn head(ctx: &Ctx, url: &str) -> Result<StatusCode> {
    let reply = send_with_retry(url, || ctx.http.head(url)).await?;
    Ok(reply.status)
}

/// Pauses before each retry of a transient failure (so up to 3 attempts in total).
#[cfg(not(test))]
const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(3), Duration::from_secs(10)];
#[cfg(test)]
const RETRY_DELAYS: [Duration; 2] = [Duration::from_millis(10), Duration::from_millis(10)];

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

/// Sends the request built by `build`, retrying transient failures: DNS and connection
/// errors, timeouts, interrupted bodies, and HTTP 429/502/503/504. Other statuses are
/// returned to the caller as-is.
async fn send_with_retry(url: &str, build: impl Fn() -> reqwest::RequestBuilder) -> Result<Reply> {
    let mut attempt = 0;
    loop {
        let result = async {
            let resp = build().send().await?;
            let status = resp.status();
            let headers = resp.headers().clone();
            let body = resp.bytes().await?.to_vec();
            Ok::<_, reqwest::Error>(Reply { status, headers, body })
        }
        .await;
        let reason = match &result {
            Ok(r) if matches!(r.status.as_u16(), 429 | 502 | 503 | 504) => Some(format!("HTTP {}", r.status)),
            Ok(_) => None,
            Err(e) if e.is_connect() || e.is_timeout() || e.is_body() || e.is_request() => Some(format!("{e:#}")),
            Err(_) => None,
        };
        match reason {
            Some(reason) if attempt < RETRY_DELAYS.len() => {
                tracing::warn!(url, attempt = attempt + 1, %reason, "transient fetch failure; retrying");
                tokio::time::sleep(RETRY_DELAYS[attempt]).await;
                attempt += 1;
            }
            _ => return result.with_context(|| format!("GET {url}")),
        }
    }
}

pub struct Pending {
    url: Option<String>,
    etag: Option<String>,
    last_modified: Option<String>,
    hash: String,
}

impl Pending {
    fn none() -> Self {
        Pending { url: None, etag: None, last_modified: None, hash: String::new() }
    }

    pub fn commit(self, ctx: &Ctx) -> Result<()> {
        let Some(url) = self.url else { return Ok(()) };
        let conn = ctx.db.get()?;
        conn.execute(
            "INSERT INTO http_cache (url, etag, last_modified, content_hash, fetched_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(url) DO UPDATE SET etag=excluded.etag, last_modified=excluded.last_modified,
               content_hash=excluded.content_hash, fetched_at=excluded.fetched_at",
            params![url, self.etag, self.last_modified, self.hash, db::now()],
        )?;
        Ok(())
    }
}

/// Keeps a copy of every distinct payload for provenance and re-parsing.
fn archive(ctx: &Ctx, source_id: &str, url: &str, hash: &str, body: &[u8]) {
    let dir = ctx.cfg.data_dir.join("raw").join(source_id);
    let ext = url
        .split('?')
        .next()
        .and_then(|p| p.rsplit('/').next())
        .and_then(|f| f.rsplit_once('.').map(|(_, e)| e))
        .filter(|e| e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("bin");
    let name = format!("{}-{}.{}", chrono::Utc::now().format("%Y%m%dT%H%M"), &hash[..10], ext);
    if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(dir.join(name), body)) {
        tracing::warn!(source_id, error = %e, "could not archive payload");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Minimal HTTP server answering successive connections with the given statuses.
    fn serve(statuses: Vec<u16>) -> (String, std::thread::JoinHandle<usize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/x", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut served = 0;
            for status in statuses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf);
                let body = "hello";
                let reply = format!(
                    "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                stream.write_all(reply.as_bytes()).unwrap();
                served += 1;
            }
            served
        });
        (url, handle)
    }

    #[tokio::test]
    async fn retries_transient_status_then_succeeds() {
        let (url, server) = serve(vec![503, 502, 200]);
        let client = reqwest::Client::new();
        let reply = send_with_retry(&url, || client.get(&url)).await.unwrap();
        assert_eq!(reply.status, StatusCode::OK);
        assert_eq!(reply.body, b"hello");
        assert_eq!(server.join().unwrap(), 3);
    }

    #[tokio::test]
    async fn does_not_retry_client_errors() {
        let (url, server) = serve(vec![404]);
        let client = reqwest::Client::new();
        let reply = send_with_retry(&url, || client.get(&url)).await.unwrap();
        assert_eq!(reply.status, StatusCode::NOT_FOUND);
        assert_eq!(server.join().unwrap(), 1);
    }

    #[tokio::test]
    async fn gives_up_after_repeated_connection_errors() {
        // Bind and drop to get a port that refuses connections.
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{port}/x");
        let client = reqwest::Client::new();
        let err = send_with_retry(&url, || client.get(&url)).await.err().unwrap();
        assert!(format!("{err:#}").contains("GET http://127.0.0.1"));
    }
}
