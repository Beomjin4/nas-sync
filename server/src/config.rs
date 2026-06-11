use anyhow::Result;
use figment::{
    providers::{Env, Format, Serialized, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Address to bind, e.g. "0.0.0.0:8080"
    pub bind: String,

    /// Root directory holding vault, trash, conflicts, db
    pub data_dir: PathBuf,
    pub vault_path: PathBuf,
    pub trash_path: PathBuf,
    pub conflicts_path: PathBuf,
    pub db_path: PathBuf,

    /// TTL before items in trash are permanently removed
    pub trash_ttl_days: u32,

    /// Reject PUTs with body larger than this
    pub max_file_size_mb: u64,

    /// JWT signing secret
    pub jwt_secret: String,

    /// Pairing code required to register a new device. If `None`, pairing is disabled.
    pub pairing_code: Option<String>,

    /// JWT lifetime in days
    pub jwt_ttl_days: u32,

    /// Password for the /admin web console. If `None`, the console is disabled.
    pub admin_password: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        let data_dir = PathBuf::from("./data");
        Self {
            bind: "0.0.0.0:8080".into(),
            vault_path: data_dir.join("vault"),
            trash_path: data_dir.join("trash"),
            conflicts_path: data_dir.join("conflicts"),
            db_path: data_dir.join("meta.db"),
            data_dir,
            trash_ttl_days: 30,
            max_file_size_mb: 100,
            jwt_secret: "change-me-in-production".into(),
            pairing_code: None,
            jwt_ttl_days: 365,
            admin_password: None,
        }
    }
}

/// Load config from defaults, optional config.toml, and `ONS_*` env vars
/// (env wins over file, file wins over defaults).
pub fn load() -> Result<Config> {
    let cfg: Config = Figment::from(Serialized::defaults(Config::default()))
        .merge(Toml::file("config.toml"))
        .merge(Env::prefixed("ONS_").split("__"))
        .extract()?;

    if cfg.jwt_secret == "change-me-in-production" {
        tracing::warn!("jwt_secret is using the default placeholder; set ONS_JWT_SECRET");
    }

    Ok(cfg)
}
