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
//!
//! Where SENAMHI is unreachable (it drops connections from some server networks), a
//! relay on another network runs the fetch, writes `export` to a JSON file and copies
//! it over; with `SENAMHI_INBOX` set, `run` imports that file instead of fetching.

use crate::db;
use crate::fetch::{self, Ctx};
use crate::sources::chirps;
use anyhow::{Context, Result, bail};
use chrono::{Duration, NaiveDate, NaiveDateTime};
use regex::Regex;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::LazyLock;

pub const DATASET: &str = "senamhi";
const LIST_URL: &str = "https://www.senamhi.gob.pe/?p=estaciones";
const CHART_URL: &str = "https://www.senamhi.gob.pe/mapas/mapa-estaciones-ms/map_red_graf.php";
/// Pause between station requests: ~450 stations take about 30 minutes in all.
const PAUSE: std::time::Duration = std::time::Duration::from_secs(2);
/// Department means are recomputed for this many recent days on every run.
pub const RECOMPUTE_DAYS: i64 = 45;
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

fn upsert_station(conn: &rusqlite::Connection, st: &ExportStation) -> Result<()> {
    conn.execute(
        "INSERT INTO station (id, source_id, code, name, kind, dept, lat, lon, updated)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET name=excluded.name, kind=excluded.kind, dept=excluded.dept,
           lat=excluded.lat, lon=excluded.lon, updated=excluded.updated",
        params![st.id, DATASET, st.code, st.name, st.kind, st.dept, st.lat, st.lon, db::now()],
    )?;
    Ok(())
}

