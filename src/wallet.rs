use std::fs;
use std::io::{Read, Write};
use std::iter;
use std::path::Path;
use std::str::FromStr;

use age::secrecy::ExposeSecret;
use alloy::{
    network::TransactionBuilder,
    primitives::{Address, Bytes, U256},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
};
use alloy_dyn_abi::eip712::TypedData;
use alloy_signer::SignerSync;
use alloy_signer_local::{coins_bip39::English, MnemonicBuilder};
use anyhow::{bail, Context, Result};
use bip39::{Language, Mnemonic};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::chain::{self, ChainSelector};
use crate::config::{write_file, Paths};

const DEFAULT_ADDRESS_COUNT: u32 = 5;
const MAX_ADDRESS_COUNT: u32 = 20;

#[derive(Debug, Clone)]
pub struct WalletSummary {
    pub address: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DerivedAddress {
    pub index: u32,
    pub address: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignatureOutput {
    pub address: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SentTransaction {
    pub tx_hash: String,
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
    let signer = signer_for_index(paths, index)?;
    Ok(format!("{:#x}", signer.address()))
}

pub fn list_addresses(paths: &Paths, count: Option<u32>) -> Result<Vec<DerivedAddress>> {
    let count = count.unwrap_or(DEFAULT_ADDRESS_COUNT).min(MAX_ADDRESS_COUNT);
    (0..count)
        .map(|index| {
            derive_address(paths, index).map(|address| DerivedAddress { index, address })
        })
        .collect()
}

pub fn sign_message(paths: &Paths, message: &str, index: u32) -> Result<SignatureOutput> {
    let signer = signer_for_index(paths, index)?;
    let signature = signer
        .sign_message_sync(message.as_bytes())
        .context("failed to sign message")?;

    Ok(SignatureOutput {
        address: format!("{:#x}", signer.address()),
        signature: signature.to_string(),
    })
}

pub fn sign_typed_data(paths: &Paths, typed_data_json: &str, index: u32) -> Result<SignatureOutput> {
    let signer = signer_for_index(paths, index)?;
    let typed_data: TypedData =
        serde_json::from_str(typed_data_json).context("failed to parse typed data JSON")?;
    let signature = signer
        .sign_dynamic_typed_data_sync(&typed_data)
        .context("failed to sign typed data")?;

    Ok(SignatureOutput {
        address: format!("{:#x}", signer.address()),
        signature: signature.to_string(),
    })
}

pub async fn send_transaction(
    paths: &Paths,
    selector: &ChainSelector,
    to: &str,
    value_wei: &str,
    data: Option<&str>,
    index: u32,
) -> Result<SentTransaction> {
    let chain = chain::resolve(paths, selector)?;
    let signer = signer_for_index(paths, index)?;
    let rpc_url = chain
        .rpc_url
        .parse()
        .with_context(|| format!("invalid rpc url `{}`", chain.rpc_url))?;
    let provider = ProviderBuilder::new().wallet(signer).connect_http(rpc_url);

    let mut tx = TransactionRequest::default()
        .with_to(parse_address(to)?)
        .with_value(parse_u256_dec(value_wei)?)
        .with_chain_id(chain.chain_id);

    if let Some(data) = data {
        tx = tx.with_input(parse_hex_bytes(data)?);
    }

    let pending = provider
        .send_transaction(tx)
        .await
        .context("failed to send transaction")?;

    Ok(SentTransaction {
        tx_hash: format!("{:#x}", pending.tx_hash()),
    })
}

pub fn ensure_identity(paths: &Paths) -> Result<(std::path::PathBuf, String)> {
    let identity_path = paths.identity_file()?;
    if identity_path.exists() {
        let public = load_identity(&identity_path)?.to_public().to_string();
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
    stdout
        .write_all(prompt.as_bytes())
        .context("failed to write prompt")?;
    stdout.flush().context("failed to flush prompt")?;

    let value = rpassword::read_password().context("failed to read secret input")?;
    Ok(value)
}

fn signer_for_index(paths: &Paths, index: u32) -> Result<PrivateKeySigner> {
    let payload = load_payload(paths)?;
    let mut builder = MnemonicBuilder::<English>::default()
        .phrase(payload.mnemonic)
        .index(index)
        .context("failed to apply mnemonic index")?;

    if let Some(passphrase) = payload.passphrase {
        builder = builder.password(passphrase);
    }

    builder.build().context("failed to derive signer from mnemonic")
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
        writer
            .finish()
            .context("failed to finalize seed payload")?;
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

fn load_identity(path: &Path) -> Result<age::x25519::Identity> {
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

fn parse_address(value: &str) -> Result<Address> {
    value
        .parse::<Address>()
        .with_context(|| format!("invalid address `{value}`"))
}

fn parse_hex_bytes(value: &str) -> Result<Bytes> {
    let trimmed = value.strip_prefix("0x").unwrap_or(value);
    let bytes = hex::decode(trimmed).with_context(|| format!("invalid hex data `{value}`"))?;
    Ok(Bytes::from(bytes))
}

fn parse_u256_dec(value: &str) -> Result<U256> {
    U256::from_str_radix(value, 10).with_context(|| format!("invalid decimal U256 `{value}`"))
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

    #[test]
    fn derives_known_address() {
        let signer = MnemonicBuilder::<English>::default()
            .phrase("test test test test test test test test test test test junk")
            .build()
            .expect("build signer");
        assert_eq!(format!("{:#x}", signer.address()), "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");
    }

    #[test]
    fn signs_message_with_65_byte_hex_signature() {
        let signer = MnemonicBuilder::<English>::default()
            .phrase("test test test test test test test test test test test junk")
            .build()
            .expect("build signer");
        let signature = signer
            .sign_message_sync(b"hello")
            .expect("sign message")
            .to_string();
        assert!(signature.starts_with("0x"));
        assert_eq!(signature.len(), 132);
    }
}
