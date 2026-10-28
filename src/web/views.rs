//! Server-side view models for the HTML pages, plus locale-aware formatting.

use crate::db;
use crate::sources::gdacs::REGIONS;
use crate::sources::indices::SEASONS;
use crate::sources::REGISTRY;
use anyhow::Result;
use chrono::{Datelike, NaiveDate};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub type Dict = BTreeMap<String, String>;

pub struct L<'a> {
    pub code: &'a str,
    pub t: &'a Dict,
}

impl L<'_> {
    pub fn tr(&self, key: &str) -> String {
        self.t.get(key).cloned().unwrap_or_else(|| key.to_string())
    }
    fn es(&self) -> bool {
        self.code == "es"
    }
}

// ------------------------------------------------------------------ formatting

const MONTHS_ES: [&str; 12] = ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sep", "oct", "nov", "dic"];
const MONTHS_EN: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

fn parse_date(iso: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(iso.get(..10)?, "%Y-%m-%d").ok()
}

fn month_name(l: &L, m: u32) -> &'static str {
    let i = (m as usize).saturating_sub(1).min(11);
    if l.es() { MONTHS_ES[i] } else { MONTHS_EN[i] }
}

pub fn fmt_date(l: &L, iso: &str) -> String {
    match parse_date(iso) {
        Some(d) => format!("{} {} {}", d.day(), month_name(l, d.month()), d.year()),
        None => iso.to_string(),
    }
}

pub fn fmt_month(l: &L, iso: &str) -> String {
    match parse_date(iso) {
        Some(d) => format!("{} {}", month_name(l, d.month()), d.year()),
        None => iso.to_string(),
    }
}

/// Three-month season centred on the given month, e.g. 2026-08-01 -> "JAS 2026".
pub fn season_label(l: &L, iso: &str) -> String {
    let Some(d) = parse_date(iso) else { return iso.to_string() };
    format!("{} {}", season_code(l, SEASONS[d.month0() as usize]), d.year())
}

/// Spanish month initials differ for three seasons (Dic-Ene-Feb = DEF, ...).
pub fn season_code(l: &L, code: &str) -> String {
    if !l.es() {
        return code.to_string();
    }
    match code {
        "DJF" => "DEF",
        "JFM" => "EFM",
        "NDJ" => "NDE",
        c => c,
    }
    .to_string()
}

pub fn fmt_signed(v: f64, decimals: usize) -> String {
    let s = format!("{v:+.decimals$}");
    if s.starts_with("-0") && s.trim_start_matches(['-', '0', '.']).is_empty() {
        s[1..].to_string()
    } else {
        s.replace('-', "−")
    }
}

pub fn rel_time(l: &L, rfc3339: Option<&str>) -> String {
    let Some(t) = rfc3339.and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()) else {
        return l.tr("never");
    };
    let mins = (chrono::Utc::now() - t.to_utc()).num_minutes().max(0);
    let (n, unit) = match mins {
        0..=1 => return l.tr("just_now"),
        2..=89 => (mins, "min"),
        90..=2159 => ((mins + 30) / 60, "h"),
        _ => ((mins + 720) / 1440, if l.es() { "d" } else { "d" }),
    };
    if l.es() { format!("hace {n} {unit}") } else { format!("{n} {unit} ago") }
}

// ------------------------------------------------------------------ categories

/// NOAA convention on ONI/RONI: weak 0.5, moderate 1.0, strong 1.5, very strong 2.0.
pub fn enso_category(v: f64) -> &'static str {
    match v {
        v if v >= 2.0 => "very_strong",
        v if v >= 1.5 => "strong",
        v if v >= 1.0 => "moderate",
        v if v >= 0.5 => "weak",
        v if v > -0.5 => "neutral",
        _ => "la_nina",
    }
}

/// ENFEN magnitudes for the ICEN (warm side): weak 0.4, moderate 1.0, strong 1.7, extraordinary 3.0.
pub fn icen_category(v: f64) -> &'static str {
    match v {
        v if v >= 3.0 => "extraordinary",
        v if v >= 1.7 => "strong",
        v if v >= 1.0 => "moderate",
        v if v >= 0.4 => "weak",
        v if v > -1.0 => "neutral",
        _ => "cold",
    }
}

