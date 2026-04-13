use std::fs;
use std::fs::File;
use std::io::{Read, Write};
use std::iter;
use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use age::secrecy::ExposeSecret;
use alloy::{
    network::ReceiptResponse,
    network::TransactionBuilder,
    primitives::{Address, Bytes, U256},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
};
use alloy_contract::Interface;
use alloy_dyn_abi::eip712::TypedData;
use alloy_dyn_abi::{DynSolValue, Specifier};
use alloy_json_abi::{Function, JsonAbi};
use alloy_signer::SignerSync;
use alloy_signer_local::{MnemonicBuilder, coins_bip39::English};
use anyhow::{Context, Result, bail};
use bip39::{Language, Mnemonic};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use zeroize::Zeroizing;

use crate::chain::{self, ChainSelector};
use crate::config::{Paths, write_file};

const DEFAULT_ADDRESS_COUNT: u32 = 5;
const MAX_ADDRESS_COUNT: u32 = 20;

#[derive(Debug, Clone)]
pub struct WalletSummary {
    pub address: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AddressTarget {
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DerivedAddress {
    pub index: u32,
    pub address: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignatureOutput {
    pub address: String,
    pub signature: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SentTransaction {
    pub tx_hash: String,
    pub confirmed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<TransactionReceiptSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransactionReceiptSummary {
    pub tx_hash: String,
    pub status: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_number: Option<u64>,
    pub gas_used: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct WaitOptions {
    pub wait: bool,
    pub timeout_secs: u64,
}

impl WaitOptions {
    pub fn from_flag(wait: bool, timeout_secs: u64) -> Self {
        Self { wait, timeout_secs }
    }
}

pub(crate) struct WriteGuard {
    file: File,
}

impl WriteGuard {
    pub(crate) fn acquire(paths: &Paths) -> Result<Self> {
        paths.ensure_parent_dirs()?;
        let file = File::create(&paths.lock_file)
            .with_context(|| format!("failed to open {}", paths.lock_file.display()))?;
        file.lock_exclusive()
            .with_context(|| format!("failed to lock {}", paths.lock_file.display()))?;
        Ok(Self { file })
    }
}

impl Drop for WriteGuard {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ContractReadOutput {
    pub outputs: Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct SeedPayload {
    mnemonic: String,
    #[serde(default)]
    passphrase_required: bool,
}

#[derive(Debug)]
struct LoadedSeedPayload {
    mnemonic: Zeroizing<String>,
    passphrase_required: bool,
}

pub fn init(paths: &Paths) -> Result<(String, WalletSummary)> {
    if paths.seed_file.exists() {
        bail!(
            "seed file already exists at {}; use `ssaw address` or remove it before re-initializing",
            paths.seed_file.display()
        );
    }

    let mnemonic =
        Mnemonic::generate_in(Language::English, 24).context("failed to generate mnemonic")?;
    let phrase = mnemonic.to_string();
    persist_phrase(paths, phrase.as_str(), false)?;
    let address = derive_address(paths, 0, None)?;

    Ok((phrase, WalletSummary { address }))
}

pub fn import(paths: &Paths, phrase: &str, passphrase: Option<&str>) -> Result<WalletSummary> {
    let normalized = normalize_phrase(&phrase)?;
    persist_phrase(paths, normalized.as_str(), passphrase.is_some())?;
    let address = derive_address(paths, 0, passphrase)?;
    Ok(WalletSummary { address })
}

pub fn derive_address(
    paths: &Paths,
    index: u32,
    runtime_passphrase: Option<&str>,
) -> Result<String> {
    let signer = signer_for_index(paths, index, runtime_passphrase)?;
    Ok(format!("{:#x}", signer.address()))
}

pub fn resolve_address_target(
    paths: &Paths,
    index: Option<u32>,
    alias: Option<&str>,
) -> Result<AddressTarget> {
    match (index, alias) {
        (Some(index), None) => Ok(AddressTarget { index, alias: None }),
        (None, Some(alias)) => Ok(AddressTarget {
            index: crate::alias::resolve_alias(paths, alias)?,
            alias: Some(alias.to_owned()),
        }),
        (None, None) => Ok(AddressTarget {
            index: 0,
            alias: None,
        }),
        (Some(_), Some(_)) => bail!("use either index or alias, not both"),
    }
}

pub fn list_addresses(
    paths: &Paths,
    count: Option<u32>,
    runtime_passphrase: Option<&str>,
) -> Result<Vec<DerivedAddress>> {
    let count = count
        .unwrap_or(DEFAULT_ADDRESS_COUNT)
        .min(MAX_ADDRESS_COUNT);
    (0..count)
        .map(|index| {
            let aliases = crate::alias::aliases_for_index(paths, index)?
                .into_iter()
                .map(|entry| entry.name)
                .collect();
            derive_address(paths, index, runtime_passphrase).map(|address| DerivedAddress {
                index,
                address,
                aliases,
            })
        })
        .collect()
}

pub fn aliases_for_index(paths: &Paths, index: u32) -> Result<Vec<String>> {
    Ok(crate::alias::aliases_for_index(paths, index)?
        .into_iter()
        .map(|entry| entry.name)
        .collect())
}

pub fn sign_message(
    paths: &Paths,
    message: &str,
    index: u32,
    runtime_passphrase: Option<&str>,
) -> Result<SignatureOutput> {
    let signer = signer_for_index(paths, index, runtime_passphrase)?;
    let signature = signer
        .sign_message_sync(message.as_bytes())
        .context("failed to sign message")?;

    Ok(SignatureOutput {
        address: format!("{:#x}", signer.address()),
        signature: signature.to_string(),
    })
}

pub fn sign_typed_data(
    paths: &Paths,
    typed_data_json: &str,
    index: u32,
    runtime_passphrase: Option<&str>,
) -> Result<SignatureOutput> {
    let signer = signer_for_index(paths, index, runtime_passphrase)?;
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
    wait: WaitOptions,
    index: u32,
    runtime_passphrase: Option<&str>,
) -> Result<SentTransaction> {
    let _guard = WriteGuard::acquire(paths)?;
    let chain = chain::resolve(paths, selector)?;
    let signer = signer_for_index(paths, index, runtime_passphrase)?;
    let rpc_url = chain
        .rpc_url
        .parse()
        .context("invalid rpc url in configured chain")?;
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

    finalize_pending_transaction(pending, wait).await
}

pub async fn read_contract(
    paths: &Paths,
    selector: &ChainSelector,
    address: &str,
    abi_json: &str,
    function: &str,
    args: &[String],
) -> Result<ContractReadOutput> {
    let chain = chain::resolve(paths, selector)?;
    let rpc_url = chain
        .rpc_url
        .parse()
        .context("invalid rpc url in configured chain")?;
    let provider = ProviderBuilder::new().connect_http(rpc_url);
    let interface = parse_interface(abi_json)?;
    let contract =
        interface.connect::<_, alloy::network::Ethereum>(parse_address(address)?, provider);
    let resolved_function = resolve_contract_function(contract.abi(), function)?;
    let values = parse_contract_args(resolved_function, args)?;
    let outputs = contract
        .function_from_selector(&resolved_function.selector(), &values)
        .with_context(|| format!("failed to prepare function `{function}`"))?
        .call()
        .await
        .with_context(|| format!("failed to call function `{function}`"))?;

    Ok(ContractReadOutput {
        outputs: dyn_values_to_json(&outputs),
    })
}

pub async fn write_contract(
    paths: &Paths,
    selector: &ChainSelector,
    address: &str,
    abi_json: &str,
    function: &str,
    args: &[String],
    value_wei: Option<&str>,
    wait: WaitOptions,
    index: u32,
    runtime_passphrase: Option<&str>,
) -> Result<SentTransaction> {
    let _guard = WriteGuard::acquire(paths)?;
    let chain = chain::resolve(paths, selector)?;
    let signer = signer_for_index(paths, index, runtime_passphrase)?;
    let rpc_url = chain
        .rpc_url
        .parse()
        .context("invalid rpc url in configured chain")?;
    let provider = ProviderBuilder::new().wallet(signer).connect_http(rpc_url);
    let interface = parse_interface(abi_json)?;
    let contract =
        interface.connect::<_, alloy::network::Ethereum>(parse_address(address)?, provider);
    let resolved_function = resolve_contract_function(contract.abi(), function)?;
    let values = parse_contract_args(resolved_function, args)?;
    let mut call = contract
        .function_from_selector(&resolved_function.selector(), &values)
        .with_context(|| format!("failed to prepare function `{function}`"))?;

    if let Some(value_wei) = value_wei {
        call = call.value(parse_u256_dec(value_wei)?);
    }

    let pending = call
        .send()
        .await
        .with_context(|| format!("failed to send contract call `{function}`"))?;

    finalize_pending_transaction(pending, wait).await
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
        .context("failed to read stdin input")?;

    if buffer.trim().is_empty() {
        bail!("stdin input was empty");
    }

    Ok(buffer)
}

pub fn read_secret_from_stdin() -> Result<Zeroizing<String>> {
    let mut buffer = Zeroizing::new(String::new());
    std::io::stdin()
        .read_to_string(&mut buffer)
        .context("failed to read secret input from stdin")?;

    if buffer.trim().is_empty() {
        bail!("secret input was empty");
    }

    Ok(buffer)
}

pub fn read_secret_line(prompt: &str) -> Result<Zeroizing<String>> {
    let mut stdout = std::io::stdout();
    stdout
        .write_all(prompt.as_bytes())
        .context("failed to write prompt")?;
    stdout.flush().context("failed to flush prompt")?;

    let value = rpassword::read_password().context("failed to read secret input")?;
    Ok(Zeroizing::new(value))
}

pub(crate) fn signer_for_index(
    paths: &Paths,
    index: u32,
    runtime_passphrase: Option<&str>,
) -> Result<PrivateKeySigner> {
    let payload = load_payload(paths)?;
    let bip39_passphrase = select_bip39_passphrase(paths, &payload, runtime_passphrase)?;
    let mut builder = MnemonicBuilder::<English>::default()
        .phrase(payload.mnemonic.as_str())
        .index(index)
        .context("failed to apply mnemonic index")?;

    if let Some(passphrase) = bip39_passphrase {
        builder = builder.password(passphrase);
    }

    builder
        .build()
        .context("failed to derive signer from mnemonic")
}

fn persist_phrase(paths: &Paths, phrase: &str, passphrase_required: bool) -> Result<()> {
    paths.ensure_parent_dirs()?;
    let identity_path = paths.identity_file()?;
    let identity = if identity_path.exists() {
        load_identity(&identity_path)?
    } else {
        ensure_identity(paths)?;
        load_identity(&identity_path)?
    };

    let payload = SeedPayload {
        mnemonic: phrase.to_owned(),
        passphrase_required,
    };
    let body =
        Zeroizing::new(toml::to_string(&payload).context("failed to serialize seed payload")?);
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

fn load_payload(paths: &Paths) -> Result<LoadedSeedPayload> {
    if !paths.seed_file.exists() {
        bail!(
            "project `{}` has no wallet seed yet; run `ssaw --project {} init` or `ssaw --project {} import`",
            paths.project_name,
            paths.project_name,
            paths.project_name
        );
    }

    let identity_path = paths.identity_file()?;
    let identity = load_identity(&identity_path)?;
    let encrypted = fs::read(&paths.seed_file)
        .with_context(|| format!("failed to read {}", paths.seed_file.display()))?;
    let decryptor =
        age::Decryptor::new_buffered(&encrypted[..]).context("failed to parse age file")?;
    let mut reader = decryptor
        .decrypt(iter::once(&identity as &dyn age::Identity))
        .context("failed to decrypt seed file with local identity")?;
    let mut decrypted = Zeroizing::new(String::new());
    reader
        .read_to_string(&mut decrypted)
        .context("failed to decode decrypted seed payload")?;

    let payload: SeedPayload =
        toml::from_str(decrypted.as_str()).context("failed to parse decrypted seed payload")?;
    Ok(LoadedSeedPayload::from(payload))
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

impl From<SeedPayload> for LoadedSeedPayload {
    fn from(payload: SeedPayload) -> Self {
        Self {
            mnemonic: Zeroizing::new(payload.mnemonic),
            passphrase_required: payload.passphrase_required,
        }
    }
}

fn select_bip39_passphrase<'a>(
    paths: &Paths,
    payload: &'a LoadedSeedPayload,
    runtime_passphrase: Option<&'a str>,
) -> Result<Option<&'a str>> {
    if payload.passphrase_required {
        let message = format!(
            "project `{}` requires a BIP-39 passphrase; rerun with --prompt-passphrase or restart `ssaw serve` with --prompt-passphrase",
            paths.project_name
        );
        return runtime_passphrase.map(Some).with_context(|| message);
    }

    Ok(None)
}

pub fn passphrase_required(paths: &Paths) -> Result<bool> {
    Ok(load_payload(paths)?.passphrase_required)
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

async fn finalize_pending_transaction<N: alloy::network::Network>(
    pending: alloy::providers::PendingTransactionBuilder<N>,
    wait: WaitOptions,
) -> Result<SentTransaction> {
    let tx_hash = format!("{:#x}", pending.tx_hash());
    if !wait.wait {
        return Ok(SentTransaction {
            tx_hash,
            confirmed: false,
            receipt: None,
        });
    }

    let receipt = pending
        .with_timeout(Some(Duration::from_secs(wait.timeout_secs)))
        .get_receipt()
        .await
        .context("failed waiting for transaction receipt")?;

    Ok(SentTransaction {
        tx_hash,
        confirmed: true,
        receipt: Some(TransactionReceiptSummary {
            tx_hash: format!("{:#x}", receipt.transaction_hash()),
            status: receipt.status(),
            block_number: receipt.block_number(),
            gas_used: receipt.gas_used(),
        }),
    })
}

fn parse_interface(abi_json: &str) -> Result<Interface> {
    let abi: JsonAbi = serde_json::from_str(abi_json).context("failed to parse ABI JSON")?;
    Ok(Interface::new(abi))
}

fn resolve_contract_function<'a>(abi: &'a JsonAbi, requested: &str) -> Result<&'a Function> {
    let requested = requested.trim();
    if requested.is_empty() {
        bail!("function name or signature cannot be empty");
    }

    if requested.contains('(') {
        let parsed = Function::parse(requested)
            .with_context(|| format!("invalid function signature `{requested}`"))?;
        let signature = parsed.signature();
        return abi
            .functions()
            .find(|function| function.signature() == signature)
            .with_context(|| missing_signature_error(abi, requested, &parsed.name));
    }

    let candidates = abi
        .function(requested)
        .with_context(|| format!("function `{requested}` not found in ABI"))?;
    if candidates.len() == 1 {
        return Ok(&candidates[0]);
    }

    bail!(
        "function `{requested}` is overloaded; use a full signature: {}",
        format_signatures(candidates)
    );
}

fn missing_signature_error(abi: &JsonAbi, requested: &str, name: &str) -> String {
    match abi.function(name) {
        Some(candidates) => format!(
            "function signature `{requested}` not found in ABI; available signatures: {}",
            format_signatures(candidates)
        ),
        None => format!("function signature `{requested}` not found in ABI"),
    }
}

fn format_signatures(candidates: &[Function]) -> String {
    candidates
        .iter()
        .map(Function::signature)
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_contract_args(function: &Function, args: &[String]) -> Result<Vec<DynSolValue>> {
    if function.inputs.len() != args.len() {
        bail!(
            "function `{}` expects {} argument(s), got {}",
            function.signature(),
            function.inputs.len(),
            args.len()
        );
    }

    function
        .inputs
        .iter()
        .zip(args)
        .map(|(param, arg)| {
            let ty = param
                .resolve()
                .context("failed to resolve function argument type")?;
            ty.coerce_str(arg)
                .with_context(|| format!("failed to parse argument `{arg}` as `{}`", param.ty))
        })
        .collect()
}

fn dyn_values_to_json(values: &[DynSolValue]) -> Value {
    Value::Array(values.iter().map(dyn_value_to_json).collect())
}

fn dyn_value_to_json(value: &DynSolValue) -> Value {
    match value {
        DynSolValue::Bool(inner) => Value::Bool(*inner),
        DynSolValue::Int(inner, _) => Value::String(inner.to_string()),
        DynSolValue::Uint(inner, _) => Value::String(inner.to_string()),
        DynSolValue::FixedBytes(inner, size) => {
            Value::String(format!("0x{}", hex::encode(&inner[..*size])))
        }
        DynSolValue::Address(inner) => Value::String(format!("{:#x}", inner)),
        DynSolValue::Function(inner) => Value::String(format!("{:#x}", inner)),
        DynSolValue::Bytes(inner) => Value::String(format!("0x{}", hex::encode(inner))),
        DynSolValue::String(inner) => Value::String(inner.clone()),
        DynSolValue::Array(inner) | DynSolValue::FixedArray(inner) | DynSolValue::Tuple(inner) => {
            Value::Array(inner.iter().map(dyn_value_to_json).collect())
        }
        DynSolValue::CustomStruct {
            name,
            prop_names,
            tuple,
        } => {
            let mut object = Map::new();
            object.insert("_type".to_owned(), Value::String(name.clone()));
            for (prop_name, prop_value) in prop_names.iter().zip(tuple.iter()) {
                object.insert(prop_name.clone(), dyn_value_to_json(prop_value));
            }
            Value::Object(object)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    use tempfile::TempDir;

    fn temp_paths(temp: &TempDir, project_name: &str) -> Paths {
        let state_dir = temp.path().join(".ssaw");
        let config_dir = temp.path().join(".config").join("ssaw");
        let projects_dir = state_dir.join("projects");
        let project_dir = if project_name == "default" {
            state_dir.clone()
        } else {
            projects_dir.join(project_name)
        };

        Paths {
            project_name: project_name.to_owned(),
            state_dir: state_dir.clone(),
            project_dir: project_dir.clone(),
            projects_dir,
            config_dir: config_dir.clone(),
            current_project_file: state_dir.join("current-project"),
            seed_file: project_dir.join("seed.age"),
            chains_file: project_dir.join("chains.toml"),
            addresses_file: project_dir.join("addresses.toml"),
            lock_file: project_dir.join("wallet.lock"),
            config_file: config_dir.join("config.toml"),
            default_identity_file: config_dir.join("identity.txt"),
        }
    }

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
        assert_eq!(
            format!("{:#x}", signer.address()),
            "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"
        );
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

    #[test]
    fn parses_contract_args_from_strings() {
        let abi: JsonAbi = serde_json::from_str(
            r#"[{"type":"function","name":"transfer","inputs":[{"name":"to","type":"address"},{"name":"amount","type":"uint256"}],"outputs":[]}]"#,
        )
        .expect("abi");
        let function = resolve_contract_function(&abi, "transfer").expect("function");

        let args = parse_contract_args(
            function,
            &[
                "0x000000000000000000000000000000000000dead".to_owned(),
                "42".to_owned(),
            ],
        )
        .expect("args");

        assert_eq!(args.len(), 2);
        assert_eq!(
            dyn_values_to_json(&args),
            Value::Array(vec![
                Value::String("0x000000000000000000000000000000000000dead".to_owned()),
                Value::String("42".to_owned()),
            ])
        );
    }

    #[test]
    fn renders_dynamic_values_as_json() {
        let values = vec![
            DynSolValue::Bool(true),
            DynSolValue::Uint(U256::from(7u8), 256),
            DynSolValue::Tuple(vec![
                DynSolValue::String("hello".to_owned()),
                DynSolValue::Address(
                    parse_address("0x000000000000000000000000000000000000dead").expect("address"),
                ),
            ]),
        ];

        assert_eq!(
            dyn_values_to_json(&values),
            Value::Array(vec![
                Value::Bool(true),
                Value::String("7".to_owned()),
                Value::Array(vec![
                    Value::String("hello".to_owned()),
                    Value::String("0x000000000000000000000000000000000000dead".to_owned()),
                ]),
            ])
        );
    }

    #[test]
    fn wait_options_from_flag() {
        let wait = WaitOptions::from_flag(true, 90);
        assert!(wait.wait);
        assert_eq!(wait.timeout_secs, 90);
    }

    #[test]
    fn resolve_address_target_defaults_to_index_zero() {
        let paths = Paths::discover().expect("paths");
        let target = resolve_address_target(&paths, None, None).expect("target");
        assert_eq!(target.index, 0);
        assert!(target.alias.is_none());
    }

    #[test]
    fn write_guard_uses_wallet_lock_path() {
        let paths = Paths::discover().expect("paths");
        assert!(paths.lock_file.ends_with("wallet.lock"));
    }

    #[test]
    fn resolve_contract_function_accepts_full_signature() {
        let abi: JsonAbi = serde_json::from_str(
            r#"[{"type":"function","name":"foo","inputs":[{"name":"value","type":"uint256"}],"outputs":[]},{"type":"function","name":"foo","inputs":[{"name":"recipient","type":"address"}],"outputs":[]}]"#,
        )
        .expect("abi");

        let function =
            resolve_contract_function(&abi, "foo(address)").expect("resolve overloaded function");
        assert_eq!(function.signature(), "foo(address)");
    }

    #[test]
    fn resolve_contract_function_rejects_ambiguous_name() {
        let abi: JsonAbi = serde_json::from_str(
            r#"[{"type":"function","name":"foo","inputs":[{"name":"value","type":"uint256"}],"outputs":[]},{"type":"function","name":"foo","inputs":[{"name":"recipient","type":"address"}],"outputs":[]}]"#,
        )
        .expect("abi");

        let error = resolve_contract_function(&abi, "foo").expect_err("ambiguous function");
        let message = error.to_string();
        assert!(message.contains("function `foo` is overloaded"));
        assert!(message.contains("foo(uint256)"));
        assert!(message.contains("foo(address)"));
    }

    #[test]
    fn passphrase_protected_wallet_requires_runtime_passphrase() {
        let temp = TempDir::new().expect("temp home");
        let paths = temp_paths(&temp, "dex");

        let summary = import(
            &paths,
            "test test test test test test test test test test test junk",
            Some("hunter2"),
        )
        .expect("import wallet");
        assert!(summary.address.starts_with("0x"));
        assert!(passphrase_required(&paths).expect("passphrase flag"));

        let error = derive_address(&paths, 0, None).expect_err("missing runtime passphrase");
        assert!(error.to_string().contains("requires a BIP-39 passphrase"));

        let address = derive_address(&paths, 0, Some("hunter2")).expect("derive with passphrase");
        assert!(address.starts_with("0x"));
    }

    #[test]
    fn write_guard_serializes_project_writes() {
        let temp = TempDir::new().expect("temp home");
        let paths = temp_paths(&temp, "dex");

        let (acquired_tx, acquired_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first_paths = paths.clone();
        let first = thread::spawn(move || {
            let _guard = WriteGuard::acquire(&first_paths).expect("first lock");
            acquired_tx.send(()).expect("notify acquired");
            release_rx.recv().expect("wait release");
        });

        acquired_rx.recv().expect("first acquired");

        let second_paths = paths.clone();
        let second = thread::spawn(move || {
            let started = Instant::now();
            let _guard = WriteGuard::acquire(&second_paths).expect("second lock");
            started.elapsed()
        });

        thread::sleep(Duration::from_millis(150));
        release_tx.send(()).expect("release first");

        let elapsed = second.join().expect("second join");
        first.join().expect("first join");
        assert!(
            elapsed >= Duration::from_millis(100),
            "second writer acquired lock too early: {elapsed:?}"
        );
    }
}
