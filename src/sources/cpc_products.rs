//! CPC monthly products: ENSO Diagnostic Discussion (status + synopsis) and the
//! official RONI outlook tables (percentile ranges, category and strength probabilities).

use crate::fetch::{self, Ctx, Fetched};
use anyhow::{Context, Result, bail};
use chrono::NaiveDate;
use regex::Regex;
use rusqlite::params;
use scraper::{Html, Selector};
use serde_json::json;
use std::sync::LazyLock;

const DISC_URL: &str = "https://www.cpc.ncep.noaa.gov/products/analysis_monitoring/enso_advisory/ensodisc.shtml";
const TABLES: [(&str, &str, &str); 3] = [
    ("cpc_roni_outlook", "https://www.cpc.ncep.noaa.gov/products/analysis_monitoring/enso/roni/outlook/", "table#outlook-table"),
    ("cpc_probabilities", "https://www.cpc.ncep.noaa.gov/products/analysis_monitoring/enso/roni/probabilities/", "table#probabilities-table"),
    ("cpc_strength", "https://www.cpc.ncep.noaa.gov/products/analysis_monitoring/enso/roni/strengths/", "table#probabilities-table"),
];

/// Visible text of a document, skipping script/style, whitespace collapsed.
pub fn visible_text(doc: &Html) -> String {
    let mut out = String::new();
    for node in doc.root_element().descendants() {
        let Some(text) = node.value().as_text() else { continue };
        let hidden = node
            .parent()
            .and_then(|p| p.value().as_element().map(|e| matches!(e.name(), "script" | "style")))
            .unwrap_or(false);
        if !hidden {
            out.push_str(text);
            out.push(' ');
        }
    }
    collapse(&out)
}

pub fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug)]
pub struct Discussion {
    pub issued: NaiveDate,
    pub status: String,
    pub synopsis: String,
    pub body: Vec<String>,
    pub next: Option<String>,
}

static ISSUED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"CLIMATE PREDICTION CENTER/NCEP/NWS (\d{1,2} [A-Z][a-z]+ \d{4})").unwrap());
static STATUS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"ENSO Alert System Status:\s*(.+?)\s*Synopsis:").unwrap());
static NEXT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"scheduled for (\d{1,2} [A-Z][a-z]+ \d{4})").unwrap());

pub fn parse_discussion(html: &str) -> Result<Discussion> {
    let doc = Html::parse_document(html);
    let text = visible_text(&doc);
    let issued = ISSUED.captures(&text).context("issue date not found")?[1].to_string();
    let issued = NaiveDate::parse_from_str(&issued, "%d %B %Y")?;
    let status = STATUS.captures(&text).context("alert status not found")?[1].to_string();
    let next = NEXT.captures(&text).map(|c| c[1].to_string());

    let p = Selector::parse("p").unwrap();
    let mut synopsis = None;
    let mut body = Vec::new();
    for el in doc.select(&p) {
        let t = collapse(&el.text().collect::<String>());
        if let Some(rest) = t.strip_prefix("Synopsis:") {
            synopsis = Some(rest.trim().to_string());
        } else if synopsis.is_some() {
            if t.starts_with("This discussion is") {
                break;
            }
            if t.len() > 40 {
                body.push(t);
            }
        }
    }
    let synopsis = synopsis.context("synopsis not found")?;
    Ok(Discussion { issued, status, synopsis, body, next })
}

#[derive(Debug)]
pub struct ForecastTable {
    pub issued: String, // YYYY-MM
    pub columns: Vec<String>,
    pub rows: Vec<(String, Vec<f64>)>,
}

static TABLE_ISSUED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Issued ([A-Z][a-z]+) (\d{4})").unwrap());

