//! ENFEN (Peru) official statements. Two inputs are merged:
//! - WordPress REST posts: title carries the alert state, excerpt a summary (lags behind);
//! - homepage "Últimos Comunicados" list: newest PDFs with number and date.

use crate::fetch::{self, Ctx, Fetched};
use crate::sources::cpc_products::collapse;
use anyhow::{Context, Result};
use chrono::NaiveDate;
use regex::Regex;
use rusqlite::params;
use serde::Deserialize;
use std::sync::LazyLock;

const POSTS_URL: &str = "https://enfen.imarpe.gob.pe/wp-json/wp/v2/posts?search=comunicado&per_page=30&_fields=id,date,link,title,excerpt";
const HOME_URL: &str = "https://enfen.imarpe.gob.pe/";

#[derive(Debug, Default, PartialEq)]
pub struct Comunicado {
    pub number: u32,
    pub year: i32,
    pub issued: Option<NaiveDate>,
    pub title: Option<String>,
    pub status: Option<String>,
    pub summary: Option<String>,
    pub url: Option<String>,
    pub pdf_url: Option<String>,
}

impl Comunicado {
    pub fn id(&self) -> String {
        format!("enfen:{:02}-{}", self.number, self.year)
    }
}

static NUMBER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)N[°º]\s*(\d{1,2})\s*[-–]\s*(\d{4})").unwrap());
static STATE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:sistema de alerta|alerta ante [^:]+)\s*:\s*(.+?)[”\x22]*$").unwrap()
});
static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());

fn decode_entities(s: &str) -> String {
    s.replace("&#8211;", "–")
        .replace("&#8221;", "”")
        .replace("&#8220;", "“")
        .replace("&#8230;", "…")
        .replace("&#038;", "&")
        .replace("&amp;", "&")
        .replace("&nbsp;", " ")
        .replace("&#8216;", "‘")
        .replace("&#8217;", "’")
}

#[derive(Deserialize)]
struct Rendered {
    rendered: String,
}

#[derive(Deserialize)]
struct Post {
    date: String,
    link: String,
    title: Rendered,
    excerpt: Rendered,
}

pub fn parse_posts(json: &str) -> Result<Vec<Comunicado>> {
    let posts: Vec<Post> = serde_json::from_str(json)?;
    let mut out = Vec::new();
    for p in posts {
        let title = collapse(&decode_entities(&p.title.rendered));
        if !title.to_lowercase().starts_with("comunicado oficial") {
            continue;
        }
        let Some(cap) = NUMBER.captures(&title) else { continue };
        let status = STATE.captures(&title).map(|c| c[1].trim().trim_end_matches('"').to_string());
        let summary = collapse(&decode_entities(&TAGS.replace_all(&p.excerpt.rendered, "")));
        out.push(Comunicado {
            number: cap[1].parse()?,
            year: cap[2].parse()?,
            issued: NaiveDate::parse_from_str(p.date.get(..10).unwrap_or_default(), "%Y-%m-%d").ok(),
            title: Some(title),
            status,
            summary: (!summary.is_empty()).then_some(summary),
            url: Some(p.link),
            pdf_url: None,
        });
    }
    Ok(out)
}

static DOWNLOAD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"href="(https://enfen\.imarpe\.gob\.pe/download/comunicado-oficial-enfen-n-(\d{1,2})-(\d{4})/\?wpdmdl=\d+)[^"]*"[^>]*>([^<]+)</a>"#,
    )
    .unwrap()
});
static LIST_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\|\s*(\d{1,2}) ([A-Za-zé]+),? (\d{4})").unwrap());

fn spanish_month(m: &str) -> Option<u32> {
    let months = [
        "enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre",
        "noviembre", "diciembre",
    ];
    let m = m.to_lowercase();
    if m == "setiembre" {
        return Some(9);
    }
    months.iter().position(|x| *x == m).map(|i| i as u32 + 1)
}

