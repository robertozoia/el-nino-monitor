use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub data_dir: PathBuf,
    pub bind: String,
    pub user_agent: String,
    /// Directory holding templates/, static/ and i18n/.
    pub assets_dir: PathBuf,
    /// Disable background fetching (useful for local UI work).
    pub scheduler: bool,
    /// How many daily SST frames to keep and backfill.
    pub sst_days: i64,
    /// How many daily precipitation frames to keep and backfill.
    pub precip_days: i64,
    /// When set, the SENAMHI source imports this export file (pushed by a relay that
    /// can reach SENAMHI) instead of fetching from SENAMHI itself.
    pub senamhi_inbox: Option<PathBuf>,
}

impl Config {
    pub fn from_env() -> Self {
        let env = |k: &str, d: &str| std::env::var(k).unwrap_or_else(|_| d.to_string());
        let contact = env("CONTACT", "admin@example.com");
        Config {
            data_dir: env("DATA_DIR", "./data").into(),
            bind: env("BIND", "127.0.0.1:8080"),
            user_agent: format!("elnino-monitor/{} (+{contact})", env!("CARGO_PKG_VERSION")),
            assets_dir: env("ASSETS_DIR", ".").into(),
            scheduler: env("SCHEDULER", "on") != "off",
            sst_days: env("SST_DAYS", "90").parse().unwrap_or(90),
            precip_days: env("PRECIP_DAYS", "120").parse().unwrap_or(120),
            senamhi_inbox: Some(env("SENAMHI_INBOX", "")).filter(|s| !s.is_empty()).map(PathBuf::from),
        }
    }
}
