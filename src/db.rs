use anyhow::{Context, Result};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;

pub type Pool = r2d2::Pool<SqliteConnectionManager>;

const MIGRATIONS: &[&str] = &[include_str!("../migrations/0001_init.sql")];

pub fn open(path: &Path) -> Result<Pool> {
    // Switch to WAL once, before the pool opens connections concurrently.
    Connection::open(path)?.execute_batch("PRAGMA journal_mode=WAL;")?;
    let manager = SqliteConnectionManager::file(path).with_init(|c| {
        c.execute_batch(
            "PRAGMA busy_timeout=5000;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;",
        )
    });
    let pool = r2d2::Pool::builder()
        .max_size(4)
        .build(manager)
        .context("opening sqlite pool")?;
    let mut conn = pool.get()?;
    migrate(&mut conn)?;
    Ok(pool)
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(current as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)
            .with_context(|| format!("migration {}", i + 1))?;
        tx.pragma_update(None, "user_version", i as i64 + 1)?;
        tx.commit()?;
        tracing::info!(version = i + 1, "applied migration");
    }
    Ok(())
}

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Inserts or replaces observations; returns how many rows actually changed.
pub fn upsert_series(
    conn: &mut Connection,
    series_id: &str,
    source_id: &str,
    unit: &str,
    freq: &str,
    points: &[(String, f64)],
) -> Result<usize> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO series (id, source_id, unit, freq) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET source_id=excluded.source_id, unit=excluded.unit, freq=excluded.freq",
        params![series_id, source_id, unit, freq],
    )?;
    let mut changed = 0;
    {
        let mut stmt = tx.prepare_cached(
            "INSERT INTO observation (series_id, t, value) VALUES (?1, ?2, ?3)
             ON CONFLICT(series_id, t) DO UPDATE SET value=excluded.value WHERE value != excluded.value",
        )?;
        for (t, v) in points {
            changed += stmt.execute(params![series_id, t, v])?;
        }
    }
    tx.commit()?;
    Ok(changed)
}

pub fn latest_point(conn: &Connection, series_id: &str) -> Result<Option<(String, f64)>> {
    Ok(conn
        .query_row(
            "SELECT t, value FROM observation WHERE series_id=?1 ORDER BY t DESC LIMIT 1",
            [series_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?)
}

pub fn point_at(conn: &Connection, series_id: &str, t: &str) -> Result<Option<f64>> {
    Ok(conn
        .query_row(
            "SELECT value FROM observation WHERE series_id=?1 AND t=?2",
            params![series_id, t],
            |r| r.get(0),
        )
        .optional()?)
}

pub fn series_points(conn: &Connection, series_id: &str, from: Option<&str>) -> Result<Vec<(String, f64)>> {
    let mut stmt = conn.prepare_cached(
        "SELECT t, value FROM observation WHERE series_id=?1 AND t >= ?2 ORDER BY t",
    )?;
    let rows = stmt
        .query_map(params![series_id, from.unwrap_or("0000")], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
