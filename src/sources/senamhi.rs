//! SENAMHI (Peru's national weather service) rain-gauge observations. SENAMHI has no
//! open API and its data tables sit behind a CAPTCHA, which we never touch. Instead we
//! read the chart that the station map embeds for each station (map_red_graf.php), once
//! a day and slowly:
//!
//! - real-time conventional stations chart 30 days of daily totals;
//! - automatic stations chart the last 48 hours, hourly. We keep the hours for a few
//!   days and add them up into SENAMHI's climatological day (07:00 to 07:00 local time,
//!   labelled with the first date, as in the conventional series), counting a day only
//!   when all 24 hours reported.
//!
//! Automatic stations next to a conventional one are skipped so a site is not counted
//! twice. Stations are assigned to departments through the CHIRPS mask, and each
//! department gets a daily mean over the stations that reported that day, plus a daily
//! count of those stations.

use crate::db;
use crate::fetch::{self, Ctx};
use crate::sources::chirps;
use anyhow::{Context, Result, bail};
use chrono::{Duration, NaiveDate, NaiveDateTime};
use regex::Regex;
use rusqlite::params;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::LazyLock;

pub const DATASET: &str = "senamhi";
const LIST_URL: &str = "https://www.senamhi.gob.pe/?p=estaciones";
const CHART_URL: &str = "https://www.senamhi.gob.pe/mapas/mapa-estaciones-ms/map_red_graf.php";
/// Pause between station requests: ~450 stations take about 30 minutes in all.
const PAUSE: std::time::Duration = std::time::Duration::from_secs(2);
/// Department means are recomputed for this many recent days on every run.
const RECOMPUTE_DAYS: i64 = 45;
/// Hourly readings from automatic stations are kept this long (days).
const HOURLY_KEEP_DAYS: i64 = 10;
/// Stations off the land mask snap to a department cell this many cells (0.05°) away.
const SNAP_CELLS: i64 = 4;
/// An automatic station this close to a conventional one is the same site (km).
const TWIN_KM: f64 = 3.5;
/// Daily totals above this are treated as data errors (Peru's records are ~250 mm).
const MAX_DAILY_MM: f64 = 500.0;
const MAX_HOURLY_MM: f64 = 200.0;
/// SENAMHI's day runs from 07:00 local time; hourly labels mark the end of each hour,
/// so the hours labelled 08:00 on day D through 07:00 on D+1 make up day D.
const DAY_START_HOUR: i64 = 8;

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

impl Station {
    fn is_automatic(&self) -> bool {
        self.estado == "AUTOMATICA"
    }
}

#[derive(Deserialize)]
struct MapData {
    puntos: Vec<Station>,
}

pub fn station_series_id(cod: &str) -> String {
    format!("senamhi_st_{cod}")
}

fn hourly_series_id(cod: &str) -> String {
    format!("senamhi_hr_{cod}")
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

fn distance_km(a: &Station, b: &Station) -> f64 {
    let dx = (a.lon - b.lon) * a.lat.to_radians().cos();
    111.2 * dx.hypot(a.lat - b.lat)
}

/// Weather stations we read from the station map page, which prints the whole network
/// as the JSON argument of `MapaEstacionesMs.iniciar(element, {...})`: real-time
/// conventional stations, plus automatic ones that are not next to one of those.
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
    let (conventional, automatic): (Vec<Station>, Vec<Station>) = data
        .puntos
        .into_iter()
        .filter(|s| s.ico == "M" && matches!(s.estado.as_str(), "REAL" | "AUTOMATICA"))
        .partition(|s| !s.is_automatic());
    let lone: Vec<Station> = automatic
        .into_iter()
        .filter(|a| conventional.iter().all(|c| distance_km(a, c) > TWIN_KM))
        .collect();
    Ok(conventional.into_iter().chain(lone).collect())
}

/// Precipitation from a station chart: daily totals (conventional stations) or hourly
/// amounts (automatic stations). Missing values are `null` in the chart and are skipped.
#[derive(Debug)]
pub enum Chart {
    Daily(Points),
    Hourly(Vec<(NaiveDateTime, f64)>),
}

