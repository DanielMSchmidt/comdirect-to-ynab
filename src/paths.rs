use anyhow::{Context, Result};
use directories::ProjectDirs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Paths {
    pub config: PathBuf,
    pub state: PathBuf,
    pub base_dir: PathBuf,
}

impl Paths {
    pub fn new(config_override: Option<PathBuf>) -> Result<Self> {
        let default_dir = default_base_dir()?;
        let config_path = config_override.unwrap_or_else(|| default_dir.join("config.toml"));
        let base_dir = config_path
            .parent()
            .context("config path has no parent directory")?
            .to_path_buf();
        let state_path = base_dir.join("state.json");
        Ok(Self {
            config: config_path,
            state: state_path,
            base_dir,
        })
    }
}

fn default_base_dir() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("com", "comdirect", "comdirect-ynab")
        .context("unable to resolve application directory")?;
    Ok(dirs.data_dir().to_path_buf())
}
