//! One background task per source: poll at its interval with jitter, back off on errors.

use crate::fetch::Ctx;
use crate::sources::{self, REGISTRY, SourceDef};
use rusqlite::OptionalExtension;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn spawn(ctx: Ctx) {
    for (i, def) in REGISTRY.iter().enumerate() {
        let ctx = ctx.clone();
        tokio::spawn(async move {
            // Stagger start-up so we don't hit every server in the same second.
            tokio::time::sleep(Duration::from_secs(2 + 3 * i as u64)).await;
            tokio::time::sleep(initial_wait(&ctx, def)).await;
            loop {
                let next = match sources::run_and_record(&ctx, def.id).await {
                    Ok(_) => Duration::from_secs(def.poll) + jitter(def.poll / 10),
                    Err(_) => backoff(&ctx, def),
                };
                tokio::time::sleep(next).await;
            }
        });
    }
}

/// Skip the immediate run when the last success is more recent than the poll interval.
fn initial_wait(ctx: &Ctx, def: &SourceDef) -> Duration {
    let last: Option<String> = ctx.db.get().ok().and_then(|c| {
        c.query_row("SELECT last_success FROM source_status WHERE id=?1", [def.id], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    });
    let Some(last) = last.and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok()) else {
        return Duration::ZERO;
    };
    let elapsed = (chrono::Utc::now() - last.to_utc()).num_seconds().max(0) as u64;
    Duration::from_secs(def.poll.saturating_sub(elapsed))
}

/// 5 min, 10, 20, ... capped at the source's poll interval.
fn backoff(ctx: &Ctx, def: &SourceDef) -> Duration {
    let fails: u32 = ctx
        .db
        .get()
        .ok()
        .and_then(|c| {
            c.query_row("SELECT fail_count FROM source_status WHERE id=?1", [def.id], |r| r.get(0))
                .ok()
        })
        .unwrap_or(1);
    let secs = 300u64.saturating_mul(1 << fails.saturating_sub(1).min(8));
    Duration::from_secs(secs.min(def.poll)) + jitter(60)
}

fn jitter(max_secs: u64) -> Duration {
    if max_secs == 0 {
        return Duration::ZERO;
    }
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().subsec_nanos() as u64;
    Duration::from_secs(nanos % max_secs)
}
