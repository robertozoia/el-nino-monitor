//! JSON API consumed by the page scripts. Responses are cached in memory until
//! any source stores new data (Ctx::data_version).

use super::AppState;
use super::views::{self, L};
use crate::db;
use crate::grid::Grid;
use crate::sources::oisst;
use anyhow::Result;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/series/{id}", get(series))
        .route("/analogs/{id}", get(analogs))
        .route("/forecast/{product}", get(forecast))
        .route("/events", get(events))
        .route("/sst/frames", get(sst_frames))
        .route("/sst/hovmoller", get(sst_hovmoller))
        .route("/sst/point", get(sst_point))
        .route("/status", get(status))
}

/// Runs `f` on the blocking pool unless a cached body for this URI is still current.
async fn cached<F>(st: Arc<AppState>, uri: &Uri, f: F) -> Response
where
    F: FnOnce(&AppState) -> Result<Option<Value>> + Send + 'static,
{
    let key = uri.to_string();
    let version = st.ctx.data_version.load(Ordering::Relaxed);
    if let Some((v, body)) = st.cache.lock().unwrap().get(&key) {
        if *v == version {
            return json_response(body.as_str().to_owned());
        }
    }
    let st2 = st.clone();
    let result = tokio::task::spawn_blocking(move || f(&st2)).await;
    match result {
        Ok(Ok(Some(v))) => {
            let body = Arc::new(v.to_string());
            let mut cache = st.cache.lock().unwrap();
            if cache.len() > 500 {
                cache.clear();
            }
            cache.insert(key, (version, body.clone()));
            json_response(body.as_str().to_owned())
        }
        Ok(Ok(None)) => (StatusCode::NOT_FOUND, "not found").into_response(),
        Ok(Err(e)) => {
            tracing::error!(error = %format!("{e:#}"), %uri, "api error");
            (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response()
        }
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal error").into_response(),
    }
}

fn json_response(body: String) -> Response {
    (
        [(header::CONTENT_TYPE, "application/json"), (header::CACHE_CONTROL, "public, max-age=300")],
        body,
    )
        .into_response()
}

#[derive(Deserialize)]
struct SeriesQuery {
    from: Option<String>,
}

async fn series(State(st): State<Arc<AppState>>, uri: Uri, Path(id): Path<String>, Query(q): Query<SeriesQuery>) -> Response {
    cached(st, &uri, move |st| {
        let conn = st.ctx.db.get()?;
        let meta: Option<(String, String, String)> = rusqlite::OptionalExtension::optional(conn.query_row(
            "SELECT unit, freq, source_id FROM series WHERE id=?1",
            [&id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ))?;
        let Some((unit, freq, source)) = meta else { return Ok(None) };
        let points: Vec<Value> = db::series_points(&conn, &id, q.from.as_deref())?
            .into_iter()
            .map(|(t, v)| json!([t, v]))
            .collect();
        Ok(Some(json!({ "id": id, "unit": unit, "freq": freq, "source": source, "points": points })))
    })
    .await
}

/// Past strong events aligned by month: Jan of the onset year through Dec of the following year.
const ANALOG_YEARS: [i32; 5] = [1972, 1982, 1997, 2015, 2023];
const CURRENT_YEAR: i32 = 2026;

async fn analogs(State(st): State<Arc<AppState>>, uri: Uri, Path(id): Path<String>) -> Response {
    cached(st, &uri, move |st| {
        if !matches!(id.as_str(), "oni" | "roni" | "icen" | "mei") {
            return Ok(None);
        }
        let conn = st.ctx.db.get()?;
        let mut out = Vec::new();
        for year in ANALOG_YEARS.iter().copied().chain([CURRENT_YEAR]) {
            let pts = db::series_points(&conn, &id, Some(&format!("{year}-01-01")))?;
            let map: HashMap<String, f64> = pts.into_iter().collect();
            let values: Vec<Value> = (0..24)
                .map(|i| {
                    let (y, m) = (year + i / 12, i % 12 + 1);
                    map.get(&format!("{y:04}-{m:02}-01")).map(|v| json!(v)).unwrap_or(Value::Null)
                })
                .collect();
            out.push(json!({ "year": year, "label": format!("{}–{}", year, (year + 1) % 100), "values": values, "current": year == CURRENT_YEAR }));
        }
        Ok(Some(json!({ "id": id, "events": out })))
    })
    .await
}

async fn forecast(State(st): State<Arc<AppState>>, uri: Uri, Path(product): Path<String>) -> Response {
    cached(st, &uri, move |st| {
        let conn = st.ctx.db.get()?;
        let Some((issued, columns, rows)) = views::forecast(&conn, &product)? else { return Ok(None) };
        let rows: Vec<Value> = rows.into_iter().map(|(s, v)| json!({ "season": s, "values": v })).collect();
        Ok(Some(json!({ "product": product, "issued": issued, "columns": columns, "rows": rows })))
    })
    .await
}

#[derive(Deserialize)]
struct EventsQuery {
    days: Option<i64>,
    lang: Option<String>,
}

async fn events(State(st): State<Arc<AppState>>, uri: Uri, Query(q): Query<EventsQuery>) -> Response {
    cached(st, &uri, move |st| {
        let conn = st.ctx.db.get()?;
        let lang = q.lang.filter(|l| super::LANGS.contains(&l.as_str())).unwrap_or_else(|| "es".into());
        let l = L { code: &lang, t: &st.i18n[&lang] };
        let since = (chrono::Utc::now().date_naive() - chrono::Duration::days(q.days.unwrap_or(180).clamp(1, 730))).to_string();
        let mut stmt = conn.prepare(
            "SELECT id, type, name, alert, countries, from_date, to_date, lon, lat, region, url, is_current, severity
             FROM event WHERE COALESCE(to_date, from_date) >= ?1 ORDER BY from_date",
        )?;
        let features: Vec<Value> = stmt
            .query_map([since], |r| {
                let kind: String = r.get(1)?;
                let from: String = r.get(5)?;
                let to: Option<String> = r.get(6)?;
                Ok(json!({
                    "type": "Feature",
                    "geometry": { "type": "Point", "coordinates": [r.get::<_, f64>(7)?, r.get::<_, f64>(8)?] },
                    "properties": {
                        "id": r.get::<_, String>(0)?,
                        "kind": kind,
                        "kind_label": l.tr(&format!("ev_{kind}")),
                        "name": r.get::<_, String>(2)?,
                        "alert": r.get::<_, String>(3)?,
                        "countries": r.get::<_, Option<String>>(4)?,
                        "from": views::fmt_date(&l, &from),
                        "to": to.map(|t| views::fmt_date(&l, &t)),
                        "region": r.get::<_, String>(9)?,
                        "url": r.get::<_, Option<String>>(10)?,
                        "current": r.get::<_, bool>(11)?,
                        "severity": r.get::<_, Option<String>>(12)?,
                    }
                }))
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Some(json!({ "type": "FeatureCollection", "features": features })))
    })
    .await
}

async fn sst_frames(State(st): State<Arc<AppState>>, uri: Uri) -> Response {
    cached(st, &uri, move |st| {
        let dates = oisst::frame_dates(&st.ctx)?;
        Ok(Some(json!({
            "dates": dates,
            "tiles": "/tiles/oisst/{date}/{z}/{x}/{y}.png",
            "maxzoom": TILE_MAX_ZOOM,
        })))
    })
    .await
}

/// Time × longitude diagram of equatorial (5°S–5°N) SST anomalies, 120°E–80°W.
async fn sst_hovmoller(State(st): State<Arc<AppState>>, uri: Uri) -> Response {
    cached(st, &uri, move |st| {
        let dates = oisst::frame_dates(&st.ctx)?;
        let mut lons: Vec<f64> = Vec::new();
        let mut cells = Vec::new();
        for (yi, d) in dates.iter().enumerate() {
            let Ok(g) = Grid::load(&oisst::grid_path(&st.ctx, d)) else { continue };
            let band = g.equatorial_band(120.0, 280.0);
            if lons.is_empty() {
                lons = band.iter().map(|(l, _)| *l).collect();
            }
            for (xi, (_, v)) in band.iter().enumerate() {
                if let Some(v) = v {
                    cells.push(json!([xi, yi, (v * 100.0).round() / 100.0]));
                }
            }
        }
        Ok(Some(json!({ "lons": lons, "dates": dates, "values": cells })))
    })
    .await
}

/// 1° data gains nothing beyond this zoom; MapLibre over-zooms the last level.
const TILE_MAX_ZOOM: u32 = 5;
const GRID_CACHE: usize = 8;
const TILE_CACHE: usize = 1500;

/// Decoded grid for a date, from a small most-recently-used cache.
fn grid_for(st: &AppState, date: &str) -> Result<Arc<Grid>> {
    let mut cache = st.grids.lock().unwrap();
    if let Some(i) = cache.iter().position(|(d, _)| d == date) {
        let entry = cache.remove(i);
        let g = entry.1.clone();
        cache.push(entry);
        return Ok(g);
    }
    drop(cache);
    let g = Arc::new(Grid::load(&oisst::grid_path(&st.ctx, date))?);
    let mut cache = st.grids.lock().unwrap();
    if cache.len() >= GRID_CACHE {
        cache.remove(0);
    }
    cache.push((date.to_string(), g.clone()));
    Ok(g)
}

pub async fn sst_tile(
    State(st): State<Arc<AppState>>,
    Path((date, z, x, y)): Path<(String, u32, u32, String)>,
) -> Response {
    let Some(y) = y.strip_suffix(".png").and_then(|v| v.parse::<u32>().ok()) else {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    };
    if chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d").is_err() || z > TILE_MAX_ZOOM {
        return (StatusCode::NOT_FOUND, "not found").into_response();
    }
    let key = format!("{date}/{z}/{x}/{y}");
    let hit = st.tiles.lock().unwrap().get(&key).cloned();
    let png = match hit {
        Some(p) => p,
        None => {
            let st2 = st.clone();
            let r = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
                grid_for(&st2, &date)?.render_tile(z, x, y, &crate::grid::SST_ANOMALY)
            })
            .await;
            let Ok(Ok(bytes)) = r else { return (StatusCode::NOT_FOUND, "no tile").into_response() };
            let bytes = Arc::new(bytes);
            let mut tiles = st.tiles.lock().unwrap();
            if tiles.len() >= TILE_CACHE {
                tiles.clear();
            }
            tiles.insert(key, bytes.clone());
            bytes
        }
    };
    (
        [(header::CONTENT_TYPE, "image/png"), (header::CACHE_CONTROL, "public, max-age=86400")],
        png.as_ref().clone(),
    )
        .into_response()
}

#[derive(Deserialize)]
struct PointQuery {
    date: String,
    lat: f64,
    lon: f64,
}

async fn sst_point(State(st): State<Arc<AppState>>, Query(q): Query<PointQuery>) -> Response {
    if chrono::NaiveDate::parse_from_str(&q.date, "%Y-%m-%d").is_err() {
        return (StatusCode::BAD_REQUEST, "bad date").into_response();
    }
    let r = tokio::task::spawn_blocking(move || {
        grid_for(&st, &q.date).map(|g| g.sample_nearest(q.lat, q.lon).map(|v| (v as f64 * 100.0).round() / 100.0))
    })
    .await;
    match r {
        Ok(Ok(v)) => (
            [(header::CACHE_CONTROL, "public, max-age=86400")],
            axum::Json(json!({ "value": v })),
        )
            .into_response(),
        _ => (StatusCode::NOT_FOUND, "no frame").into_response(),
    }
}

async fn status(State(st): State<Arc<AppState>>, uri: Uri) -> Response {
    cached(st, &uri, move |st| {
        let conn = st.ctx.db.get()?;
        let l = L { code: "en", t: &st.i18n["en"] };
        Ok(Some(json!({ "sources": views::source_rows(&conn, &l)? })))
    })
    .await
}
