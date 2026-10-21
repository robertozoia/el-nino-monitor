//! Shared HTTP layer: conditional GETs, content hashing and raw archiving.

use crate::config::Config;
use crate::db::{self, Pool};
use anyhow::{Context, Result, bail};
use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
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
    let mut req = ctx.http.get(url);
    if let Some((etag, lm, _)) = &cached {
        if let Some(e) = etag {
            req = req.header(IF_NONE_MATCH, e);
        }
        if let Some(l) = lm {
            req = req.header(IF_MODIFIED_SINCE, l);
        }
    }
    let resp = req.send().await.with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_MODIFIED {
        return Ok((Fetched::Unchanged, Pending::none()));
    }
    if !status.is_success() {
        bail!("GET {url}: HTTP {status}");
    }
    let header = |h| resp.headers().get(h).and_then(|v: &reqwest::header::HeaderValue| v.to_str().ok()).map(String::from);
    let etag = header(ETAG);
    let last_modified = header(LAST_MODIFIED);
    let body = resp.bytes().await?.to_vec();
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
    let resp = ctx.http.get(url).send().await.with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        bail!("GET {url}: HTTP {status}");
    }
    Ok(resp.bytes().await?.to_vec())
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
