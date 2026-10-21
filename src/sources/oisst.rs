//! Daily NOAA OISST v2.1 (preliminary/NRT) SST anomalies from CoastWatch ERDDAP,
//! subsampled to 1°. Each day is stored as a compact binary grid (rendered to map tiles on demand),
//! and daily Niño 3.4 / Niño 1+2 means are derived from it.

use crate::db;
use crate::fetch::{self, Ctx};
use crate::grid::Grid;
use anyhow::{Context, Result};
use chrono::{Duration, NaiveDate};
use rusqlite::params;
use std::collections::HashSet;
use std::path::PathBuf;

const ERDDAP: &str = "https://coastwatch.pfeg.noaa.gov/erddap/griddap/ncdcOisst21NrtAgg_LonPM180";
pub const DATASET: &str = "oisst";
/// Frames downloaded per scheduled run; the backfill command loops until complete.
const PER_RUN: usize = 12;

pub fn grid_path(ctx: &Ctx, date: &str) -> PathBuf {
    ctx.cfg.data_dir.join("grids").join(DATASET).join(format!("{date}.bin"))
}

async fn latest_date(ctx: &Ctx) -> Result<NaiveDate> {
    let body = fetch::get_plain(ctx, &format!("{ERDDAP}.csv?time%5B(last)%5D")).await?;
    let text = String::from_utf8_lossy(&body);
    let last = text.lines().last().context("empty time response")?;
    NaiveDate::parse_from_str(last.get(..10).unwrap_or_default(), "%Y-%m-%d")
        .with_context(|| format!("unexpected time value {last:?}"))
}

fn frame_url(date: NaiveDate) -> String {
    // Stride 4 on the 0.25° grid gives 1° cells: 360 x 180.
    format!(
        "{ERDDAP}.esriAscii?anom%5B({date}T12:00:00Z)%5D%5B(0.0)%5D%5B(-89.875):4:(89.875)%5D%5B(-179.875):4:(179.875)%5D"
    )
}

async fn fetch_frame(ctx: &Ctx, date: NaiveDate) -> Result<Grid> {
    let body = fetch::get_plain(ctx, &frame_url(date)).await?;
    Grid::parse_esri_ascii(&String::from_utf8_lossy(&body)).with_context(|| format!("parsing OISST frame {date}"))
}

/// Downloads up to `limit` missing frames (newest first). Returns how many were stored.
pub async fn sync(ctx: &Ctx, limit: usize) -> Result<usize> {
    let latest = latest_date(ctx).await?;
    let first = latest - Duration::days(ctx.cfg.sst_days - 1);
    let have: HashSet<String> = {
        let conn = ctx.db.get()?;
        let mut stmt = conn.prepare("SELECT date FROM grid_frame WHERE dataset=?1")?;
        stmt.query_map([DATASET], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
    };
    let missing: Vec<NaiveDate> = first
        .iter_days()
        .take_while(|d| *d <= latest)
        .filter(|d| !have.contains(&d.to_string()))
        .collect();

    let mut stored = 0;
    for date in missing.into_iter().rev().take(limit) {
        // A failed frame ends this batch; frames already stored are kept and the
        // rest are retried on the next run.
        let grid = match fetch_frame(ctx, date).await {
            Ok(g) => g,
            Err(e) if stored > 0 => {
                tracing::warn!(%date, error = %format!("{e:#}"), "OISST frame failed; will retry");
                break;
            }
            Err(e) => return Err(e),
        };
        let d = date.to_string();
        grid.save(&grid_path(ctx, &d))?;

        let mut conn = ctx.db.get()?;
        let mut derived = Vec::new();
        if let Some(v) = grid.area_mean(-5.0, 5.0, -170.0, -120.0) {
            derived.push(("nino34_oisst", v));
        }
        if let Some(v) = grid.area_mean(-10.0, 0.0, -90.0, -80.0) {
            derived.push(("nino12_oisst", v));
        }
        for (id, v) in derived {
            let v = (v * 100.0).round() / 100.0;
            db::upsert_series(&mut conn, id, "oisst", "°C", "daily", &[(d.clone(), v)])?;
        }
        conn.execute(
            "INSERT OR REPLACE INTO grid_frame (dataset, date, created) VALUES (?1, ?2, ?3)",
            params![DATASET, d, db::now()],
        )?;
        stored += 1;
        // Be gentle with the shared ERDDAP server.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    prune(ctx, first)?;
    Ok(stored)
}

/// Removes frames older than the retention window (derived series are kept).
fn prune(ctx: &Ctx, first: NaiveDate) -> Result<()> {
    let conn = ctx.db.get()?;
    let old: Vec<String> = {
        let mut stmt = conn.prepare("SELECT date FROM grid_frame WHERE dataset=?1 AND date < ?2")?;
        stmt.query_map(params![DATASET, first.to_string()], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?
    };
    for d in old {
        std::fs::remove_file(grid_path(ctx, &d)).ok();
        conn.execute("DELETE FROM grid_frame WHERE dataset=?1 AND date=?2", params![DATASET, d])?;
    }
    Ok(())
}

pub async fn run(ctx: &Ctx) -> Result<bool> {
    Ok(sync(ctx, PER_RUN).await? > 0)
}

pub fn frame_dates(ctx: &Ctx) -> Result<Vec<String>> {
    let conn = ctx.db.get()?;
    let mut stmt = conn.prepare("SELECT date FROM grid_frame WHERE dataset=?1 ORDER BY date")?;
    let rows = stmt.query_map([DATASET], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}