pub fn parse_chart(html: &str) -> Result<Chart> {
    let cats = CATEGORIES
        .captures(html)
        .context("chart categories not found")?;
    let labels: Vec<String> = serde_json::from_str(&cats[1]).context("parsing chart categories")?;
    let data = PRECIP
        .captures(html)
        .context("precipitation series not found")?;
    let values: Vec<Option<f64>> =
        serde_json::from_str(&data[1]).context("parsing precipitation series")?;
    if values.len() != labels.len() {
        bail!("{} precipitation values for {} categories", values.len(), labels.len());
    }
    let pairs = labels.iter().zip(values);
    let round = |v: f64| (v * 10.0).round() / 10.0;
    // Daily charts label days "2026-10-27"; hourly charts label hours "2026/10/27 - 09".
    if labels
        .iter()
        .all(|l| NaiveDate::parse_from_str(l, "%Y-%m-%d").is_ok())
    {
        return Ok(Chart::Daily(
            pairs
                .filter_map(|(l, v)| {
                    let v = v.filter(|v| (0.0..=MAX_DAILY_MM).contains(v))?;
                    Some((l.clone(), round(v)))
                })
                .collect(),
        ));
    }
    let mut hours = Vec::new();
    for (l, v) in pairs {
        let t = NaiveDateTime::parse_from_str(&format!("{l}:00"), "%Y/%m/%d - %H:%M")
            .with_context(|| format!("unexpected chart category {l:?}"))?;
        if let Some(v) = v.filter(|v| (0.0..=MAX_HOURLY_MM).contains(v)) {
            hours.push((t, round(v)));
        }
    }
    Ok(Chart::Hourly(hours))
}

/// Adds hourly amounts up into SENAMHI days, keeping only days with all 24 hours.
pub fn daily_from_hourly(hours: &[(NaiveDateTime, f64)]) -> Points {
    let mut days: BTreeMap<NaiveDate, (f64, usize)> = BTreeMap::new();
    for (t, v) in hours {
        let day = (*t - Duration::hours(DAY_START_HOUR)).date();
        let e = days.entry(day).or_default();
        e.0 += v;
        e.1 += 1;
    }
    days.into_iter()
        .filter(|(_, (_, n))| *n == 24)
        .map(|(d, (sum, _))| (d.to_string(), (sum * 10.0).round() / 10.0))
        .collect()
}

const HOUR_FMT: &str = "%Y-%m-%dT%H";

