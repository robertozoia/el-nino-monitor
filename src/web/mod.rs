mod api;
pub mod views;

use crate::fetch::Ctx;
use anyhow::{Context, Result};
use axum::Router;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use minijinja::Environment;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tower_http::compression::CompressionLayer;
use tower_http::services::ServeDir;
use tower_http::set_header::SetResponseHeaderLayer;
use views::{Dict, L};

pub const LANGS: [&str; 2] = ["es", "en"];
pub const PAGES: [&str; 7] = ["", "ocean", "indices", "peru", "impacts", "sources", "learn"];

pub struct AppState {
    pub ctx: Ctx,
    env: Environment<'static>,
    i18n: HashMap<String, Dict>,
    /// Changes on every deploy; appended to static URLs for cache busting.
    asset_version: String,
    /// API response cache: uri -> (data_version, body).
    cache: Mutex<HashMap<String, (u64, Arc<String>)>>,
    /// Recently used SST grids (date, grid), most recent last.
    grids: Mutex<Vec<(String, Arc<crate::grid::Grid>)>>,
    /// Rendered SST tiles keyed by "date/z/x/y".
    tiles: Mutex<HashMap<String, Arc<Vec<u8>>>>,
}

pub fn router(ctx: Ctx) -> Result<Router> {
    let assets = ctx.cfg.assets_dir.clone();
    let mut env = Environment::new();
    env.set_loader(minijinja::path_loader(assets.join("templates")));
    let mut i18n = HashMap::new();
    for lang in LANGS {
        let path = assets.join("i18n").join(format!("{lang}.toml"));
        let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let dict: Dict = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        i18n.insert(lang.to_string(), dict);
    }
    let state = Arc::new(AppState {
        ctx: ctx.clone(),
        env,
        i18n,
        asset_version: format!("{}-{}", env!("CARGO_PKG_VERSION"), chrono::Utc::now().timestamp()),
        cache: Mutex::new(HashMap::new()),
        grids: Mutex::new(Vec::new()),
        tiles: Mutex::new(HashMap::new()),
    });

    // Vendor libraries are pinned, so they can be cached for a long time. App JS/CSS
    // revalidate on every load (ES module imports carry no version query).
    let vendor = Router::new()
        .fallback_service(ServeDir::new(assets.join("static").join("vendor")))
        .layer(SetResponseHeaderLayer::overriding(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=2592000, immutable"),
        ));
    let static_files = Router::new()
        .fallback_service(ServeDir::new(assets.join("static")))
        .layer(SetResponseHeaderLayer::overriding(header::CACHE_CONTROL, HeaderValue::from_static("no-cache")));

    Ok(Router::new()
        .route("/", get(root))
        .route("/healthz", get(|| async { "ok" }))
        .route("/{lang}", get(|Path(lang): Path<String>| async move { Redirect::permanent(&format!("/{lang}/")) }))
        .route("/{lang}/", get(home))
        .route("/{lang}/{page}", get(page))
        .nest("/api", api::router())
        .nest_service("/static/vendor", vendor)
        .nest_service("/static", static_files)
        .route("/tiles/oisst/{date}/{z}/{x}/{y}", get(api::sst_tile))
        .layer(CompressionLayer::new())
        .with_state(state))
}

