use std::collections::BTreeMap;
use std::fs;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::{Paths, write_file};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChainEntry {
    pub chain_id: u64,
    pub rpc_url: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ChainConfig {
    pub chains: BTreeMap<String, ChainEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChainSelector {
    Name(String),
    ChainId(u64),
}

pub fn load(paths: &Paths) -> Result<ChainConfig> {
    if !paths.chains_file.exists() {
        return Ok(ChainConfig::default());
    }

    let raw = fs::read_to_string(&paths.chains_file)
        .with_context(|| format!("failed to read {}", paths.chains_file.display()))?;
    Ok(toml::from_str(&raw)
        .with_context(|| format!("failed to parse {}", paths.chains_file.display()))?)
}

pub fn add_chain(paths: &Paths, name: &str, chain_id: u64, rpc_url: String) -> Result<()> {
    let mut config = load(paths)?;

    if rpc_url.trim().is_empty() {
        bail!("rpc url cannot be empty");
    }

    config.chains.insert(
        name.to_owned(),
        ChainEntry {
            chain_id,
            rpc_url: rpc_url.trim().to_owned(),
        },
    );

    let body = toml::to_string_pretty(&config).context("failed to serialize chain config")?;
    write_file(&paths.chains_file, body)
}

pub fn resolve(paths: &Paths, selector: &ChainSelector) -> Result<ChainEntry> {
    let config = load(paths)?;

    match selector {
        ChainSelector::Name(name) => config
            .chains
            .get(name)
            .cloned()
            .with_context(|| format!("unknown chain `{name}`")),
        ChainSelector::ChainId(chain_id) => config
            .chains
            .values()
            .find(|entry| entry.chain_id == *chain_id)
            .cloned()
            .with_context(|| format!("unknown chain id `{chain_id}`")),
    }
}

impl ChainSelector {
    pub fn parse(value: &str) -> Self {
        match value.parse::<u64>() {
            Ok(chain_id) => Self::ChainId(chain_id),
            Err(_) => Self::Name(value.to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_chain_config() {
        let mut config = ChainConfig::default();
        config.chains.insert(
            "local".to_owned(),
            ChainEntry {
                chain_id: 31337,
                rpc_url: "http://localhost:8545".to_owned(),
            },
        );

        let body = toml::to_string_pretty(&config).expect("serialize config");
        assert!(body.contains("[chains.local]"));
        assert!(body.contains("chain_id = 31337"));
    }

    #[test]
    fn parses_chain_selector() {
        assert_eq!(
            ChainSelector::parse("base-sepolia"),
            ChainSelector::Name("base-sepolia".to_owned())
        );
        assert_eq!(ChainSelector::parse("84532"), ChainSelector::ChainId(84532));
    }
}
