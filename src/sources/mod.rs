pub mod chirps;
pub mod cpc_products;
pub mod enfen;
pub mod gdacs;
pub mod indices;
pub mod oisst;

use crate::db;
use crate::fetch::Ctx;
use anyhow::{Result, bail};
use rusqlite::params;
use std::time::Instant;

const HOUR: u64 = 3600;
const DAY: u64 = 24 * HOUR;

pub struct SourceDef {
    pub id: &'static str,
    pub org: &'static str,
    pub name_en: &'static str,
    pub name_es: &'static str,
    /// Human-facing page for attribution.
    pub url: &'static str,
    pub license: &'static str,
    /// How often we poll (seconds).
    pub poll: u64,
    /// Expected publishing interval (seconds); data older than 2x this is flagged stale.
    pub expected: u64,
    pub cadence_en: &'static str,
    pub cadence_es: &'static str,
}

pub static REGISTRY: &[SourceDef] = &[
    SourceDef {
        id: "cpc_weekly",
        org: "NOAA CPC",
        name_en: "Weekly Niño region SST anomalies",
        name_es: "Anomalías semanales de TSM en regiones Niño",
        url: "https://www.cpc.ncep.noaa.gov/data/indices/wksst9120.for",
        license: "Public domain (U.S. Government)",
        poll: 6 * HOUR,
        expected: 7 * DAY,
        cadence_en: "Weekly (Mondays)",
        cadence_es: "Semanal (lunes)",
    },
    SourceDef {
        id: "cpc_oni",
        org: "NOAA CPC",
        name_en: "Oceanic Niño Index (ONI) and Relative ONI (RONI)",
        name_es: "Índice Oceánico El Niño (ONI) y ONI relativo (RONI)",
        url: "https://www.cpc.ncep.noaa.gov/products/analysis_monitoring/ensostuff/ONI_v5.php",
        license: "Public domain (U.S. Government)",
        poll: 12 * HOUR,
        expected: 31 * DAY,
        cadence_en: "Monthly",
        cadence_es: "Mensual",
    },
    SourceDef {
        id: "cpc_soi",
        org: "NOAA CPC",
        name_en: "Southern Oscillation Index (standardized)",
        name_es: "Índice de Oscilación del Sur (estandarizado)",
        url: "https://www.cpc.ncep.noaa.gov/data/indices/soi",
        license: "Public domain (U.S. Government)",
        poll: 12 * HOUR,
        expected: 31 * DAY,
        cadence_en: "Monthly",
        cadence_es: "Mensual",
    },
    SourceDef {
        id: "cpc_products",
        org: "NOAA CPC",
        name_en: "ENSO Diagnostic Discussion and official RONI outlook",
        name_es: "Discusión diagnóstica ENSO y pronóstico oficial del RONI",
        url: "https://www.cpc.ncep.noaa.gov/products/analysis_monitoring/enso_advisory/ensodisc.shtml",
        license: "Public domain (U.S. Government)",
        poll: 6 * HOUR,
        expected: 31 * DAY,
        cadence_en: "Monthly (2nd Thursday)",
        cadence_es: "Mensual (2.º jueves)",
    },
    SourceDef {
        id: "psl_mei",
        org: "NOAA PSL",
        name_en: "Multivariate ENSO Index v2 (MEI.v2)",
        name_es: "Índice ENSO Multivariado v2 (MEI.v2)",
        url: "https://psl.noaa.gov/enso/mei/",
        license: "Public domain (U.S. Government)",
        poll: 12 * HOUR,
        expected: 31 * DAY,
        cadence_en: "Monthly (~10th)",
        cadence_es: "Mensual (~día 10)",
    },
    SourceDef {
        id: "igp_icen",
        org: "IGP / ENFEN (Perú)",
        name_en: "Coastal El Niño Index (ICEN)",
        name_es: "Índice Costero El Niño (ICEN)",
        url: "http://met.igp.gob.pe/variabclim/indices.html",
        license: "Public data, Government of Peru — cite IGP/ENFEN",
        poll: 12 * HOUR,
        expected: 31 * DAY,
        cadence_en: "Monthly",
        cadence_es: "Mensual",
    },
    SourceDef {
        id: "enfen",
        org: "ENFEN (Perú)",
        name_en: "ENFEN official statements (Comunicados Oficiales)",
        name_es: "Comunicados Oficiales del ENFEN",
        url: "https://enfen.imarpe.gob.pe/comunicados/",
        license: "Public information, Government of Peru",
        poll: 6 * HOUR,
        expected: 16 * DAY,
        cadence_en: "About every two weeks",
        cadence_es: "Aproximadamente quincenal",
    },
    SourceDef {
        id: "oisst",
        org: "NOAA NCEI (via CoastWatch ERDDAP)",
        name_en: "Daily OISST v2.1 sea surface temperature anomaly (preliminary)",
        name_es: "Anomalía diaria de temperatura superficial del mar OISST v2.1 (preliminar)",
        url: "https://www.ncei.noaa.gov/products/optimum-interpolation-sst",
        license: "Public domain (U.S. Government)",
        poll: 6 * HOUR,
        expected: 2 * DAY,
        cadence_en: "Daily (~1 day lag)",
        cadence_es: "Diaria (~1 día de retraso)",
    },
    SourceDef {
        id: "chirps",
        org: "UCSB Climate Hazards Center (CHIRPS)",
        name_en: "Daily precipitation estimates (CHIRPS v3.0 preliminary)",
        name_es: "Estimación diaria de precipitación (CHIRPS v3.0 preliminar)",
        url: "https://www.chc.ucsb.edu/data/chirps3",
        license: "Public domain / CC BY 4.0 (UCSB CHC)",
        poll: 12 * HOUR,
        expected: 8 * DAY,
        cadence_en: "Daily, 2–7 day lag (pentads)",
        cadence_es: "Diaria, retraso de 2–7 días (pentadas)",
    },
    SourceDef {
        id: "gdacs",
        org: "GDACS (European Commission JRC / UN OCHA)",
        name_en: "Global disaster alerts: floods, droughts, cyclones, wildfires",
        name_es: "Alertas globales de desastres: inundaciones, sequías, ciclones, incendios",
        url: "https://www.gdacs.org/",
        license: "Free use with attribution to GDACS",
        poll: HOUR / 2,
        expected: DAY,
        cadence_en: "Near real time",
        cadence_es: "Casi en tiempo real",
    },
    SourceDef {
        id: "ncei_global",
        org: "NOAA NCEI",
        name_en: "Global land and ocean temperature anomaly",
        name_es: "Anomalía de temperatura global (tierra y océano)",
        url: "https://www.ncei.noaa.gov/access/monitoring/climate-at-a-glance/global/time-series",
        license: "Public domain (U.S. Government)",
        poll: 12 * HOUR,
        expected: 31 * DAY,
        cadence_en: "Monthly (~mid-month)",
        cadence_es: "Mensual (~mitad de mes)",
    },
];