async fn root(headers: HeaderMap) -> Redirect {
    let accept = headers
        .get(header::ACCEPT_LANGUAGE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_lowercase();
    let es = accept.find("es");
    let en = accept.find("en");
    let lang = match (es, en) {
        (Some(a), Some(b)) if b < a => "en",
        (None, Some(_)) => "en",
        _ => "es",
    };
    Redirect::temporary(&format!("/{lang}/"))
}

async fn home(State(st): State<Arc<AppState>>, Path(lang): Path<String>) -> Response {
    render_page(st, lang, String::new()).await
}

async fn page(State(st): State<Arc<AppState>>, Path((lang, page)): Path<(String, String)>) -> Response {
    render_page(st, lang, page).await
}

async fn render_page(st: Arc<AppState>, lang: String, page: String) -> Response {
    if !LANGS.contains(&lang.as_str()) || !PAGES.contains(&page.as_str()) {
        return (StatusCode::NOT_FOUND, "Not found").into_response();
    }
    let result = tokio::task::spawn_blocking(move || build_page(&st, &lang, &page)).await;
    match result {
        Ok(Ok(html)) => (
            [(header::CACHE_CONTROL, "public, max-age=120")],
            Html(html),
        )
            .into_response(),
        Ok(Err(e)) => {
            tracing::error!(error = %format!("{e:#}"), "page render failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "page task panicked");
            (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
        }
    }
}

fn build_page(st: &AppState, lang: &str, page: &str) -> Result<String> {
    let t = &st.i18n[lang];
    let l = L { code: lang, t };
    let conn = st.ctx.db.get()?;
    let other = if lang == "es" { "en" } else { "es" };
    let name = if page.is_empty() { "now" } else { page };

    let data: Value = match name {
        "now" => {
            let cpc = views::cpc_status(&conn, &l)?;
            let enfen = views::enfen_status(&conn, &l)?;
            json!({
                "headline": views::headline(&conn, &l, &cpc, &enfen)?,
                "kpis": views::kpis(&conn, &l)?,
                "cpc": cpc,
                "enfen": enfen,
                "probs": views::probability_bars(&conn, &l)?,
            })
        }
        "peru" => json!({
            "enfen": views::enfen_status(&conn, &l)?,
            "documents": views::enfen_documents(&conn, &l, 40)?,
            "events": views::events_in(&conn, &l, &["andean_pacific", "south_america"], 180, 12)?,
            "kpis": views::kpis(&conn, &l)?.into_iter()
                .filter(|k| matches!(k["id"].as_str(), Some("icen" | "nino12_weekly")))
                .collect::<Vec<_>>(),
            "precip": views::peru_precip(&conn, &l)?,
        }),
        "impacts" => json!({ "regions": views::region_cards(&conn, &l)? }),
        "sources" => json!({
            "sources": views::source_rows(&conn, &l)?,
            "links": views::links(&conn)?,
        }),
        "indices" => json!({ "cpc": views::cpc_status(&conn, &l)? }),
        "learn" => {
            // Latest weekly value per Niño region, for the scene's live mode.
            let mut regions = serde_json::Map::new();
            for id in ["nino12_weekly", "nino3_weekly", "nino34_weekly", "nino4_weekly"] {
                if let Some((t, v)) = crate::db::latest_point(&conn, id)? {
                    regions.insert(id.into(), json!({ "value": v, "label": views::fmt_signed(v, 1), "date": views::fmt_date(&l, &t) }));
                }
            }
            json!({
                "kpis": views::kpis(&conn, &l)?,
                "cpc": views::cpc_status(&conn, &l)?,
                "enfen": views::enfen_status(&conn, &l)?,
                "regions": regions,
            })
        }
        _ => json!({}),
    };

    let nav: Vec<Value> = PAGES
        .iter()
        .map(|p| {
            let key = if p.is_empty() { "now" } else { p };
            json!({ "href": format!("/{lang}/{p}"), "label": l.tr(&format!("nav_{key}")), "active": *p == page })
        })
        .collect();

    let tmpl = st.env.get_template(&format!("{name}.html"))?;
    let html = tmpl.render(minijinja::context! {
        lang => lang,
        other_lang => other,
        other_href => format!("/{other}/{page}"),
        page => name,
        t => minijinja::Value::from_serialize(t),
        t_json => serde_json::to_string(t)?,
        nav => minijinja::Value::from_serialize(&nav),
        d => minijinja::Value::from_serialize(&data),
        // For inline <script>: "</" must not appear inside it.
        d_json => serde_json::to_string(&data)?.replace("</", "<\\/"),
        last_update => views::last_update(&conn, &l)?,
        v => &st.asset_version,
    })?;
    Ok(html)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn load_dict(lang: &str) -> Dict {
        let path = format!("{}/i18n/{lang}.toml", env!("CARGO_MANIFEST_DIR"));
        toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn i18n_keys_match() {
        let es = load_dict("es");
        let en = load_dict("en");
        let only_es: Vec<_> = es.keys().filter(|k| !en.contains_key(*k)).collect();
        let only_en: Vec<_> = en.keys().filter(|k| !es.contains_key(*k)).collect();
        assert!(only_es.is_empty() && only_en.is_empty(), "es-only {only_es:?}, en-only {only_en:?}");
    }

    /// Every page renders in both languages, with an empty database.
    #[test]
    fn pages_render_empty() {
        let dir = std::env::temp_dir().join(format!("elnino-web-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = Config::from_env();
        cfg.data_dir = dir.clone();
        cfg.assets_dir = env!("CARGO_MANIFEST_DIR").into();
        let pool = crate::db::open(&dir.join("t.db")).unwrap();
        let ctx = Ctx::new(Arc::new(cfg), pool).unwrap();
        let _ = router(ctx.clone()).unwrap();
        let st = test_state(ctx);
        for lang in LANGS {
            for page in PAGES {
                let html = build_page(&st, lang, page).unwrap_or_else(|e| panic!("{lang}/{page}: {e:#}"));
                assert!(html.contains("</html>"));
            }
        }
        std::fs::remove_dir_all(dir).ok();
    }

    fn test_state(ctx: Ctx) -> AppState {
        let mut env = Environment::new();
        env.set_loader(minijinja::path_loader(ctx.cfg.assets_dir.join("templates")));
        let i18n = LANGS.iter().map(|l| (l.to_string(), load_dict(l))).collect();
        AppState {
            ctx,
            env,
            i18n,
            asset_version: "t".into(),
            cache: Mutex::new(HashMap::new()),
            grids: Mutex::new(Vec::new()),
            tiles: Mutex::new(HashMap::new()),
        }
    }
}