pub fn parse_table(html: &str, selector: &str) -> Result<ForecastTable> {
    let doc = Html::parse_document(html);
    // Visible text skips HTML comments, which on these pages hold stale "Issued ..." lines.
    let text = visible_text(&doc);
    let cap = TABLE_ISSUED.captures(&text).context("issue month not found")?;
    let issued = NaiveDate::parse_from_str(&format!("1 {} {}", &cap[1], &cap[2]), "%d %B %Y")?
        .format("%Y-%m")
        .to_string();

    let table_sel = Selector::parse(selector).unwrap();
    let tr = Selector::parse("tr").unwrap();
    let th = Selector::parse("th").unwrap();
    let td = Selector::parse("td").unwrap();
    let abbr = Selector::parse("abbr").unwrap();
    let table = doc.select(&table_sel).next().context("forecast table not found")?;
    let mut columns = Vec::new();
    let mut rows = Vec::new();
    for row in table.select(&tr) {
        let ths: Vec<_> = row.select(&th).collect();
        let tds: Vec<_> = row.select(&td).collect();
        if tds.is_empty() {
            columns = ths.iter().skip(1).map(|h| collapse(&h.text().collect::<String>())).collect();
            continue;
        }
        let season = ths
            .first()
            .and_then(|h| h.select(&abbr).next())
            .and_then(|a| a.text().next())
            .map(|s| s.trim().to_string())
            .context("season label missing")?;
        let values = tds
            .iter()
            .map(|d| d.text().collect::<String>().trim().parse::<f64>())
            .collect::<Result<Vec<_>, _>>()
            .with_context(|| format!("non-numeric cell in season {season}"))?;
        rows.push((season, values));
    }
    if rows.is_empty() {
        bail!("forecast table has no rows");
    }
    Ok(ForecastTable { issued, columns, rows })
}

pub async fn run(ctx: &Ctx) -> Result<bool> {
    let mut changed = false;

    let (fetched, pending) = fetch::get(ctx, "cpc_products", DISC_URL).await?;
    if let Fetched::Changed(body) = fetched {
        let d = parse_discussion(&String::from_utf8_lossy(&body))?;
        let issued = d.issued.format("%Y-%m-%d").to_string();
        let conn = ctx.db.get()?;
        conn.execute(
            "INSERT INTO document (id, source_id, number, issued, title, status, summary, url, extra)
             VALUES (?1, 'cpc_products', NULL, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET status=excluded.status, summary=excluded.summary, extra=excluded.extra",
            params![
                format!("cpc_disc:{issued}"),
                issued,
                "ENSO Diagnostic Discussion",
                d.status,
                d.synopsis,
                DISC_URL,
                json!({ "body": d.body, "next": d.next }).to_string(),
            ],
        )?;
        changed = true;
    }
    pending.commit(ctx)?;

    for (product, url, selector) in TABLES {
        let (fetched, pending) = fetch::get(ctx, "cpc_products", url).await?;
        if let Fetched::Changed(body) = fetched {
            let t = parse_table(&String::from_utf8_lossy(&body), selector)
                .with_context(|| format!("parsing {product}"))?;
            let mut conn = ctx.db.get()?;
            let tx = conn.transaction()?;
            tx.execute("DELETE FROM forecast WHERE product=?1 AND issued=?2", params![product, t.issued])?;
            for (i, (season, values)) in t.rows.iter().enumerate() {
                tx.execute(
                    "INSERT INTO forecast (product, issued, season, ord, data) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![product, t.issued, season, i as i64, json!({ "columns": t.columns, "values": values }).to_string()],
                )?;
            }
            tx.commit()?;
            changed = true;
        }
        pending.commit(ctx)?;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let bytes = std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    #[test]
    fn discussion() {
        let d = parse_discussion(&fixture("ensodisc.shtml")).unwrap();
        assert_eq!(d.issued, NaiveDate::from_ymd_opt(2026, 10, 8).unwrap());
        assert_eq!(d.status, "El Niño Advisory");
        assert!(d.synopsis.starts_with("El Niño continues to strengthen"));
        assert_eq!(d.body.len(), 2);
        assert_eq!(d.next.as_deref(), Some("12 November 2026"));
    }

    #[test]
    fn outlook_table() {
        let t = parse_table(&fixture("cpc_roni_outlook.html"), "table#outlook-table").unwrap();
        assert_eq!(t.issued, "2026-10");
        assert_eq!(t.columns, vec!["5%", "15%", "25%", "50%", "75%", "85%", "95%"]);
        assert_eq!(t.rows[1].0, "OND");
        assert_eq!(t.rows[1].1[3], 2.74);
        assert_eq!(t.rows.len(), 9);
    }

    #[test]
    fn probability_tables() {
        let p = parse_table(&fixture("cpc_roni_probabilities.html"), "table#probabilities-table").unwrap();
        assert_eq!(p.rows.last().unwrap(), &("MJJ".to_string(), vec![13.0, 70.0, 17.0]));
        let s = parse_table(&fixture("cpc_roni_strengths.html"), "table#probabilities-table").unwrap();
        assert_eq!(s.issued, "2026-10");
        assert_eq!(s.columns.len(), 9);
        assert_eq!(s.rows[3].1[8], 81.0);
    }
}