pub async fn run(ctx: &Ctx) -> Result<bool> {
    if let Some(path) = &ctx.cfg.senamhi_inbox {
        return run_import(ctx, path);
    }
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
        let station = ExportStation {
            id: id.clone(),
            code: st.cod.clone(),
            name: st.nom.clone(),
            kind: if st.is_automatic() { "automatic" } else { "conventional" }.to_string(),
            dept: dept.code.clone(),
            lat: st.lat,
            lon: st.lon,
        };
        upsert_station(&*ctx.db.get()?, &station)?;
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

// ---------------------------------------------------------------- relay export / import

const EXPORT_VERSION: u32 = 1;
/// An imported export older than this means the relay has stopped.
const EXPORT_MAX_AGE_DAYS: i64 = 3;
const MAX_STATIONS_PER_DAY: f64 = 2000.0;

#[derive(Debug, Serialize, Deserialize)]
pub struct Export {
    pub version: u32,
    pub generated: String,
    pub stations: Vec<ExportStation>,
    pub series: Vec<ExportSeries>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportStation {
    pub id: String,
    pub code: String,
    pub name: String,
    pub kind: String,
    pub dept: String,
    pub lat: f64,
    pub lon: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSeries {
    pub id: String,
    pub unit: String,
    pub points: Points,
}

/// Stations and daily series (station, department mean, station count) since `since`.
/// Hourly readings stay with the fetcher.
pub fn export(conn: &rusqlite::Connection, since: &str) -> Result<Export> {
    let mut stmt = conn.prepare(
        "SELECT id, code, name, kind, COALESCE(dept, ''), lat, lon FROM station WHERE source_id=?1 ORDER BY id",
    )?;
    let stations = stmt
        .query_map([DATASET], |r| {
            Ok(ExportStation {
                id: r.get(0)?,
                code: r.get(1)?,
                name: r.get(2)?,
                kind: r.get(3)?,
                dept: r.get(4)?,
                lat: r.get(5)?,
                lon: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut stmt = conn.prepare(
        "SELECT o.series_id, s.unit, o.t, o.value FROM observation o JOIN series s ON s.id = o.series_id
         WHERE (o.series_id GLOB 'senamhi_st_*' OR o.series_id GLOB 'senamhi_precip_*'
                OR o.series_id GLOB 'senamhi_n_*') AND o.t >= ?1
         ORDER BY o.series_id, o.t",
    )?;
    let rows = stmt.query_map([since], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, f64>(3)?))
    })?;
    let mut series: Vec<ExportSeries> = Vec::new();
    for row in rows {
        let (id, unit, t, v) = row?;
        match series.last_mut() {
            Some(s) if s.id == id => s.points.push((t, v)),
            _ => series.push(ExportSeries { id, unit, points: vec![(t, v)] }),
        }
    }
    Ok(Export { version: EXPORT_VERSION, generated: db::now(), stations, series })
}

/// Checks an export from the relay before anything is written: it arrives over the
/// network, so every id, date and value must look like what `export` produces.
fn validate(e: &Export) -> Result<()> {
    if e.version != EXPORT_VERSION {
        bail!("unsupported SENAMHI export version {}", e.version);
    }
    chrono::DateTime::parse_from_rfc3339(&e.generated).context("export timestamp")?;
    let dept_ok = |code: &str| chirps::departments().iter().any(|d| d.code == code);
    let code_ok = |c: &str| !c.is_empty() && c.len() <= 16 && c.chars().all(|c| c.is_ascii_alphanumeric());
    for st in &e.stations {
        let ok = st.id.strip_prefix("senamhi_st_") == Some(st.code.as_str())
            && code_ok(&st.code)
            && matches!(st.kind.as_str(), "conventional" | "automatic")
            && dept_ok(&st.dept)
            && !st.name.is_empty()
            && st.name.len() <= 80
            && (-20.0..=1.0).contains(&st.lat)
            && (-82.0..=-68.0).contains(&st.lon);
        if !ok {
            bail!("invalid station in SENAMHI export: {st:?}");
        }
    }
    for s in &e.series {
        let max = if let Some(code) = s.id.strip_prefix("senamhi_st_") {
            code_ok(code).then_some(MAX_DAILY_MM)
        } else if let Some(dept) = s.id.strip_prefix("senamhi_precip_") {
            dept_ok(dept).then_some(MAX_DAILY_MM)
        } else if let Some(dept) = s.id.strip_prefix("senamhi_n_") {
            dept_ok(dept).then_some(MAX_STATIONS_PER_DAY)
        } else {
            None
        };
        let unit = if s.id.starts_with("senamhi_n_") { "stations" } else { "mm" };
        let Some(max) = max.filter(|_| s.unit == unit) else {
            bail!("invalid series {:?} ({}) in SENAMHI export", s.id, s.unit);
        };
        for (t, v) in &s.points {
            if NaiveDate::parse_from_str(t, "%Y-%m-%d").is_err() || !(0.0..=max).contains(v) {
                bail!("invalid point ({t:?}, {v}) in SENAMHI export series {}", s.id);
            }
        }
    }
    Ok(())
}

/// Writes a validated export into the database; returns how many values changed.
/// Re-importing the same file changes nothing.
pub fn import(conn: &mut rusqlite::Connection, e: &Export) -> Result<usize> {
    validate(e)?;
    for st in &e.stations {
        upsert_station(conn, st)?;
    }
    let mut changed = 0;
    for s in &e.series {
        changed += db::upsert_series(conn, &s.id, DATASET, &s.unit, "daily", &s.points)?;
    }
    Ok(changed)
}

pub fn read_export(path: &std::path::Path) -> Result<Export> {
    let body = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_slice(&body).with_context(|| format!("parsing {}", path.display()))
}

fn run_import(ctx: &Ctx, path: &std::path::Path) -> Result<bool> {
    if !path.exists() {
        bail!("no SENAMHI export received yet at {}", path.display());
    }
    let e = read_export(path)?;
    let generated = chrono::DateTime::parse_from_rfc3339(&e.generated).context("export timestamp")?;
    let age = chrono::Utc::now() - generated.to_utc();
    if age > Duration::days(EXPORT_MAX_AGE_DAYS) {
        bail!("SENAMHI export is {} days old (relay down?)", age.num_days());
    }
    let mut conn = ctx.db.get()?;
    let changed = import(&mut conn, &e)?;
    tracing::info!(generated = e.generated, stations = e.stations.len(), changed, "SENAMHI export imported");
    Ok(changed > 0)
}

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

    fn test_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!("../../migrations/0001_init.sql")).unwrap();
        conn.execute_batch(include_str!("../../migrations/0002_station.sql")).unwrap();
        conn
    }

    fn sample_db() -> rusqlite::Connection {
        let mut conn = test_db();
        let st = |code: &str, name: &str, kind: &str| ExportStation {
            id: station_series_id(code),
            code: code.into(),
            name: name.into(),
            kind: kind.into(),
            dept: "piura".into(),
            lat: -5.56,
            lon: -79.52,
        };
        upsert_station(&conn, &st("105064", "HUARMACA", "conventional")).unwrap();
        upsert_station(&conn, &st("472FC5B8", "PAIMAS", "automatic")).unwrap();
        let day = |d: &str, v: f64| (d.to_string(), v);
        db::upsert_series(&mut conn, "senamhi_st_105064", DATASET, "mm", "daily", &[day("2026-10-27", 17.5), day("2026-10-28", 72.0)]).unwrap();
        db::upsert_series(&mut conn, "senamhi_st_472FC5B8", DATASET, "mm", "daily", &[day("2026-10-28", 0.7)]).unwrap();
        db::upsert_series(&mut conn, "senamhi_precip_piura", DATASET, "mm", "daily", &[day("2026-10-28", 36.35)]).unwrap();
        db::upsert_series(&mut conn, "senamhi_n_piura", DATASET, "stations", "daily", &[day("2026-10-28", 2.0)]).unwrap();
        // Hourly readings and old days stay behind.
        db::upsert_series(&mut conn, "senamhi_hr_472FC5B8", DATASET, "mm", "hourly", &[day("2026-10-28T09", 0.3)]).unwrap();
        db::upsert_series(&mut conn, "senamhi_st_105064", DATASET, "mm", "daily", &[day("2026-08-01", 1.0)]).unwrap();
        conn
    }

    #[test]
    fn export_import_round_trip() {
        let e = export(&sample_db(), "2026-09-14").unwrap();
        assert_eq!(e.stations.len(), 2);
        let ids: Vec<&str> = e.series.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["senamhi_n_piura", "senamhi_precip_piura", "senamhi_st_105064", "senamhi_st_472FC5B8"]);
        let huarmaca = e.series.iter().find(|s| s.id == "senamhi_st_105064").unwrap();
        assert_eq!(huarmaca.points, vec![("2026-10-27".to_string(), 17.5), ("2026-10-28".to_string(), 72.0)]);

        // Through JSON, as the relay sends it, into an empty database.
        let e: Export = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        let mut conn = test_db();
        assert_eq!(import(&mut conn, &e).unwrap(), 5);
        assert_eq!(import(&mut conn, &e).unwrap(), 0, "re-import changes nothing");
        assert_eq!(export(&conn, "2026-09-14").unwrap().series, e.series);
        let (name, kind, dept): (String, String, String) = conn
            .query_row("SELECT name, kind, dept FROM station WHERE id='senamhi_st_472FC5B8'", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        assert_eq!((name.as_str(), kind.as_str(), dept.as_str()), ("PAIMAS", "automatic", "piura"));
    }

    #[test]
    fn import_rejects_bad_exports_without_writing() {
        let good = export(&sample_db(), "2026-09-14").unwrap();
        let cases: Vec<(&str, Box<dyn Fn(&mut Export)>)> = vec![
            ("version", Box::new(|e| e.version = 2)),
            ("series id", Box::new(|e| e.series[0].id = "precip_piura".into())),
            ("department", Box::new(|e| e.series[1].id = "senamhi_precip_atlantis".into())),
            ("negative value", Box::new(|e| e.series[2].points[0].1 = -1.0)),
            ("date", Box::new(|e| e.series[2].points[0].0 = "2026-13-01".into())),
            ("unit", Box::new(|e| e.series[3].unit = "stations".into())),
            ("station dept", Box::new(|e| e.stations[0].dept = "atlantis".into())),
            ("station id", Box::new(|e| e.stations[1].id = "senamhi_st_other".into())),
        ];
        for (what, break_it) in cases {
            let mut e: Export = serde_json::from_str(&serde_json::to_string(&good).unwrap()).unwrap();
            break_it(&mut e);
            let mut conn = test_db();
            assert!(import(&mut conn, &e).is_err(), "accepted bad {what}");
            let rows: i64 = conn.query_row("SELECT COUNT(*) FROM observation", [], |r| r.get(0)).unwrap();
            let stations: i64 = conn.query_row("SELECT COUNT(*) FROM station", [], |r| r.get(0)).unwrap();
            assert_eq!((rows, stations), (0, 0), "wrote data for bad {what}");
        }
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
