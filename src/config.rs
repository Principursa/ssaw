use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct Paths {
    pub state_dir: PathBuf,
    pub config_dir: PathBuf,
    pub seed_file: PathBuf,
    pub chains_file: PathBuf,
    pub config_file: PathBuf,
    pub default_identity_file: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AppConfig {
    pub identity_file: Option<PathBuf>,
}

impl Paths {
    pub fn discover() -> Result<Self> {
        let home = home::home_dir().context("failed to determine home directory")?;
        let state_dir = home.join(".ssaw");
        let config_dir = home.join(".config").join("ssaw");

        Ok(Self {
            seed_file: state_dir.join("seed.age"),
            chains_file: state_dir.join("chains.toml"),
            config_file: config_dir.join("config.toml"),
            default_identity_file: config_dir.join("identity.txt"),
            state_dir,
            config_dir,
        })
    }

    pub fn ensure_parent_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.state_dir)
            .with_context(|| format!("failed to create {}", self.state_dir.display()))?;
        fs::create_dir_all(&self.config_dir)
            .with_context(|| format!("failed to create {}", self.config_dir.display()))?;
        Ok(())
    }

    pub fn load_config(&self) -> Result<AppConfig> {
        if !self.config_file.exists() {
            return Ok(AppConfig::default());
        }

        let raw = fs::read_to_string(&self.config_file)
            .with_context(|| format!("failed to read {}", self.config_file.display()))?;
        Ok(toml::from_str(&raw)
            .with_context(|| format!("failed to parse {}", self.config_file.display()))?)
    }

    pub fn identity_file(&self) -> Result<PathBuf> {
        let config = self.load_config()?;
        Ok(config.identity_file.unwrap_or_else(|| self.default_identity_file.clone()))
    }

    pub fn write_config(&self, config: &AppConfig) -> Result<()> {
        self.ensure_parent_dirs()?;
        let body = toml::to_string_pretty(config).context("failed to serialize config")?;
        fs::write(&self.config_file, body)
            .with_context(|| format!("failed to write {}", self.config_file.display()))?;
        Ok(())
    }
}

pub fn write_file(path: &Path, body: impl AsRef<[u8]>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }

    fs::write(path, body).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}
