mod config;
mod db;
mod fetch;
mod grid;
mod scheduler;
mod sources;
mod web;

use anyhow::{Context, Result, bail};
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

const USAGE: &str = "usage:
  elnino serve                      start web server and background fetchers (default)
  elnino run <source|all>           run fetchers once and exit
  elnino backfill                   fetch everything, including the full SST frame window
  elnino add-link <org> <title> <url> [YYYY-MM-DD]
                                    add a curated publication link (WMO, BoM, ...)
  elnino sources                    list source ids";

/// Runs a frame source's `sync` in batches until complete. An unreachable provider
/// must not abort the whole deployment: after a few failed attempts we log and move
/// on, leaving the scheduler to retry with backoff.
async fn backfill_frames(ctx: &fetch::Ctx, source: &str, per_batch: usize) -> Result<()> {
    let sync = |ctx: &fetch::Ctx, n: usize| {
        let ctx = ctx.clone();
        async move {
            match source {
                "oisst" => sources::oisst::sync(&ctx, n).await,
                "chirps" => sources::chirps::sync(&ctx, n).await,
                _ => unreachable!("backfill_frames: {source}"),
            }
        }
    };
    let mut failures = 0;
    loop {
        match sync(ctx, per_batch).await {
            Ok(0) => break,
            Ok(n) => {
                failures = 0;
                tracing::info!(source, frames = n, "backfill batch");
            }
            Err(e) if failures < 5 => {
                failures += 1;
                tracing::warn!(source, error = %format!("{e:#}"), "backfill error; retrying in 30 s");
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
            Err(e) => {
                // Records the failure in source_status like a scheduled run would.
                let _ = sources::run_and_record(ctx, source).await;
                tracing::error!(source, error = %format!("{e:#}"), "backfill gave up; the scheduler will keep retrying");
                break;
            }
        }
    }
    // Records the source status like a scheduled run would.
    sources::run_and_record(ctx, source).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,tower_http=warn")))
        .init();

    let cfg = Arc::new(config::Config::from_env());
    std::fs::create_dir_all(&cfg.data_dir).with_context(|| format!("creating {}", cfg.data_dir.display()))?;
    let pool = db::open(&cfg.data_dir.join("elnino.db"))?;
    let ctx = fetch::Ctx::new(cfg.clone(), pool)?;

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str).unwrap_or("serve") {
        "serve" => serve(ctx).await,
        "run" => {
            let target = args.get(1).context(USAGE)?;
            let ids: Vec<&str> = if target == "all" {
                sources::REGISTRY.iter().map(|d| d.id).collect()
            } else {
                vec![target.as_str()]
            };
            let mut failed = false;
            for id in ids {
                if sources::def(id).is_none() {
                    bail!("unknown source {id}\n{USAGE}");
                }
                failed |= sources::run_and_record(&ctx, id).await.is_err();
            }
            if failed {
                bail!("one or more sources failed (see log)");
            }
            Ok(())
        }
        "backfill" => {
            for d in sources::REGISTRY.iter().filter(|d| !matches!(d.id, "oisst" | "chirps")) {
                let _ = sources::run_and_record(&ctx, d.id).await;
            }
            backfill_frames(&ctx, "oisst", 30).await?;
            backfill_frames(&ctx, "chirps", 30).await?;
            Ok(())
        }
        "add-link" => {
            let (Some(org), Some(title), Some(url)) = (args.get(1), args.get(2), args.get(3)) else {
                bail!(USAGE);
            };
            let conn = ctx.db.get()?;
            conn.execute(
                "INSERT INTO link (org, title, url, issued) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(url) DO UPDATE SET org=excluded.org, title=excluded.title, issued=excluded.issued",
                rusqlite::params![org, title, url, args.get(4)],
            )?;
            println!("saved");
            Ok(())
        }
        "sources" => {
            for d in sources::REGISTRY {
                println!("{:<14} {:<40} every {}h", d.id, d.org, d.poll as f64 / 3600.0);
            }
            Ok(())
        }
        _ => bail!(USAGE),
    }
}

async fn serve(ctx: fetch::Ctx) -> Result<()> {
    if ctx.cfg.scheduler {
        scheduler::spawn(ctx.clone());
    }
    let app = web::router(ctx.clone())?;
    let listener = tokio::net::TcpListener::bind(&ctx.cfg.bind)
        .await
        .with_context(|| format!("binding {}", ctx.cfg.bind))?;
    tracing::info!(bind = %ctx.cfg.bind, "listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let ctrl_c = tokio::signal::ctrl_c();
            #[cfg(unix)]
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).unwrap();
            #[cfg(unix)]
            tokio::select! { _ = ctrl_c => {}, _ = term.recv() => {} }
            #[cfg(not(unix))]
            ctrl_c.await.ok();
            tracing::info!("shutting down");
        })
        .await?;
    Ok(())
}