/// Stores a station's chart; returns how many daily values changed.
fn store_chart(conn: &mut rusqlite::Connection, cod: &str, chart: Chart) -> Result<usize> {
    let daily = match chart {
        Chart::Daily(points) => points,
        Chart::Hourly(hours) => {
            let hr_id = hourly_series_id(cod);
            let pts: Points = hours
                .iter()
                .map(|(t, v)| (t.format(HOUR_FMT).to_string(), *v))
                .collect();
            db::upsert_series(conn, &hr_id, DATASET, "mm", "hourly", &pts)?;
            // Days can straddle two runs, so add up everything we have kept.
            let mut stmt = conn.prepare("SELECT t, value FROM observation WHERE series_id=?1")?;
            let kept: Vec<(String, f64)> = stmt
                .query_map([&hr_id], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            let kept: Vec<(NaiveDateTime, f64)> = kept
                .into_iter()
                .filter_map(|(t, v)| Some((NaiveDateTime::parse_from_str(&format!("{t}:00"), "%Y-%m-%dT%H:%M").ok()?, v)))
                .collect();
            daily_from_hourly(&kept)
        }
    };
    db::upsert_series(conn, &station_series_id(cod), DATASET, "mm", "daily", &daily)
}

async fn fetch_station(ctx: &Ctx, cod: &str) -> Result<Chart> {
    let body = fetch::get_plain(ctx, &chart_url(cod)).await?;
    parse_chart(&String::from_utf8_lossy(&body)).with_context(|| format!("station {cod}"))
}

pub async fn run(ctx: &Ctx) -> Result<bool> {
    let body = fetch::get_plain(ctx, LIST_URL).await?;
    let stations = parse_station_list(&String::from_utf8_lossy(&body))?;
    if stations.is_empty() {
        bail!("no stations in the SENAMHI station list");
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
        ctx.db.get()?.execute(
            "INSERT INTO station (id, source_id, code, name, kind, dept, lat, lon, updated)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, kind=excluded.kind, dept=excluded.dept,
               lat=excluded.lat, lon=excluded.lon, updated=excluded.updated",
            params![
                id,
                DATASET,
                st.cod,
                st.nom,
                if st.is_automatic() { "automatic" } else { "conventional" },
                dept.code,
                st.lat,
                st.lon,
                db::now()
            ],
        )?;
        // A station that fails this time still counts with the days we already have.
        members[idx].push(id);
        tokio::time::sleep(PAUSE).await;
        match fetch_station(ctx, &st.cod).await {
            Ok(chart) => {
                let mut conn = ctx.db.get()?;
                changed += store_chart(&mut conn, &st.cod, chart)?;
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

    let today = chrono::Utc::now().date_naive();
    let since = (today - Duration::days(RECOMPUTE_DAYS)).to_string();
    let mut conn = ctx.db.get()?;
    for (dept, ids) in depts.iter().zip(&members) {
        if ids.is_empty() {
            continue;
        }
        let (means, counts) = department_daily(&conn, ids, &since)?;
        changed += db::upsert_series(&mut conn, &dept_series_id(dept), DATASET, "mm", "daily", &means)?;
        db::upsert_series(&mut conn, &count_series_id(dept), DATASET, "stations", "daily", &counts)?;
    }
    conn.execute(
        "DELETE FROM observation WHERE series_id LIKE 'senamhi_hr_%' AND t < ?1",
        [(today - Duration::days(HOURLY_KEEP_DAYS)).to_string()],
    )?;
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

    fn hour(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M").unwrap()
    }

    #[test]
    fn parses_station_list() {
        let stations = parse_station_list(&fixture("senamhi_estaciones.html")).unwrap();
        let conventional = stations.iter().filter(|s| !s.is_automatic()).count();
        let automatic = stations.len() - conventional;
        assert!(conventional > 250, "{conventional}");
        assert!((100..216).contains(&automatic), "{automatic}");
        assert!(stations.iter().all(|s| s.ico == "M" && s.estado != "DIFERIDO"));
        let acostambo = stations.iter().find(|s| s.cod == "112051").unwrap();
        assert_eq!(acostambo.nom, "ACOSTAMBO");
        assert!((acostambo.lat + 12.36079).abs() < 1e-4);
        // Paimas (automatic only) is read; Huarmaca's automatic twin of the conventional
        // gauge 0.6 km away is not.
        assert!(stations.iter().any(|s| s.nom == "PAIMAS" && s.is_automatic()));
        assert!(stations.iter().any(|s| s.cod == "105064"));
        assert!(!stations.iter().any(|s| s.cod == "472761E6"));
        // Nearly every station lands in a department, coastal ones included.
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
        let Chart::Daily(points) = parse_chart(&fixture("senamhi_graf_daily_112051.html")).unwrap() else {
            panic!("expected a daily chart");
        };
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
    fn parses_hourly_chart() {
        let Chart::Hourly(hours) = parse_chart(&fixture("senamhi_graf_hourly_105112.html")).unwrap() else {
            panic!("expected an hourly chart");
        };
        assert_eq!(hours.len(), 48);
        assert_eq!(hours[0], (hour("2026-10-27 09:00"), 0.2));
        assert!(hours.contains(&(hour("2026-10-28 00:00"), 2.6)));
        // 27 Oct 07:00 to 28 Oct 07:00 is incomplete (the chart starts at 09:00);
        // 28 Oct 07:00 to 29 Oct 07:00 is complete.
        assert_eq!(daily_from_hourly(&hours), vec![("2026-10-28".to_string(), 0.0)]);
    }

    #[test]
    fn adds_hours_into_senamhi_days() {
        let mut hours: Vec<(NaiveDateTime, f64)> = (0..24)
            .map(|h| (hour("2026-10-28 08:00") + Duration::hours(h), 1.0))
            .collect();
        // One hour of the next day: still incomplete.
        hours.push((hour("2026-10-29 08:00"), 5.0));
        assert_eq!(daily_from_hourly(&hours), vec![("2026-10-28".to_string(), 24.0)]);
        hours.remove(3);
        assert!(daily_from_hourly(&hours).is_empty());
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