// ------------------------------------------------------------------ sparkline

pub fn sparkline(values: &[f64]) -> Value {
    let (w, h, pad) = (120.0, 36.0, 3.0);
    if values.len() < 2 {
        return json!(null);
    }
    let min = values.iter().copied().fold(0.0_f64, f64::min);
    let max = values.iter().copied().fold(0.0_f64, f64::max);
    let span = (max - min).max(0.5);
    let y = |v: f64| pad + (max - v) / span * (h - 2.0 * pad);
    let step = (w - 2.0 * pad) / (values.len() - 1) as f64;
    let mut d = String::new();
    for (i, v) in values.iter().enumerate() {
        d.push_str(&format!("{}{:.1},{:.1}", if i == 0 { "M" } else { " L" }, pad + i as f64 * step, y(*v)));
    }
    let last = values[values.len() - 1];
    json!({
        "path": d,
        "zero": format!("{:.1}", y(0.0)),
        "cx": format!("{:.1}", pad + (values.len() - 1) as f64 * step),
        "cy": format!("{:.1}", y(last)),
        "w": w, "h": h,
    })
}

// ------------------------------------------------------------------ KPIs

#[derive(Clone, Copy)]
enum Kind {
    Weekly,
    Seasonal,
    Monthly,
    Daily,
}

struct KpiDef {
    series: &'static str,
    key: &'static str,
    kind: Kind,
    spark: usize,
    source: &'static str,
    href: &'static str,
}

const KPIS: &[KpiDef] = &[
    KpiDef { series: "nino34_weekly", key: "kpi_nino34", kind: Kind::Weekly, spark: 26, source: "NOAA CPC", href: "indices" },
    KpiDef { series: "nino12_weekly", key: "kpi_nino12", kind: Kind::Weekly, spark: 26, source: "NOAA CPC", href: "peru" },
    KpiDef { series: "roni", key: "kpi_roni", kind: Kind::Seasonal, spark: 24, source: "NOAA CPC", href: "indices" },
    KpiDef { series: "icen", key: "kpi_icen", kind: Kind::Monthly, spark: 24, source: "IGP / ENFEN", href: "peru" },
    KpiDef { series: "soi", key: "kpi_soi", kind: Kind::Monthly, spark: 24, source: "NOAA CPC", href: "indices" },
    KpiDef { series: "nino34_oisst", key: "kpi_nino34_daily", kind: Kind::Daily, spark: 60, source: "NOAA OISST", href: "ocean" },
];

/// Value of `series` at the same point of the 1997–98 event (same month/day in 1997).
fn same_point_1997(conn: &Connection, series: &str, t: &str, kind: Kind) -> Result<Option<f64>> {
    let Some(d) = parse_date(t) else { return Ok(None) };
    let target = NaiveDate::from_ymd_opt(1997, d.month(), d.day().min(28)).unwrap();
    Ok(match kind {
        Kind::Weekly | Kind::Daily => conn
            .query_row(
                "SELECT value FROM observation WHERE series_id=?1 AND t BETWEEN ?2 AND ?3
                 ORDER BY abs(julianday(t) - julianday(?4)) LIMIT 1",
                params![
                    if matches!(kind, Kind::Daily) { "nino34_weekly" } else { series },
                    (target - chrono::Duration::days(4)).to_string(),
                    (target + chrono::Duration::days(4)).to_string(),
                    target.to_string()
                ],
                |r| r.get(0),
            )
            .optional()?,
        Kind::Seasonal | Kind::Monthly => db::point_at(conn, series, &format!("1997-{:02}-01", d.month()))?,
    })
}

