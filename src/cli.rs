use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};

use crate::chain;
use crate::config::Paths;
use crate::server;
use crate::wallet;

#[derive(Debug, Parser)]
#[command(name = "ssaw", version, about = "Shark's Secure Agent Wallet")]
pub struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init,
    Import(ImportArgs),
    Address(AddressArgs),
    AddChain(AddChainArgs),
    ListChains,
    SignMessage(SignMessageArgs),
    SignTypedData(SignTypedDataArgs),
    SendTransaction(SendTransactionArgs),
    Doctor,
    Serve,
}

#[derive(Debug, Args)]
struct ImportArgs {
    #[arg(long, default_value_t = false)]
    passphrase_stdin: bool,
}

#[derive(Debug, Args)]
struct AddressArgs {
    #[arg(long, default_value_t = 0)]
    index: u32,
}

#[derive(Debug, Args)]
struct AddChainArgs {
    name: String,
    chain_id: u64,
    #[arg(long)]
    rpc_url_stdin: bool,
}

#[derive(Debug, Args)]
struct SignMessageArgs {
    message: String,
    #[arg(long, default_value_t = 0)]
    index: u32,
}

#[derive(Debug, Args)]
struct SignTypedDataArgs {
    #[arg(long, default_value_t = 0)]
    index: u32,
}

#[derive(Debug, Args)]
struct SendTransactionArgs {
    #[arg(long)]
    chain: String,
    #[arg(long)]
    to: String,
    #[arg(long)]
    value_wei: String,
    #[arg(long)]
    data: Option<String>,
    #[arg(long, default_value_t = 0)]
    index: u32,
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::discover()?;

    match cli.command {
        Command::Init => cmd_init(&paths),
        Command::Import(args) => cmd_import(&paths, args),
        Command::Address(args) => cmd_address(&paths, args),
        Command::AddChain(args) => cmd_add_chain(&paths, args),
        Command::ListChains => cmd_list_chains(&paths),
        Command::SignMessage(args) => cmd_sign_message(&paths, args),
        Command::SignTypedData(args) => cmd_sign_typed_data(&paths, args),
        Command::SendTransaction(args) => cmd_send_transaction(&paths, args).await,
        Command::Doctor => cmd_doctor(&paths),
        Command::Serve => cmd_serve(&paths).await,
    }
}

fn cmd_init(paths: &Paths) -> Result<()> {
    let (identity_path, recipient) = wallet::ensure_identity(paths)?;
    let (mnemonic, summary) = wallet::init(paths)?;

    println!("SSAW identity: {}", identity_path.display());
    println!("SSAW recipient: {recipient}");
    println!();
    println!("Mnemonic:");
    println!("{mnemonic}");
    println!();
    println!("Address[0]: {}", summary.address);
    Ok(())
}

fn cmd_import(paths: &Paths, args: ImportArgs) -> Result<()> {
    let phrase = wallet::read_phrase_from_stdin()?;
    let passphrase = if args.passphrase_stdin {
        Some(wallet::read_secret_line("BIP-39 passphrase: ")?)
    } else {
        None
    };
    let summary = wallet::import(paths, phrase, passphrase)?;
    println!("Address[0]: {}", summary.address);
    Ok(())
}

fn cmd_address(paths: &Paths, args: AddressArgs) -> Result<()> {
    let address = wallet::derive_address(paths, args.index)?;
    println!("{address}");
    Ok(())
}

fn cmd_add_chain(paths: &Paths, args: AddChainArgs) -> Result<()> {
    if !args.rpc_url_stdin {
        bail!("`ssaw add-chain` requires --rpc-url-stdin");
    }

    let rpc_url = wallet::read_phrase_from_stdin().context("failed to read rpc url from stdin")?;
    chain::add_chain(paths, &args.name, args.chain_id, rpc_url)?;
    println!("Added chain `{}`", args.name);
    Ok(())
}

fn cmd_list_chains(paths: &Paths) -> Result<()> {
    let config = chain::load(paths)?;
    if config.chains.is_empty() {
        println!("No chains configured.");
        return Ok(());
    }

    for (name, entry) in config.chains {
        println!("{name}\t{}\t{}", entry.chain_id, entry.rpc_url);
    }
    Ok(())
}

fn cmd_sign_message(paths: &Paths, args: SignMessageArgs) -> Result<()> {
    let output = wallet::sign_message(paths, &args.message, args.index)?;
    println!("{}", output.signature);
    Ok(())
}

fn cmd_sign_typed_data(paths: &Paths, args: SignTypedDataArgs) -> Result<()> {
    let typed_data_json =
        wallet::read_phrase_from_stdin().context("failed to read typed data JSON from stdin")?;
    let output = wallet::sign_typed_data(paths, &typed_data_json, args.index)?;
    println!("{}", output.signature);
    Ok(())
}

async fn cmd_send_transaction(paths: &Paths, args: SendTransactionArgs) -> Result<()> {
    let selector = chain::ChainSelector::parse(&args.chain);
    let sent = wallet::send_transaction(
        paths,
        &selector,
        &args.to,
        &args.value_wei,
        args.data.as_deref(),
        args.index,
    )
    .await?;
    println!("{}", sent.tx_hash);
    Ok(())
}

fn cmd_doctor(paths: &Paths) -> Result<()> {
    let identity = paths.identity_file()?;
    println!("state_dir\t{}", paths.state_dir.display());
    println!("config_dir\t{}", paths.config_dir.display());
    println!(
        "seed_file\t{}\t{}",
        paths.seed_file.display(),
        exists_marker(&paths.seed_file)
    );
    println!(
        "chains_file\t{}\t{}",
        paths.chains_file.display(),
        exists_marker(&paths.chains_file)
    );
    println!(
        "identity_file\t{}\t{}",
        identity.display(),
        exists_marker(&identity)
    );
    Ok(())
}

async fn cmd_serve(paths: &Paths) -> Result<()> {
    server::run(paths).await
}

fn exists_marker(path: &std::path::Path) -> &'static str {
    if path.exists() {
        "exists"
    } else {
        "missing"
    }
}