pub fn parse_home(html: &str) -> Vec<Comunicado> {
    let mut out: Vec<Comunicado> = Vec::new();
    let anchors: Vec<_> = DOWNLOAD.captures_iter(html).collect();
    for (i, c) in anchors.iter().enumerate() {
        // Only list entries ("Comunicado Oficial ENFEN N° ..."), not "Ver Comunicado" buttons.
        if !c[4].to_lowercase().contains("comunicado oficial") {
            continue;
        }
        let number: u32 = c[2].parse().unwrap_or(0);
        let year: i32 = c[3].parse().unwrap_or(0);
        if out.iter().any(|x| x.number == number && x.year == year) {
            continue;
        }
        // The "size | date" line follows the anchor, before the next entry.
        let start = c.get(0).unwrap().end();
        let mut end = anchors
            .get(i + 1)
            .map(|n| n.get(0).unwrap().start())
            .unwrap_or(html.len())
            .min(start + 600);
        while !html.is_char_boundary(end) {
            end -= 1;
        }
        let issued = LIST_DATE.captures(&html[start..end]).and_then(|d| {
            NaiveDate::from_ymd_opt(d[3].parse().ok()?, spanish_month(&d[2])?, d[1].parse().ok()?)
        });
        out.push(Comunicado {
            number,
            year,
            issued,
            title: Some(collapse(&c[4])),
            pdf_url: Some(c[1].to_string()),
            ..Default::default()
        });
    }
    out
}

fn store(ctx: &Ctx, items: &[Comunicado]) -> Result<()> {
    let conn = ctx.db.get()?;
    for c in items {
        // Fall back to Jan 1 when the date is unknown; a later run fills the real one.
        let issued = c
            .issued
            .unwrap_or_else(|| NaiveDate::from_ymd_opt(c.year, 1, 1).unwrap())
            .format("%Y-%m-%d")
            .to_string();
        conn.execute(
            "INSERT INTO document (id, source_id, number, issued, title, status, summary, url, pdf_url)
             VALUES (?1, 'enfen', ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
               issued  = CASE WHEN ?9 THEN excluded.issued ELSE issued END,
               title   = COALESCE(excluded.title, title),
               status  = COALESCE(excluded.status, status),
               summary = COALESCE(excluded.summary, summary),
               url     = COALESCE(excluded.url, url),
               pdf_url = COALESCE(excluded.pdf_url, pdf_url)",
            params![
                c.id(),
                format!("{:02}-{}", c.number, c.year),
                issued,
                c.title.clone().unwrap_or_else(|| format!("Comunicado Oficial ENFEN N° {:02}-{}", c.number, c.year)),
                c.status,
                c.summary,
                c.url,
                c.pdf_url,
                c.issued.is_some(),
            ],
        )?;
    }
    Ok(())
}

pub async fn run(ctx: &Ctx) -> Result<bool> {
    let mut changed = false;
    let (fetched, pending) = fetch::get(ctx, "enfen", POSTS_URL).await?;
    if let Fetched::Changed(body) = fetched {
        let items = parse_posts(&String::from_utf8_lossy(&body)).context("parsing ENFEN posts")?;
        store(ctx, &items)?;
        changed = true;
    }
    pending.commit(ctx)?;

    // The homepage embeds per-request tokens, so its hash changes every time;
    // compare the parsed list instead.
    let body = fetch::get_plain(ctx, HOME_URL).await?;
    let items = parse_home(&String::from_utf8_lossy(&body));
    if items.is_empty() {
        anyhow::bail!("no comunicados found on ENFEN homepage (layout changed?)");
    }
    let known: i64 = {
        let conn = ctx.db.get()?;
        let mut n = 0;
        for c in &items {
            n += conn.query_row(
                "SELECT COUNT(*) FROM document WHERE id=?1 AND pdf_url IS NOT NULL",
                [c.id()],
                |r| r.get::<_, i64>(0),
            )?;
        }
        n
    };
    if known < items.len() as i64 {
        store(ctx, &items)?;
        changed = true;
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
    fn posts() {
        let items = parse_posts(&fixture("enfen_posts.json")).unwrap();
        assert_eq!(items.len(), 13);
        let first = &items[0];
        assert_eq!((first.number, first.year), (13, 2026));
        assert_eq!(first.status.as_deref(), Some("Alerta de El Niño Costero"));
        assert_eq!(first.issued, NaiveDate::from_ymd_opt(2026, 7, 17));
        let last = items.last().unwrap();
        assert_eq!(last.number, 1);
        assert_eq!(last.status.as_deref(), Some("Vigilancia de El Niño Costero"));
    }

    #[test]
    fn home() {
        let items = parse_home(&fixture("enfen_home.html"));
        assert!(items.len() >= 4, "{items:?}");
        assert_eq!((items[0].number, items[0].year), (18, 2026));
        assert_eq!(items[0].issued, NaiveDate::from_ymd_opt(2026, 10, 16));
        assert!(items[0].pdf_url.as_ref().unwrap().ends_with("wpdmdl=2165"));
    }
}