pub fn kpis(conn: &Connection, l: &L) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for k in KPIS {
        let pts = {
            let mut stmt = conn.prepare_cached(
                "SELECT t, value FROM observation WHERE series_id=?1 ORDER BY t DESC LIMIT ?2",
            )?;
            let mut v: Vec<(String, f64)> = stmt
                .query_map(params![k.series, k.spark as i64], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<rusqlite::Result<_>>()?;
            v.reverse();
            v
        };
        let Some((t, v)) = pts.last().cloned() else { continue };
        let prev = pts.len().checked_sub(2).map(|i| pts[i].1);
        let date_label = match k.kind {
            Kind::Weekly => format!("{} {}", l.tr("week_of"), fmt_date(l, &t)),
            Kind::Daily => fmt_date(l, &t),
            Kind::Seasonal => season_label(l, &t),
            Kind::Monthly => fmt_month(l, &t),
        };
        let category = match k.series {
            "roni" | "nino34_weekly" => Some(enso_category(v)),
            "icen" | "nino12_weekly" => Some(icen_category(v)),
            _ => None,
        };
        let ref97 = same_point_1997(conn, k.series, &t, k.kind)?;
        let values: Vec<f64> = pts.iter().map(|p| p.1).collect();
        let unit = if k.series == "soi" { "" } else { " °C" };
        out.push(json!({
            "id": k.series,
            "label": l.tr(k.key),
            "hint": l.tr(&format!("{}_hint", k.key)),
            "value": fmt_signed(v, if k.series == "soi" { 1 } else { 2 }) ,
            "raw": v,
            "unit": unit,
            "date": date_label,
            "delta": prev.map(|p| fmt_signed(v - p, 2)),
            "delta_up": prev.map(|p| v >= p),
            "category": category,
            "category_label": category.map(|c| l.tr(&format!("cat_{c}"))),
            "ref97": ref97.map(|r| fmt_signed(r, 2)),
            "spark": sparkline(&values),
            "source": k.source,
            "href": k.href,
            "warm": if k.series == "soi" { v < 0.0 } else { v > 0.0 },
        }));
    }
    Ok(out)
}

// ------------------------------------------------------------------ peru precipitation

