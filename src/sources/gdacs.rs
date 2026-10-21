//! GDACS hazard events (floods, droughts, tropical cyclones, wildfires), Orange and Red alerts.

use crate::fetch::{self, Ctx};
use anyhow::{Context, Result};
use rusqlite::{OptionalExtension, params};
use serde_json::Value;

const EVENT_TYPES: &str = "FL;DR;TC;WF";
const PAGE_SIZE: usize = 100;

/// Regions used for the impact cards. Checked in order; first match wins.
/// (id, lon_min, lon_max, lat_min, lat_max)
pub const REGIONS: &[(&str, f64, f64, f64, f64)] = &[
    ("andean_pacific", -95.0, -66.0, -25.0, 13.0),
    ("south_america", -82.0, -34.0, -56.0, 13.0),
    ("north_central_america", -125.0, -59.0, 7.0, 38.0),
    ("east_africa", 28.0, 52.0, -8.0, 16.0),
    ("southern_africa", 8.0, 52.0, -36.0, -8.0),
    ("south_asia", 60.0, 97.0, 5.0, 37.0),
    ("se_asia_australia", 90.0, 160.0, -45.0, 24.0),
    ("pacific_islands", 160.0, 180.0, -30.0, 25.0),
    ("pacific_islands", -180.0, -130.0, -30.0, 25.0),
];

pub fn region_of(lon: f64, lat: f64) -> &'static str {
    REGIONS
        .iter()
        .find(|(_, x0, x1, y0, y1)| lon >= *x0 && lon <= *x1 && lat >= *y0 && lat <= *y1)
        .map(|r| r.0)
        .unwrap_or("other")
}

#[derive(Debug)]
pub struct Event {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub alert: String,
    pub countries: String,
    pub iso3: String,
    pub lon: f64,
    pub lat: f64,
    pub from_date: String,
    pub to_date: String,
    pub modified: String,
    pub severity: String,
    pub url: String,
    pub is_current: bool,
}

pub fn parse(json: &str) -> Result<Vec<Event>> {
    let v: Value = serde_json::from_str(json)?;
    let features = v["features"].as_array().context("no features array")?;
    let s = |p: &Value, k: &str| p[k].as_str().unwrap_or_default().to_string();
    let mut out = Vec::new();
    for f in features {
        let p = &f["properties"];
        let coords = &f["geometry"]["coordinates"];
        let (Some(lon), Some(lat)) = (coords[0].as_f64(), coords[1].as_f64()) else { continue };
        let kind = s(p, "eventtype");
        let Some(eventid) = p["eventid"].as_i64() else { continue };
        out.push(Event {
            id: format!("gdacs:{kind}:{eventid}"),
            name: s(p, "name"),
            alert: s(p, "alertlevel"),
            countries: s(p, "country"),
            iso3: s(p, "iso3"),
            lon,
            lat,
            from_date: s(p, "fromdate"),
            to_date: s(p, "todate"),
            modified: s(p, "datemodified"),
            severity: p["severitydata"]["severitytext"].as_str().unwrap_or_default().to_string(),
            url: p["url"]["report"].as_str().unwrap_or_default().to_string(),
            is_current: p["iscurrent"].as_str() == Some("true"),
            kind,
        });
    }
    Ok(out)
}

pub async fn run(ctx: &Ctx) -> Result<bool> {
    let today = chrono::Utc::now().date_naive();
    let from = today - chrono::Duration::days(180);
    let mut events = Vec::new();
    for page in 1..=5 {
        let url = format!(
            "https://www.gdacs.org/gdacsapi/api/events/geteventlist/SEARCH?eventlist={EVENT_TYPES}&alertlevel=Orange;Red&fromDate={from}&toDate={today}&pagesize={PAGE_SIZE}&pagenumber={page}"
        );
        let body = fetch::get_plain(ctx, &url).await?;
        // GDACS answers an empty page with 204/empty body.
        if body.iter().all(|b| b.is_ascii_whitespace()) {
            break;
        }
        let batch = parse(&String::from_utf8_lossy(&body)).context("parsing GDACS")?;
        let n = batch.len();
        events.extend(batch);
        if n < PAGE_SIZE {
            break;
        }
    }

    let mut conn = ctx.db.get()?;
    let tx = conn.transaction()?;
    let mut changed = false;
    for e in &events {
        let prev: Option<(String, i64)> = tx
            .query_row("SELECT COALESCE(modified,''), is_current FROM event WHERE id=?1", [&e.id], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        if prev.as_ref() == Some(&(e.modified.clone(), e.is_current as i64)) {
            continue;
        }
        tx.execute(
            "INSERT INTO event (id, source_id, type, name, alert, countries, iso3, lon, lat, region,
                                from_date, to_date, modified, severity, url, is_current)
             VALUES (?1, 'gdacs', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
             ON CONFLICT(id) DO UPDATE SET name=excluded.name, alert=excluded.alert, countries=excluded.countries,
               iso3=excluded.iso3, lon=excluded.lon, lat=excluded.lat, region=excluded.region,
               from_date=excluded.from_date, to_date=excluded.to_date, modified=excluded.modified,
               severity=excluded.severity, url=excluded.url, is_current=excluded.is_current",
            params![
                e.id, e.kind, e.name, e.alert, e.countries, e.iso3, e.lon, e.lat,
                region_of(e.lon, e.lat), e.from_date, e.to_date, e.modified, e.severity, e.url, e.is_current
            ],
        )?;
        changed = true;
    }
    // Events no longer returned as current are closed.
    let ids: Vec<&str> = events.iter().filter(|e| e.is_current).map(|e| e.id.as_str()).collect();
    let open: Vec<String> = {
        let mut stmt = tx.prepare("SELECT id FROM event WHERE is_current=1")?;
        stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
    };
    for id in open.iter().filter(|id| !ids.contains(&id.as_str())) {
        tx.execute("UPDATE event SET is_current=0 WHERE id=?1", [id])?;
        changed = true;
    }
    tx.commit()?;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fixture() {
        let text = std::fs::read_to_string(format!("{}/tests/fixtures/gdacs.json", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let ev = parse(&text).unwrap();
        assert_eq!(ev.len(), 21);
        assert_eq!(ev[0].id, "gdacs:TC:1001338");
        assert_eq!(ev[0].alert, "Red");
        assert!(ev[0].is_current);
    }

    #[test]
    fn regions() {
        assert_eq!(region_of(-77.0, -12.0), "andean_pacific"); // Lima
        assert_eq!(region_of(-47.9, -15.8), "south_america"); // Brasília
        assert_eq!(region_of(106.8, -6.2), "se_asia_australia"); // Jakarta
        assert_eq!(region_of(36.8, -1.3), "east_africa"); // Nairobi
        assert_eq!(region_of(28.0, -26.2), "southern_africa"); // Johannesburg
        assert_eq!(region_of(-157.8, 21.3), "pacific_islands"); // Honolulu
        assert_eq!(region_of(2.3, 48.8), "other"); // Paris
    }
}
