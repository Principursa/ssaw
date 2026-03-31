use std::fs;
use std::io::{Read, Write};
use std::iter;
use std::str::FromStr;

use age::secrecy::ExposeSecret;
use anyhow::{bail, Context, Result};
use bip39::{Language, Mnemonic};
use ethers_signers::{coins_bip39::English, MnemonicBuilder, Signer};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::config::{write_file, Paths};

#[derive(Debug, Clone)]
pub struct WalletSummary {
    pub address: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct SeedPayload {
    mnemonic: String,
    passphrase: Option<String>,
}

pub fn init(paths: &Paths) -> Result<(String, WalletSummary)> {
    if paths.seed_file.exists() {
        bail!(
            "seed file already exists at {}; use `ssaw address` or remove it before re-initializing",
            paths.seed_file.display()
        );
    }

    let mnemonic = Mnemonic::generate_in(Language::English, 24)
        .context("failed to generate mnemonic")?;
    let phrase = mnemonic.to_string();
    persist_phrase(paths, phrase.clone(), None)?;
    let address = derive_address(paths, 0)?;

    Ok((phrase, WalletSummary { address }))
}

pub fn import(paths: &Paths, phrase: String, passphrase: Option<String>) -> Result<WalletSummary> {
    let normalized = normalize_phrase(&phrase)?;
    persist_phrase(paths, normalized, passphrase)?;
    let address = derive_address(paths, 0)?;
    Ok(WalletSummary { address })
}

pub fn derive_address(paths: &Paths, index: u32) -> Result<String> {
    let payload = load_payload(paths)?;
    let mut builder = MnemonicBuilder::<English>::default().phrase(payload.mnemonic.as_str());
    if let Some(passphrase) = payload.passphrase.as_deref() {
        builder = builder.password(passphrase);
    }

    let derivation_path = format!("m/44'/60'/0'/0/{index}");
    let wallet = builder
        .derivation_path(&derivation_path)
        .context("failed to build derivation path")?
        .build()
        .context("failed to derive wallet from mnemonic")?;

    Ok(format!("{:#x}", wallet.address()))
}

pub fn ensure_identity(paths: &Paths) -> Result<(std::path::PathBuf, String)> {
    let identity_path = paths.identity_file()?;
    if identity_path.exists() {
        let public = load_identity(&identity_path)?
            .to_public()
            .to_string();
        return Ok((identity_path, public));
    }

    let identity = age::x25519::Identity::generate();
    write_file(
        &identity_path,
        format!("{}\n", identity.to_string().expose_secret()),
    )?;
    let public = identity.to_public().to_string();
    Ok((identity_path, public))
}

pub fn read_phrase_from_stdin() -> Result<String> {
    let mut buffer = String::new();
    std::io::stdin()
        .read_to_string(&mut buffer)
        .context("failed to read mnemonic from stdin")?;

    if buffer.trim().is_empty() {
        bail!("mnemonic input was empty");
    }

    Ok(buffer)
}

pub fn read_secret_line(prompt: &str) -> Result<String> {
    let mut stdout = std::io::stdout();
    stdout.write_all(prompt.as_bytes()).context("failed to write prompt")?;
    stdout.flush().context("failed to flush prompt")?;

    let value = rpassword::read_password().context("failed to read secret input")?;
    Ok(value)
}

fn persist_phrase(paths: &Paths, phrase: String, passphrase: Option<String>) -> Result<()> {
    paths.ensure_parent_dirs()?;
    let identity_path = paths.identity_file()?;
    let identity = if identity_path.exists() {
        load_identity(&identity_path)?
    } else {
        ensure_identity(paths)?;
        load_identity(&identity_path)?
    };

    let payload = SeedPayload { mnemonic: phrase, passphrase };
    let body = toml::to_string(&payload).context("failed to serialize seed payload")?;
    let recipient = identity.to_public();
    let encryptor = age::Encryptor::with_recipients(iter::once(&recipient as &dyn age::Recipient))
        .context("failed to initialize age encryptor")?;

    let mut encrypted = Vec::new();
    {
        let mut writer = encryptor
            .wrap_output(&mut encrypted)
            .context("failed to wrap encrypted output")?;
        writer
            .write_all(body.as_bytes())
            .context("failed to encrypt seed payload")?;
        writer.finish().context("failed to finalize seed payload")?;
    }

    write_file(&paths.seed_file, encrypted)
}

fn load_payload(paths: &Paths) -> Result<SeedPayload> {
    let identity_path = paths.identity_file()?;
    let identity = load_identity(&identity_path)?;
    let encrypted = fs::read(&paths.seed_file)
        .with_context(|| format!("failed to read {}", paths.seed_file.display()))?;
    let decryptor =
        age::Decryptor::new_buffered(&encrypted[..]).context("failed to parse age file")?;
    let mut reader = decryptor
        .decrypt(iter::once(&identity as &dyn age::Identity))
        .context("failed to decrypt seed file with local identity")?;
    let mut decrypted = String::new();
    reader
        .read_to_string(&mut decrypted)
        .context("failed to decode decrypted seed payload")?;

    let payload: SeedPayload =
        toml::from_str(&decrypted).context("failed to parse decrypted seed payload")?;
    Ok(payload)
}

fn load_identity(path: &std::path::Path) -> Result<age::x25519::Identity> {
    let raw = Zeroizing::new(
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?,
    );
    let trimmed = raw.trim();
    age::x25519::Identity::from_str(trimmed)
        .map_err(|error| anyhow::anyhow!(error))
        .with_context(|| format!("failed to parse age identity in {}", path.display()))
}

fn normalize_phrase(phrase: &str) -> Result<String> {
    let mnemonic = Mnemonic::parse_in_normalized(Language::English, phrase.trim())
        .context("mnemonic is not valid BIP-39 English")?;
    Ok(mnemonic.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_phrase() {
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let normalized = normalize_phrase(phrase).expect("normalize phrase");
        assert_eq!(normalized, phrase);
    }
}
