//! SENAMHI (Peru's national weather service) rain-gauge observations. SENAMHI has no
//! open API and its data tables sit behind a CAPTCHA, which we never touch. Instead we
//! read the 30-day daily chart that the station map embeds for each real-time
//! conventional station (map_red_graf.php), once a day and slowly. Stations are assigned
//! to departments through the CHIRPS mask, and each department gets a daily mean over
//! the stations that reported that day, plus a daily count of those stations.

use crate::db;
use crate::fetch::{self, Ctx};
use crate::sources::chirps;
use anyhow::{Context, Result, bail};
use chrono::{Duration, NaiveDate};
use regex::Regex;
use serde::Deserialize;
use std::sync::LazyLock;

pub const DATASET: &str = "senamhi";
const LIST_URL: &str = "https://www.senamhi.gob.pe/?p=estaciones";
const CHART_URL: &str = "https://www.senamhi.gob.pe/mapas/mapa-estaciones-ms/map_red_graf.php";
/// Pause between station requests: ~310 stations take about 20 minutes in all.
const PAUSE: std::time::Duration = std::time::Duration::from_secs(2);
/// Department means are recomputed for this many recent days on every run.
const RECOMPUTE_DAYS: i64 = 45;
/// Stations off the land mask snap to a department cell this many cells (0.05°) away.
const SNAP_CELLS: i64 = 4;
/// Daily totals above this are treated as data errors (Peru's records are ~250 mm).
const MAX_DAILY_MM: f64 = 500.0;

static CATEGORIES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"var categorias\s*=\s*(\[[^\]]*\])").unwrap());
static PRECIP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)name:\s*'Precipitaci[^']*',[^}]*?data:\s*(\[[^\]]*\])").unwrap()
});

#[derive(Debug, Clone, Deserialize)]
pub struct Station {
    pub cod: String,
    pub nom: String,
    pub lat: f64,
    pub lon: f64,
    /// M = meteorological, H = hydrological.
    pub ico: String,
    /// REAL (conventional, real time) | DIFERIDO (conventional, delayed) | AUTOMATICA.
    pub estado: String,
}

#[derive(Deserialize)]
struct MapData {
    puntos: Vec<Station>,
}

pub fn station_series_id(cod: &str) -> String {
    format!("senamhi_st_{cod}")
}

pub fn dept_series_id(dept: &chirps::Department) -> String {
    format!("senamhi_precip_{}", dept.code)
}

pub fn count_series_id(dept: &chirps::Department) -> String {
    format!("senamhi_n_{}", dept.code)
}

fn chart_url(cod: &str) -> String {
    format!("{CHART_URL}?cod={cod}")
}

/// Real-time conventional weather stations from the station map page, which prints the
/// whole network as the JSON argument of `MapaEstacionesMs.iniciar(element, {...})`.
pub fn parse_station_list(html: &str) -> Result<Vec<Station>> {
    let call = html
        .find("MapaEstacionesMs.iniciar(")
        .context("station map data not found")?;
    let start = call + html[call..].find('{').context("station map data has no object")?;
    let data: MapData = serde_json::Deserializer::from_str(&html[start..])
        .into_iter()
        .next()
        .context("station map data is empty")?
        .context("parsing station map data")?;
    Ok(data
        .puntos
        .into_iter()
        .filter(|s| s.ico == "M" && s.estado == "REAL")
        .collect())
}

