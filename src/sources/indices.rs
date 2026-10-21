//! Plain-text and JSON index files: CPC weekly/ONI/RONI/SOI, PSL MEI.v2, IGP ICEN, NCEI global temperature.

use crate::db;
use crate::fetch::{self, Ctx, Fetched};
use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use std::sync::LazyLock as Lazy;
use regex::Regex;

pub struct Series {
    pub id: &'static str,
    pub unit: &'static str,
    pub freq: &'static str,
    pub points: Vec<(String, f64)>,
}

/// Three-month season codes; the index + 1 is the centre month.
pub const SEASONS: [&str; 12] = [
    "DJF", "JFM", "FMA", "MAM", "AMJ", "MJJ", "JJA", "JAS", "ASO", "SON", "OND", "NDJ",
];

fn month_date(year: i32, month: u32) -> String {
    format!("{year:04}-{month:02}-01")
}

/// Fetches one URL, parses it and stores every series it yields.
async fn fetch_store(
    ctx: &Ctx,
    source_id: &str,
    url: &str,
    parse: fn(&str) -> Result<Vec<Series>>,
) -> Result<bool> {
    let (fetched, pending) = fetch::get(ctx, source_id, url).await?;
    let changed = match fetched {
        Fetched::Unchanged => false,
        Fetched::Changed(body) => {
            let text = String::from_utf8_lossy(&body);
            let all = parse(&text).with_context(|| format!("parsing {url}"))?;
            let mut conn = ctx.db.get()?;
            let mut n = 0;
            for s in &all {
                if s.points.is_empty() {
                    bail!("{url}: series {} came back empty", s.id);
                }
                n += db::upsert_series(&mut conn, s.id, source_id, s.unit, s.freq, &s.points)?;
            }
            n > 0
        }
    };
    pending.commit(ctx)?;
    Ok(changed)
}

// ---------------------------------------------------------------- CPC weekly

static FLOAT: Lazy<Regex> = Lazy::new(|| Regex::new(r"-?\d+\.\d+").unwrap());

pub fn parse_cpc_weekly(text: &str) -> Result<Vec<Series>> {
    let ids = ["nino12_weekly", "nino3_weekly", "nino34_weekly", "nino4_weekly"];
    let mut out: Vec<Vec<(String, f64)>> = vec![Vec::new(); 4];
    for line in text.lines() {
        let line = line.trim();
        let Some(date_str) = line.get(..9) else { continue };
        let Ok(date) = NaiveDate::parse_from_str(date_str, "%d%b%Y") else { continue };
        let nums: Vec<f64> = FLOAT
            .find_iter(&line[9..])
            .filter_map(|m| m.as_str().parse().ok())
            .collect();
        if nums.len() != 8 {
            continue;
        }
        let t = date.format("%Y-%m-%d").to_string();
        for (i, series) in out.iter_mut().enumerate() {
            series.push((t.clone(), nums[i * 2 + 1]));
        }
    }
    Ok(ids
        .iter()
        .zip(out)
        .map(|(id, points)| Series { id, unit: "°C", freq: "weekly", points })
        .collect())
}

pub async fn run_cpc_weekly(ctx: &Ctx) -> Result<bool> {
    fetch_store(ctx, "cpc_weekly", "https://www.cpc.ncep.noaa.gov/data/indices/wksst9120.for", parse_cpc_weekly).await
}

// ---------------------------------------------------------------- ONI / RONI

/// Parses "SEAS YR [TOTAL] ANOM" files; the anomaly is always the last column.
fn parse_seasonal(text: &str) -> Vec<(String, f64)> {
    text.lines()
        .filter_map(|line| {
            let tok: Vec<&str> = line.split_whitespace().collect();
            let idx = SEASONS.iter().position(|s| Some(s) == tok.first())?;
            let year: i32 = tok.get(1)?.parse().ok()?;
            let anom: f64 = tok.last()?.parse().ok()?;
            Some((month_date(year, idx as u32 + 1), anom))
        })
        .collect()
}

pub fn parse_oni(text: &str) -> Result<Vec<Series>> {
    Ok(vec![Series { id: "oni", unit: "°C", freq: "monthly", points: parse_seasonal(text) }])
}

pub fn parse_roni(text: &str) -> Result<Vec<Series>> {
    Ok(vec![Series { id: "roni", unit: "°C", freq: "monthly", points: parse_seasonal(text) }])
}

pub async fn run_cpc_oni(ctx: &Ctx) -> Result<bool> {
    let a = fetch_store(ctx, "cpc_oni", "https://www.cpc.ncep.noaa.gov/data/indices/oni.ascii.txt", parse_oni).await?;
    let b = fetch_store(ctx, "cpc_oni", "https://www.cpc.ncep.noaa.gov/data/indices/RONI.ascii.txt", parse_roni).await?;
    Ok(a || b)
}

// ---------------------------------------------------------------- SOI

/// CPC's file holds two fixed-width blocks; we keep the standardized one.
pub fn parse_soi(text: &str) -> Result<Vec<Series>> {
    let mut in_block = false;
    let mut points = Vec::new();
    for line in text.lines() {
        if line.contains("STANDARDIZED") {
            in_block = true;
            continue;
        }
        if !in_block {
            continue;
        }
        let Some(year) = line.get(..4).and_then(|y| y.parse::<i32>().ok()) else { continue };
        for m in 0..12 {
            let Some(field) = line.get(4 + m * 6..10 + m * 6) else { break };
            if let Ok(v) = field.trim().parse::<f64>() {
                if v > -999.0 {
                    points.push((month_date(year, m as u32 + 1), v));
                }
            }
        }
    }
    Ok(vec![Series { id: "soi", unit: "σ", freq: "monthly", points }])
}