pub fn def(id: &str) -> Option<&'static SourceDef> {
    REGISTRY.iter().find(|d| d.id == id)
}

/// Runs one source; returns true when new data was stored.
async fn dispatch(ctx: &Ctx, id: &str) -> Result<bool> {
    match id {
        "cpc_weekly" => indices::run_cpc_weekly(ctx).await,
        "cpc_oni" => indices::run_cpc_oni(ctx).await,
        "cpc_soi" => indices::run_cpc_soi(ctx).await,
        "cpc_products" => cpc_products::run(ctx).await,
        "psl_mei" => indices::run_mei(ctx).await,
        "igp_icen" => indices::run_icen(ctx).await,
        "enfen" => enfen::run(ctx).await,
        "oisst" => oisst::run(ctx).await,
        "chirps" => chirps::run(ctx).await,
        "gdacs" => gdacs::run(ctx).await,
        "ncei_global" => indices::run_ncei_global(ctx).await,
        _ => bail!("unknown source {id}"),
    }
}

/// Runs a source and records the outcome in source_status and fetch_log.
pub async fn run_and_record(ctx: &Ctx, id: &str) -> Result<bool> {
    let started = Instant::now();
    let at = db::now();
    let result = dispatch(ctx, id).await;
    let ms = started.elapsed().as_millis() as i64;
    let conn = ctx.db.get()?;
    let (outcome, message) = match &result {
        Ok(true) => ("changed", None),
        Ok(false) => ("unchanged", None),
        Err(e) => ("error", Some(format!("{e:#}"))),
    };
    conn.execute(
        "INSERT INTO fetch_log (source_id, at, outcome, ms, message) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, at, outcome, ms, message],
    )?;
    match &result {
        Ok(changed) => {
            conn.execute(
                "INSERT INTO source_status (id, last_attempt, last_success, last_change, status, message, fail_count)
                 VALUES (?1, ?2, ?2, CASE WHEN ?3 THEN ?2 END, ?4, NULL, 0)
                 ON CONFLICT(id) DO UPDATE SET last_attempt=?2, last_success=?2,
                   last_change=CASE WHEN ?3 THEN ?2 ELSE last_change END,
                   status=?4, message=NULL, fail_count=0",
                params![id, at, changed, outcome],
            )?;
            if *changed {
                ctx.bump();
            }
            tracing::info!(source = id, outcome, ms, "source run");
        }
        Err(e) => {
            conn.execute(
                "INSERT INTO source_status (id, last_attempt, status, message, fail_count)
                 VALUES (?1, ?2, 'error', ?3, 1)
                 ON CONFLICT(id) DO UPDATE SET last_attempt=?2, status='error', message=?3,
                   fail_count=fail_count+1",
                params![id, at, message],
            )?;
            tracing::warn!(source = id, ms, error = %format!("{e:#}"), "source failed");
        }
    }
    // Keep the log bounded.
    conn.execute(
        "DELETE FROM fetch_log WHERE id IN (SELECT id FROM fetch_log WHERE source_id=?1 ORDER BY id DESC LIMIT -1 OFFSET 500)",
        [id],
    )?;
    result
}
