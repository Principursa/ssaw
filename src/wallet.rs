use std::fs;
use std::io::{Read, Write};
use std::iter;
use std::path::Path;
use std::str::FromStr;

use age::secrecy::ExposeSecret;
use anyhow::{bail, Context, Result};
use bip39::{Language, Mnemonic};
use coins_bip32::{path::DerivationPath, xkeys::XPriv};
use ethers_core::types::transaction::eip2718::TypedTransaction;
use ethers_core::types::transaction::eip712::{Eip712, TypedData};
use ethers_core::types::{
    Address, Bytes, NameOrAddress, Signature as EthereumSignature, TransactionRequest, H256, U256,
};
use hex::ToHex;
use k256::ecdsa::SigningKey;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha3::{Digest, Keccak256};
use zeroize::{Zeroize, Zeroizing};

use crate::chain::{self, ChainSelector};
use crate::config::{write_file, Paths};
use crate::rpc::RpcClient;

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
    Ok(address_from_signing_key(&signer))
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
    let digest = ethereum_message_digest(message.as_bytes());
    let signature = sign_digest(&signer, digest)?;

    Ok(SignatureOutput {
        address: address_from_signing_key(&signer),
        signature,
    })
}

pub fn sign_typed_data(paths: &Paths, typed_data_json: &str, index: u32) -> Result<SignatureOutput> {
    let signer = signer_for_index(paths, index)?;
    let typed_data: TypedData =
        serde_json::from_str(typed_data_json).context("failed to parse typed data JSON")?;
    let digest = typed_data
        .encode_eip712()
        .context("failed to encode typed data as EIP-712")?;
    let signature = sign_digest(&signer, digest)?;

    Ok(SignatureOutput {
        address: address_from_signing_key(&signer),
        signature,
    })
}

pub fn send_transaction(
    paths: &Paths,
    selector: &ChainSelector,
    to: &str,
    value_wei: &str,
    data: Option<&str>,
    index: u32,
) -> Result<SentTransaction> {
    let chain = chain::resolve(paths, selector)?;
    let rpc = RpcClient::new(chain.rpc_url.clone());
    let signer = signer_for_index(paths, index)?;
    let from = parse_address(&address_from_signing_key(&signer))
        .context("failed to parse local signer address")?;
    let to = parse_address(to).context("failed to parse recipient address")?;
    let value =
        U256::from_dec_str(value_wei).context("failed to parse value_wei as decimal string")?;

    let nonce: U256 = rpc.request("eth_getTransactionCount", json!([from, "pending"]))?;
    let gas_price: U256 = rpc.request("eth_gasPrice", json!([]))?;

    let mut tx = TransactionRequest::new()
        .from(from)
        .to(NameOrAddress::Address(to))
        .value(value)
        .nonce(nonce)
        .gas_price(gas_price)
        .chain_id(chain.chain_id);

    if let Some(data) = data {
        let bytes = parse_hex_bytes(data).context("failed to parse transaction data hex")?;
        tx = tx.data(bytes);
    }

    let estimate: U256 = rpc.request("eth_estimateGas", json!([tx.clone()]))?;
    tx = tx.gas(estimate);

    let mut typed_tx = TypedTransaction::Legacy(tx);
    typed_tx.set_chain_id(chain.chain_id);

    let signature = sign_transaction(&signer, &typed_tx, chain.chain_id)?;
    let tx_hash: H256 = rpc.request(
        "eth_sendRawTransaction",
        json!([format!("0x{}", hex::encode(typed_tx.rlp_signed(&signature)))])
    )?;

    Ok(SentTransaction {
        tx_hash: format!("{tx_hash:#x}"),
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

fn signer_for_index(paths: &Paths, index: u32) -> Result<SigningKey> {
    let payload = load_payload(paths)?;
    let derivation_path = derivation_path(index)?;
    let mnemonic = Mnemonic::parse_in_normalized(Language::English, &payload.mnemonic)
        .context("stored mnemonic is not valid BIP-39 English")?;

    let mut seed = match payload.passphrase {
        Some(passphrase) => Zeroizing::new(mnemonic.to_seed(passphrase)),
        None => Zeroizing::new(mnemonic.to_seed_normalized("")),
    };

    let xpriv = XPriv::root_from_seed(seed.as_slice(), None).context("failed to derive root key")?;
    seed.as_mut_slice().zeroize();

    let derived = xpriv
        .derive_path(&derivation_path)
        .context("failed to derive child key")?;
    let derived_signer: &SigningKey = derived.as_ref();
    let secret = derived_signer.to_bytes();
    SigningKey::from_bytes(&secret).context("failed to build signing key")
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

fn derivation_path(index: u32) -> Result<DerivationPath> {
    format!("m/44'/60'/0'/0/{index}")
        .parse::<DerivationPath>()
        .context("failed to parse derivation path")
}

fn address_from_signing_key(signing_key: &SigningKey) -> String {
    let verify_key = signing_key.verifying_key();
    let encoded = verify_key.to_encoded_point(false);
    let public_key = encoded.as_bytes();
    let digest = Keccak256::digest(&public_key[1..]);
    format!("0x{}", hex::encode(&digest[12..]))
}

fn ethereum_message_digest(message: &[u8]) -> [u8; 32] {
    let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
    let mut hasher = Keccak256::new();
    hasher.update(prefix.as_bytes());
    hasher.update(message);
    hasher.finalize().into()
}

fn sign_digest(signing_key: &SigningKey, digest: [u8; 32]) -> Result<String> {
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(&digest)
        .context("failed to sign digest")?;
    let mut encoded = [0u8; 65];
    encoded[..64].copy_from_slice(signature.to_bytes().as_slice());
    encoded[64] = recovery_id.to_byte() + 27;
    Ok(format!("0x{}", encoded.encode_hex::<String>()))
}

fn sign_transaction(
    signing_key: &SigningKey,
    tx: &TypedTransaction,
    chain_id: u64,
) -> Result<EthereumSignature> {
    let sighash = tx.sighash();
    let (signature, recovery_id) = signing_key
        .sign_prehash_recoverable(sighash.as_bytes())
        .context("failed to sign transaction sighash")?;

    let r = U256::from_big_endian(signature.r().to_bytes().as_slice());
    let s = U256::from_big_endian(signature.s().to_bytes().as_slice());
    let v = u64::from(recovery_id.to_byte()) + chain_id * 2 + 35;

    Ok(EthereumSignature { r, s, v })
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
        let mnemonic = Mnemonic::parse_in_normalized(
            Language::English,
            "test test test test test test test test test test test junk",
        )
        .expect("parse mnemonic");
        let seed = mnemonic.to_seed_normalized("");
        let xpriv = XPriv::root_from_seed(&seed, None).expect("derive root");
        let derived = xpriv
            .derive_path("m/44'/60'/0'/0/0")
            .expect("derive child");
        let address = address_from_signing_key(derived.as_ref());
        assert_eq!(address, "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");
    }

    #[test]
    fn signs_message_with_65_byte_hex_signature() {
        let mnemonic = Mnemonic::parse_in_normalized(
            Language::English,
            "test test test test test test test test test test test junk",
        )
        .expect("parse mnemonic");
        let seed = mnemonic.to_seed_normalized("");
        let xpriv = XPriv::root_from_seed(&seed, None).expect("derive root");
        let derived = xpriv
            .derive_path("m/44'/60'/0'/0/0")
            .expect("derive child");
        let signature = sign_digest(derived.as_ref(), ethereum_message_digest(b"hello"))
            .expect("sign message");
        assert_eq!(signature.len(), 132);
        assert!(signature.starts_with("0x"));
    }
}
