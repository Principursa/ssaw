use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct Paths {
    pub project_name: String,
    pub state_dir: PathBuf,
    pub project_dir: PathBuf,
    pub projects_dir: PathBuf,
    pub config_dir: PathBuf,
    pub current_project_file: PathBuf,
    pub seed_file: PathBuf,
    pub chains_file: PathBuf,
    pub addresses_file: PathBuf,
    pub lock_file: PathBuf,
    pub config_file: PathBuf,
    pub default_identity_file: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AppConfig {
    pub identity_file: Option<PathBuf>,
}

impl Paths {
    pub fn discover() -> Result<Self> {
        Self::discover_with_project(None)
    }

    pub fn discover_with_project(project: Option<&str>) -> Result<Self> {
        let home = home::home_dir().context("failed to determine home directory")?;
        let state_dir = home.join(".ssaw");
        let projects_dir = state_dir.join("projects");
        let config_dir = home.join(".config").join("ssaw");
        let current_project_file = state_dir.join("current-project");
        let project_name = project
            .map(str::to_owned)
            .map(Ok)
            .unwrap_or_else(|| Self::read_current_project_name(&current_project_file))
            .unwrap_or_else(|_| "default".to_owned());

        validate_project_name(&project_name)?;

        let project_dir = if project_name == "default" {
            state_dir.clone()
        } else {
            projects_dir.join(&project_name)
        };

        Ok(Self {
            project_name,
            project_dir: project_dir.clone(),
            projects_dir,
            current_project_file,
            seed_file: project_dir.join("seed.age"),
            chains_file: project_dir.join("chains.toml"),
            addresses_file: project_dir.join("addresses.toml"),
            lock_file: project_dir.join("wallet.lock"),
            config_file: config_dir.join("config.toml"),
            default_identity_file: config_dir.join("identity.txt"),
            state_dir,
            config_dir,
        })
    }

    pub fn ensure_parent_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.state_dir)
            .with_context(|| format!("failed to create {}", self.state_dir.display()))?;
        fs::create_dir_all(&self.project_dir)
            .with_context(|| format!("failed to create {}", self.project_dir.display()))?;
        fs::create_dir_all(&self.config_dir)
            .with_context(|| format!("failed to create {}", self.config_dir.display()))?;
        Ok(())
    }

    pub fn write_current_project(&self, project_name: &str) -> Result<()> {
        validate_project_name(project_name)?;
        if let Some(parent) = self.current_project_file.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(&self.current_project_file, format!("{project_name}\n"))
            .with_context(|| format!("failed to write {}", self.current_project_file.display()))
    }

    pub fn named_project_dir(&self, project_name: &str) -> PathBuf {
        if project_name == "default" {
            self.state_dir.clone()
        } else {
            self.projects_dir.join(project_name)
        }
    }

    pub fn list_projects(&self) -> Result<Vec<String>> {
        let mut projects = vec!["default".to_owned()];
        if !self.projects_dir.exists() {
            return Ok(projects);
        }

        for entry in fs::read_dir(&self.projects_dir)
            .with_context(|| format!("failed to read {}", self.projects_dir.display()))?
        {
            let entry = entry.context("failed to read project entry")?;
            if !entry
                .file_type()
                .context("failed to read project entry type")?
                .is_dir()
            {
                continue;
            }

            let name = entry.file_name();
            let name = name.to_string_lossy();
            if validate_project_name(&name).is_ok() {
                projects.push(name.into_owned());
            }
        }

        projects.sort();
        projects.dedup();
        Ok(projects)
    }

    fn read_current_project_name(current_project_file: &Path) -> Result<String> {
        let raw = fs::read_to_string(current_project_file)
            .with_context(|| format!("failed to read {}", current_project_file.display()))?;
        let project_name = raw.trim();
        if project_name.is_empty() {
            bail!("current project file was empty");
        }
        Ok(project_name.to_owned())
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
        Ok(config
            .identity_file
            .unwrap_or_else(|| self.default_identity_file.clone()))
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

pub fn validate_project_name(project_name: &str) -> Result<()> {
    if project_name.is_empty() {
        bail!("project name cannot be empty");
    }

    if project_name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return Ok(());
    }

    bail!("project name must use only ASCII letters, numbers, `-`, or `_`")
}