/// Department daily precipitation rows for the Peru page: latest day, 7/30-day totals,
/// sparkline, plus the department list (with chart defaults) and overall freshness.
pub fn peru_precip(conn: &Connection, l: &L) -> Result<Value> {
    use crate::sources::chirps;
    let depts = chirps::departments();
    let today = chrono::Utc::now().date_naive();
    fn name<'a>(l: &L, d: &'a chirps::Department) -> &'a str {
        if l.es() { d.name_es.as_str() } else { d.name_en.as_str() }
    }
    let mm = |v: f64| format!("{v:.1}");

    let mut rows = Vec::new();
    let mut series = Vec::new();
    let mut latest_all: Option<String> = None;
    for d in depts {
        let id = chirps::series_id(d);
        series.push(json!({
            "code": d.code,
            "name": name(l, d),
            "default": matches!(d.code.as_str(), "tumbes" | "piura" | "lambayeque" | "la_libertad" | "ancash" | "lima"),
        }));
        let latest: Option<(String, f64)> = conn
            .query_row(
                "SELECT t, value FROM observation WHERE series_id=?1 ORDER BY t DESC LIMIT 1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((t, v)) = latest else {
            rows.push(json!({ "code": d.code, "name": name(l, d), "latest": null, "last7": null, "last30": null, "spark": null }));
            continue;
        };
        if latest_all.as_deref() < Some(t.as_str()) {
            latest_all = Some(t.clone());
        }
        let since = |days: i64| (parse_date(&t).unwrap() - chrono::Duration::days(days)).to_string();
        let total = |days: i64| -> Result<f64> {
            Ok(conn
                .query_row(
                    "SELECT COALESCE(SUM(value), 0) FROM observation WHERE series_id=?1 AND t > ?2",
                    params![id, since(days)],
                    |r| r.get(0),
                )?)
        };
        let spark_pts: Vec<f64> = {
            let mut stmt = conn.prepare("SELECT value FROM observation WHERE series_id=?1 ORDER BY t DESC LIMIT 60")?;
            let mut v: Vec<f64> = stmt.query_map([&id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
            v.reverse();
            v
        };
        rows.push(json!({
            "code": d.code,
            "name": name(l, d),
            "latest": mm(v),
            "last7": mm(total(7)?),
            "last30": mm(total(30)?),
            "spark": sparkline(&spark_pts),
        }));
    }
    let lag_days = latest_all.as_deref().and_then(parse_date).map(|d| (today - d).num_days());
    Ok(json!({
        "latest": latest_all.as_deref().map(|t| fmt_date(l, t)),
        "lag_days": lag_days,
        "series": series,
        "rows": rows,
    }))
}

// ------------------------------------------------------------------ official status

pub fn cpc_status(conn: &Connection, l: &L) -> Result<Value> {
    let row: Option<(String, String, Option<String>, Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT issued, COALESCE(status,''), summary, url, extra FROM document
             WHERE source_id='cpc_products' ORDER BY issued DESC LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let Some((issued, status, summary, url, extra)) = row else { return Ok(json!(null)) };
    let extra: Value = extra.and_then(|e| serde_json::from_str(&e).ok()).unwrap_or(json!({}));
    let status_key = match status.as_str() {
        "El Niño Advisory" => "cpc_status_en_advisory",
        "El Niño Watch" => "cpc_status_en_watch",
        "La Niña Advisory" => "cpc_status_ln_advisory",
        "La Niña Watch" => "cpc_status_ln_watch",
        "Final El Niño Advisory" => "cpc_status_en_final",
        "Final La Niña Advisory" => "cpc_status_ln_final",
        _ => "",
    };
    Ok(json!({
        "issued": fmt_date(l, &issued),
        "status": status,
        "status_local": if status_key.is_empty() { status.clone() } else { l.tr(status_key) },
        "synopsis": summary,
        "body": extra["body"],
        "next": extra["next"].as_str().and_then(|n| NaiveDate::parse_from_str(n, "%d %B %Y").ok())
            .map(|d| fmt_date(l, &d.to_string())),
        "url": url,
    }))
}

pub fn enfen_documents(conn: &Connection, l: &L, limit: usize) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare(
        "SELECT number, issued, title, status, summary, url, pdf_url FROM document
         WHERE source_id='enfen' ORDER BY issued DESC, number DESC LIMIT ?1",
    )?;
    let rows = stmt
        .query_map([limit as i64], |r| {
            Ok(json!({
                "number": r.get::<_, Option<String>>(0)?,
                "issued_iso": r.get::<_, String>(1)?,
                "title": r.get::<_, String>(2)?,
                "status": r.get::<_, Option<String>>(3)?,
                "summary": r.get::<_, Option<String>>(4)?,
                "url": r.get::<_, Option<String>>(5)?,
                "pdf_url": r.get::<_, Option<String>>(6)?,
            }))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows
        .into_iter()
        .map(|mut d| {
            let iso = d["issued_iso"].as_str().unwrap_or_default().to_string();
            d["issued"] = json!(fmt_date(l, &iso));
            d
        })
        .collect())
}

pub fn enfen_status(conn: &Connection, l: &L) -> Result<Value> {
    let docs = enfen_documents(conn, l, 40)?;
    let Some(latest) = docs.first().cloned() else { return Ok(json!(null)) };
    // Newer PDF-only comunicados carry no parsed state; use the latest stated one.
    let stated = docs.iter().find(|d| d["status"].is_string()).cloned();
    Ok(json!({
        "latest": latest,
        "status": stated.as_ref().map(|d| d["status"].clone()),
        "status_from": stated.as_ref().map(|d| d["number"].clone()),
        "summary": stated.as_ref().map(|d| d["summary"].clone()),
    }))
}

// ------------------------------------------------------------------ forecasts

pub fn forecast(conn: &Connection, product: &str) -> Result<Option<(String, Vec<String>, Vec<(String, Vec<f64>)>)>> {
    let issued: Option<String> = conn
        .query_row("SELECT MAX(issued) FROM forecast WHERE product=?1", [product], |r| r.get(0))
        .optional()?
        .flatten();
    let Some(issued) = issued else { return Ok(None) };
    let mut stmt = conn.prepare("SELECT season, data FROM forecast WHERE product=?1 AND issued=?2 ORDER BY ord")?;
    let mut columns = Vec::new();
    let rows = stmt
        .query_map(params![product, issued], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?
        .into_iter()
        .map(|(season, data)| {
            let v: Value = serde_json::from_str(&data).unwrap_or(json!({}));
            if columns.is_empty() {
                columns = v["columns"].as_array().into_iter().flatten().filter_map(|c| c.as_str().map(String::from)).collect();
            }
            let values = v["values"].as_array().into_iter().flatten().filter_map(|x| x.as_f64()).collect();
            (season, values)
        })
        .collect();
    Ok(Some((issued, columns, rows)))
}

/// Probability that each upcoming season is El Niño, for the "Now" page bars.
pub fn probability_bars(conn: &Connection, l: &L) -> Result<Value> {
    let Some((issued, _, rows)) = forecast(conn, "cpc_probabilities")? else { return Ok(json!(null)) };
    let rows: Vec<Value> = rows
        .iter()
        .take(6)
        .map(|(s, v)| {
            json!({
                "season": season_code(l, s),
                "la_nina": v.first().map(|x| x.round() as i64),
                "neutral": v.get(1).map(|x| x.round() as i64),
                "el_nino": v.get(2).map(|x| x.round() as i64),
            })
        })
        .collect();
    Ok(json!({ "issued": fmt_month(l, &format!("{issued}-01")), "rows": rows }))
}

// ------------------------------------------------------------------ headline

pub fn headline(conn: &Connection, l: &L, cpc: &Value, enfen: &Value) -> Result<Value> {
    let roni = db::latest_point(conn, "roni")?;
    let n34 = db::latest_point(conn, "nino34_weekly")?;
    let n12 = db::latest_point(conn, "nino12_weekly")?;
    let category = roni.as_ref().map(|(_, v)| enso_category(*v)).unwrap_or("neutral");
    let fill = |tpl: String| -> String {
        let mut s = tpl;
        if let Some((t, v)) = &n34 {
            s = s.replace("{n34}", &fmt_signed(*v, 1)).replace("{n34_date}", &fmt_date(l, t));
        }
        if let Some((_, v)) = &n12 {
            s = s.replace("{n12}", &fmt_signed(*v, 1));
        }
        if let Some((t, v)) = &roni {
            s = s.replace("{roni}", &fmt_signed(*v, 2)).replace("{roni_season}", &season_label(l, t));
        }
        s.replace("{cpc_status}", cpc["status_local"].as_str().unwrap_or("—"))
            .replace("{enfen_status}", enfen["status"].as_str().unwrap_or("—"))
    };
    Ok(json!({
        "category": category,
        "title": l.tr(&format!("headline_{category}")),
        "summary": fill(l.tr("summary_tpl")),
    }))
}

// ------------------------------------------------------------------ events & regions

pub fn region_ids() -> Vec<&'static str> {
    let mut ids: Vec<&str> = Vec::new();
    for r in REGIONS {
        if !ids.contains(&r.0) {
            ids.push(r.0);
        }
    }
    ids
}

fn event_json(l: &L, r: &rusqlite::Row) -> rusqlite::Result<Value> {
    let kind: String = r.get(1)?;
    let from: String = r.get(5)?;
    Ok(json!({
        "id": r.get::<_, String>(0)?,
        "type": kind,
        "type_label": l.tr(&format!("ev_{kind}")),
        "name": r.get::<_, String>(2)?,
        "alert": r.get::<_, String>(3)?,
        "countries": r.get::<_, Option<String>>(4)?,
        "from": fmt_date(l, &from),
        "url": r.get::<_, Option<String>>(6)?,
        "current": r.get::<_, bool>(7)?,
    }))
}

pub fn events_in(conn: &Connection, l: &L, regions: &[&str], days: i64, limit: usize) -> Result<Vec<Value>> {
    let since = (chrono::Utc::now().date_naive() - chrono::Duration::days(days)).to_string();
    let placeholders = vec!["?"; regions.len()].join(",");
    let sql = format!(
        "SELECT id, type, name, alert, countries, from_date, url, is_current FROM event
         WHERE region IN ({placeholders}) AND COALESCE(to_date, from_date) >= ?
         ORDER BY is_current DESC, CASE alert WHEN 'Red' THEN 0 ELSE 1 END, from_date DESC LIMIT {limit}"
    );
    let mut stmt = conn.prepare(&sql)?;
    let mut args: Vec<&dyn rusqlite::ToSql> = regions.iter().map(|r| r as &dyn rusqlite::ToSql).collect();
    args.push(&since);
    let rows = stmt.query_map(args.as_slice(), |r| event_json(l, r))?.collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

pub fn region_cards(conn: &Connection, l: &L) -> Result<Vec<Value>> {
    let since = (chrono::Utc::now().date_naive() - chrono::Duration::days(180)).to_string();
    let mut out = Vec::new();
    for id in region_ids() {
        let mut counts = serde_json::Map::new();
        let mut stmt = conn.prepare_cached(
            "SELECT type, COUNT(*) FROM event WHERE region=?1 AND COALESCE(to_date, from_date) >= ?2 GROUP BY type",
        )?;
        for row in stmt.query_map(params![id, since], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
            let (k, n) = row?;
            counts.insert(k, json!(n));
        }
        out.push(json!({
            "id": id,
            "title": l.tr(&format!("region_{id}")),
            "typical": l.tr(&format!("region_{id}_typical")),
            "season": l.tr(&format!("region_{id}_season")),
            "counts": counts,
            "events": events_in(conn, l, &[id], 180, 3)?,
        }));
    }
    Ok(out)
}

// ------------------------------------------------------------------ sources

pub fn source_rows(conn: &Connection, l: &L) -> Result<Vec<Value>> {
    let mut out = Vec::new();
    for d in REGISTRY {
        let st: Option<(Option<String>, Option<String>, Option<String>, Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT last_attempt, last_success, last_change, status, message FROM source_status WHERE id=?1",
                [d.id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let (attempt, success, change, status, message) = st.unwrap_or_default();
        let stale = success
            .as_deref()
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| (chrono::Utc::now() - t.to_utc()).num_seconds() as u64 > 2 * d.expected.max(d.poll))
            .unwrap_or(true);
        out.push(json!({
            "id": d.id,
            "org": d.org,
            "name": if l.es() { d.name_es } else { d.name_en },
            "url": d.url,
            "license": d.license,
            "cadence": if l.es() { d.cadence_es } else { d.cadence_en },
            "last_attempt": rel_time(l, attempt.as_deref()),
            "last_success": rel_time(l, success.as_deref()),
            "last_change": rel_time(l, change.as_deref()),
            "status": status.clone().unwrap_or_else(|| "pending".into()),
            "message": message,
            "stale": stale,
        }));
    }
    Ok(out)
}

pub fn links(conn: &Connection) -> Result<Vec<Value>> {
    let mut stmt = conn.prepare("SELECT org, title, url, issued FROM link ORDER BY org")?;
    let rows = stmt
        .query_map([], |r| {
            Ok(json!({ "org": r.get::<_, String>(0)?, "title": r.get::<_, String>(1)?,
                       "url": r.get::<_, String>(2)?, "issued": r.get::<_, Option<String>>(3)? }))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// Compact freshness line for the footer: newest successful fetch over all sources.
pub fn last_update(conn: &Connection, l: &L) -> Result<String> {
    let t: Option<String> = conn.query_row("SELECT MAX(last_success) FROM source_status", [], |r| r.get(0))?;
    Ok(rel_time(l, t.as_deref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting() {
        let t = Dict::new();
        let es = L { code: "es", t: &t };
        let en = L { code: "en", t: &t };
        assert_eq!(fmt_date(&es, "2026-10-14"), "14 oct 2026");
        assert_eq!(fmt_month(&en, "2026-08-01"), "Aug 2026");
        assert_eq!(season_label(&en, "2026-08-01"), "JAS 2026");
        assert_eq!(season_label(&es, "2027-01-01"), "DEF 2027");
        assert_eq!(fmt_signed(3.3, 1), "+3.3");
        assert_eq!(fmt_signed(-1.25, 2), "−1.25");
        assert_eq!(fmt_signed(-0.001, 2), "0.00");
        assert_eq!(enso_category(1.69), "strong");
        assert_eq!(icen_category(3.98), "extraordinary");
    }
}