/// Daily precipitation (date, mm) from a conventional station's 30-day chart. Missing
/// days are `null` in the chart and are skipped.
pub fn parse_daily(html: &str) -> Result<Vec<(String, f64)>> {
    let cats = CATEGORIES
        .captures(html)
        .context("chart categories not found")?;
    let dates: Vec<String> = serde_json::from_str(&cats[1]).context("parsing chart categories")?;
    // Automatic stations chart hourly values ("2026/10/27 - 09"), which we don't use.
    if let Some(bad) = dates
        .iter()
        .find(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").is_err())
    {
        bail!("not a daily chart (category {bad:?})");
    }
    let data = PRECIP
        .captures(html)
        .context("precipitation series not found")?;
    let values: Vec<Option<f64>> =
        serde_json::from_str(&data[1]).context("parsing precipitation series")?;
    if values.len() != dates.len() {
        bail!("{} precipitation values for {} dates", values.len(), dates.len());
    }
    Ok(dates
        .into_iter()
        .zip(values)
        .filter_map(|(d, v)| {
            let v = v.filter(|v| (0.0..=MAX_DAILY_MM).contains(v))?;
            Some((d, (v * 10.0).round() / 10.0))
        })
        .collect())
}

async fn fetch_station(ctx: &Ctx, cod: &str) -> Result<Vec<(String, f64)>> {
    let body = fetch::get_plain(ctx, &chart_url(cod)).await?;
    parse_daily(&String::from_utf8_lossy(&body)).with_context(|| format!("station {cod}"))
}

pub async fn run(ctx: &Ctx) -> Result<bool> {
    let body = fetch::get_plain(ctx, LIST_URL).await?;
    let stations = parse_station_list(&String::from_utf8_lossy(&body))?;
    if stations.is_empty() {
        bail!("no real-time stations in the SENAMHI station list");
    }
    let depts = chirps::departments();
    let mut members: Vec<Vec<String>> = vec![Vec::new(); depts.len()];
    let mut changed = 0;
    let mut failed = 0;
    for st in &stations {
        let Some(dept) = chirps::department_at(st.lat, st.lon, SNAP_CELLS) else {
            tracing::debug!(cod = st.cod, name = st.nom, "SENAMHI station outside Peru mask");
            continue;
        };
        let idx = depts.iter().position(|d| d.code == dept.code).expect("department in list");
        let id = station_series_id(&st.cod);
        // A station that fails this time still counts with the days we already have.
        members[idx].push(id.clone());
        tokio::time::sleep(PAUSE).await;
        match fetch_station(ctx, &st.cod).await {
            Ok(points) => {
                let mut conn = ctx.db.get()?;
                changed += db::upsert_series(&mut conn, &id, DATASET, "mm", "daily", &points)?;
            }
            Err(e) => {
                failed += 1;
                tracing::warn!(cod = st.cod, name = st.nom, error = %format!("{e:#}"), "SENAMHI station failed");
            }
        }
    }
    if failed * 2 > stations.len() {
        bail!("{failed} of {} SENAMHI stations failed", stations.len());
    }

    let since = (chrono::Utc::now().date_naive() - Duration::days(RECOMPUTE_DAYS)).to_string();
    let mut conn = ctx.db.get()?;
    for (dept, ids) in depts.iter().zip(&members) {
        if ids.is_empty() {
            continue;
        }
        let (means, counts) = department_daily(&conn, ids, &since)?;
        changed += db::upsert_series(&mut conn, &dept_series_id(dept), DATASET, "mm", "daily", &means)?;
        db::upsert_series(&mut conn, &count_series_id(dept), DATASET, "stations", "daily", &counts)?;
    }
    tracing::info!(stations = stations.len(), failed, "SENAMHI stations read");
    Ok(changed > 0)
}

type Points = Vec<(String, f64)>;

/// Per day since `since`: mean precipitation over the given station series that have a
/// value that day, and how many of them do.
fn department_daily(conn: &rusqlite::Connection, ids: &[String], since: &str) -> Result<(Points, Points)> {
    let marks = vec!["?"; ids.len()].join(",");
    let mut stmt = conn.prepare(&format!(
        "SELECT t, AVG(value), COUNT(*) FROM observation
         WHERE series_id IN ({marks}) AND t >= ? GROUP BY t ORDER BY t"
    ))?;
    let params = ids.iter().map(String::as_str).chain([since]);
    let rows: Vec<(String, f64, i64)> = stmt
        .query_map(rusqlite::params_from_iter(params), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows
        .into_iter()
        .map(|(t, mean, n)| ((t.clone(), (mean * 100.0).round() / 100.0), (t, n as f64)))
        .unzip())
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(&path).expect("fixture present")
    }

    #[test]
    fn parses_station_list() {
        let stations = parse_station_list(&fixture("senamhi_estaciones.html")).unwrap();
        assert!(stations.len() > 250, "{}", stations.len());
        assert!(stations.iter().all(|s| s.ico == "M" && s.estado == "REAL"));
        let acostambo = stations.iter().find(|s| s.cod == "112051").unwrap();
        assert_eq!(acostambo.nom, "ACOSTAMBO");
        assert!((acostambo.lat + 12.36079).abs() < 1e-4);
        // Nearly every real-time station lands in a department, coastal ones included.
        let placed = stations
            .iter()
            .filter(|s| chirps::department_at(s.lat, s.lon, SNAP_CELLS).is_some())
            .count();
        assert!(placed * 100 >= stations.len() * 98, "{placed} of {}", stations.len());
        assert_eq!(
            chirps::department_at(acostambo.lat, acostambo.lon, SNAP_CELLS).map(|d| d.code.as_str()),
            Some("huancavelica")
        );
    }

    #[test]
    fn parses_daily_chart() {
        let points = parse_daily(&fixture("senamhi_graf_daily_112051.html")).unwrap();
        // 30 categories, the last (today) still null.
        assert_eq!(points.len(), 29);
        assert_eq!(points[0], ("2026-09-27".to_string(), 0.0));
        assert_eq!(points[2], ("2026-09-29".to_string(), 3.0));
        assert!(points.contains(&("2026-10-20".to_string(), 11.0)), "{points:?}");
        // The station sent nothing on 23 Oct: the chart simply skips the date.
        assert!(!points.iter().any(|(d, _)| d == "2026-10-23"));
        assert!(points.iter().all(|(_, v)| *v >= 0.0));
    }

    #[test]
    fn rejects_hourly_chart() {
        let err = parse_daily(&fixture("senamhi_graf_hourly_105112.html")).unwrap_err();
        assert!(format!("{err:#}").contains("not a daily chart"), "{err:#}");
    }

    #[test]
    fn department_means_and_counts() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../../migrations/0001_init.sql")).unwrap();
        let day = |d: &str, v: f64| (d.to_string(), v);
        db::upsert_series(&mut conn, "a", DATASET, "mm", "daily", &[day("2026-10-01", 2.0), day("2026-10-02", 0.0)]).unwrap();
        db::upsert_series(&mut conn, "b", DATASET, "mm", "daily", &[day("2026-10-01", 5.0)]).unwrap();
        db::upsert_series(&mut conn, "c", DATASET, "mm", "daily", &[day("2026-10-01", 99.0)]).unwrap();
        let ids = ["a".to_string(), "b".to_string()];
        let (means, counts) = department_daily(&conn, &ids, "2026-09-01").unwrap();
        assert_eq!(means, vec![day("2026-10-01", 3.5), day("2026-10-02", 0.0)]);
        assert_eq!(counts, vec![day("2026-10-01", 2.0), day("2026-10-02", 1.0)]);
    }
}
