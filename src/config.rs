use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub comdirect: ComdirectConfig,
    pub ynab: YnabConfig,
    pub sync: SyncConfig,
    pub op: OpConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComdirectConfig {
    pub user_id: String,
    pub iban: String,
    pub client_id: String,
    pub client_secret: String,
    pub username: String,
    pub pin: String,
    pub tan_method: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YnabConfig {
    pub token: String,
    pub budget_id: String,
    pub account_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    pub lookback_days: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpConfig {
    pub service_account_token_env: String,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("failed to read config at {}", path.display()))?;
        let config = toml::from_str(&contents)
            .with_context(|| format!("failed to parse config at {}", path.display()))?;
        Ok(config)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("failed to create config directory {}", parent.display())
            })?;
        }
        let contents = toml::to_string_pretty(self).context("failed to serialize config")?;
        fs::write(path, contents)
            .with_context(|| format!("failed to write config to {}", path.display()))?;
        Ok(())
    }
}