pub async fn run_cpc_soi(ctx: &Ctx) -> Result<bool> {
    fetch_store(ctx, "cpc_soi", "https://www.cpc.ncep.noaa.gov/data/indices/soi", parse_soi).await
}

// ---------------------------------------------------------------- MEI.v2

/// Rows are a year plus 12 overlapping two-month seasons (DJ, JF, ...); DJ maps to January.
pub fn parse_mei(text: &str) -> Result<Vec<Series>> {
    let mut points = Vec::new();
    for line in text.lines() {
        let tok: Vec<&str> = line.split_whitespace().collect();
        if tok.len() != 13 {
            continue;
        }
        let Ok(year) = tok[0].parse::<i32>() else { continue };
        for (m, v) in tok[1..].iter().enumerate() {
            if let Ok(v) = v.parse::<f64>() {
                if v > -999.0 {
                    points.push((month_date(year, m as u32 + 1), v));
                }
            }
        }
    }
    Ok(vec![Series { id: "mei", unit: "", freq: "monthly", points }])
}

pub async fn run_mei(ctx: &Ctx) -> Result<bool> {
    fetch_store(ctx, "psl_mei", "https://psl.noaa.gov/enso/mei/data/meiv2.data", parse_mei).await
}

// ---------------------------------------------------------------- ICEN

pub fn parse_icen(text: &str) -> Result<Vec<Series>> {
    let points = text
        .lines()
        .filter(|l| !l.trim_start().starts_with('%'))
        .filter_map(|l| {
            let tok: Vec<&str> = l.split_whitespace().collect();
            if tok.len() != 3 {
                return None;
            }
            let y: i32 = tok[0].parse().ok()?;
            let m: u32 = tok[1].parse().ok()?;
            let v: f64 = tok[2].parse().ok()?;
            (1..=12).contains(&m).then(|| (month_date(y, m), v))
        })
        .collect();
    Ok(vec![Series { id: "icen", unit: "°C", freq: "monthly", points }])
}

pub async fn run_icen(ctx: &Ctx) -> Result<bool> {
    fetch_store(ctx, "igp_icen", "http://met.igp.gob.pe/datos/ICEN.txt", parse_icen).await
}

// ---------------------------------------------------------------- NCEI global temperature

pub fn parse_ncei_global(text: &str) -> Result<Vec<Series>> {
    let v: serde_json::Value = serde_json::from_str(text)?;
    let data = v["data"].as_object().context("missing data object")?;
    let mut points: Vec<(String, f64)> = data
        .iter()
        .filter_map(|(k, d)| {
            let y: i32 = k.get(..4)?.parse().ok()?;
            let m: u32 = k.get(4..6)?.parse().ok()?;
            Some((month_date(y, m), d["departure"].as_f64()?))
        })
        .collect();
    points.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(vec![Series { id: "global_temp", unit: "°C", freq: "monthly", points }])
}

pub async fn run_ncei_global(ctx: &Ctx) -> Result<bool> {
    let year = chrono::Utc::now().format("%Y");
    let url = format!(
        "https://www.ncei.noaa.gov/access/monitoring/climate-at-a-glance/global/time-series/globe/land_ocean/1/0/1850-{year}/data.json"
    );
    fetch_store(ctx, "ncei_global", &url, parse_ncei_global).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let bytes = std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn cpc_weekly() {
        let s = parse_cpc_weekly(&fixture("wksst9120.for")).unwrap();
        assert_eq!(s.len(), 4);
        let n34 = &s[2].points;
        assert_eq!(n34.first().unwrap(), &("1981-09-02".to_string(), -0.2));
        assert_eq!(n34.last().unwrap(), &("2026-10-14".to_string(), 3.3));
        // Negative anomalies glued to the SST column ("20.6-0.1") must parse.
        assert_eq!(s[0].points[0].1, -0.1);
    }

    #[test]
    fn oni_and_roni() {
        let oni = &parse_oni(&fixture("oni.ascii.txt")).unwrap()[0].points;
        assert_eq!(oni[0], ("1950-01-01".to_string(), -1.32));
        assert_eq!(oni.last().unwrap(), &("2026-08-01".to_string(), 2.16));
        let roni = &parse_roni(&fixture("RONI.ascii.txt")).unwrap()[0].points;
        assert_eq!(roni.last().unwrap(), &("2026-08-01".to_string(), 1.69));
    }

    #[test]
    fn soi_standardized() {
        let soi = &parse_soi(&fixture("soi.txt")).unwrap()[0].points;
        assert_eq!(soi[0], ("1951-01-01".to_string(), 1.5));
        assert_eq!(soi.last().unwrap(), &("2026-09-01".to_string(), -2.0));
    }

    #[test]
    fn mei() {
        let mei = &parse_mei(&fixture("meiv2.data")).unwrap()[0].points;
        assert_eq!(mei[0], ("1979-01-01".to_string(), 0.46));
        assert_eq!(mei.last().unwrap(), &("2026-09-01".to_string(), 2.37));
    }

    #[test]
    fn icen() {
        let icen = &parse_icen(&fixture("ICEN.txt")).unwrap()[0].points;
        assert_eq!(icen[0], ("1950-01-01".to_string(), -0.75));
        assert_eq!(icen.last().unwrap(), &("2026-08-01".to_string(), 3.98));
    }

    #[test]
    fn ncei_global() {
        let g = &parse_ncei_global(&fixture("cag_global.json")).unwrap()[0].points;
        assert_eq!(g.last().unwrap(), &("2026-09-01".to_string(), 1.33));
    }
}
