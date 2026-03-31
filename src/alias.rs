use std::collections::BTreeMap;
use std::fs;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::{Paths, write_file};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AddressAlias {
    pub index: u32,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AliasConfig {
    #[serde(default)]
    pub addresses: BTreeMap<String, AddressAlias>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AliasEntry {
    pub name: String,
    pub index: u32,
    pub labels: Vec<String>,
}

pub fn load(paths: &Paths) -> Result<AliasConfig> {
    if !paths.addresses_file.exists() {
        return Ok(AliasConfig::default());
    }

    let raw = fs::read_to_string(&paths.addresses_file)
        .with_context(|| format!("failed to read {}", paths.addresses_file.display()))?;
    Ok(toml::from_str(&raw)
        .with_context(|| format!("failed to parse {}", paths.addresses_file.display()))?)
}

pub fn set_alias(paths: &Paths, name: &str, index: u32, labels: Vec<String>) -> Result<()> {
    validate_alias_name(name)?;
    let mut config = load(paths)?;
    let labels = normalize_labels(labels);
    config
        .addresses
        .insert(name.to_owned(), AddressAlias { index, labels });
    let body = toml::to_string_pretty(&config).context("failed to serialize alias config")?;
    write_file(&paths.addresses_file, body)
}

pub fn get_alias(paths: &Paths, name: &str) -> Result<Option<AddressAlias>> {
    validate_alias_name(name)?;
    Ok(load(paths)?.addresses.get(name).cloned())
}

pub fn list_aliases(paths: &Paths) -> Result<Vec<AliasEntry>> {
    let config = load(paths)?;
    Ok(config
        .addresses
        .into_iter()
        .map(|(name, entry)| AliasEntry {
            name,
            index: entry.index,
            labels: entry.labels,
        })
        .collect())
}

pub fn resolve_alias(paths: &Paths, name: &str) -> Result<u32> {
    let entry = get_alias(paths, name)?
        .with_context(|| format!("unknown alias `{name}` in project `{}`", paths.project_name))?;
    Ok(entry.index)
}

pub fn aliases_for_index(paths: &Paths, index: u32) -> Result<Vec<AliasEntry>> {
    Ok(list_aliases(paths)?
        .into_iter()
        .filter(|entry| entry.index == index)
        .collect())
}

pub fn validate_alias_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("alias name cannot be empty");
    }

    if name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
    {
        return Ok(());
    }

    bail!("alias name must use only ASCII letters, numbers, `-`, or `_`")
}

fn normalize_labels(labels: Vec<String>) -> Vec<String> {
    let mut labels: Vec<String> = labels
        .into_iter()
        .map(|label| label.trim().to_owned())
        .filter(|label| !label.is_empty())
        .collect();
    labels.sort();
    labels.dedup();
    labels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_labels() {
        assert_eq!(
            normalize_labels(vec![
                " deployer ".to_owned(),
                "".to_owned(),
                "oracle".to_owned(),
                "deployer".to_owned(),
            ]),
            vec!["deployer".to_owned(), "oracle".to_owned()]
        );
    }
}
