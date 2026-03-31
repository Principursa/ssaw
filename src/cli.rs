use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};

use crate::chain;
use crate::config::Paths;
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

pub fn run() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::discover()?;

    match cli.command {
        Command::Init => cmd_init(&paths),
        Command::Import(args) => cmd_import(&paths, args),
        Command::Address(args) => cmd_address(&paths, args),
        Command::AddChain(args) => cmd_add_chain(&paths, args),
        Command::ListChains => cmd_list_chains(&paths),
        Command::Doctor => cmd_doctor(&paths),
        Command::Serve => cmd_serve(&paths),
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

fn cmd_doctor(paths: &Paths) -> Result<()> {
    let identity = paths.identity_file()?;
    println!("state_dir\t{}", paths.state_dir.display());
    println!("config_dir\t{}", paths.config_dir.display());
    println!("seed_file\t{}\t{}", paths.seed_file.display(), exists_marker(&paths.seed_file));
    println!("chains_file\t{}\t{}", paths.chains_file.display(), exists_marker(&paths.chains_file));
    println!("identity_file\t{}\t{}", identity.display(), exists_marker(&identity));
    Ok(())
}

fn cmd_serve(paths: &Paths) -> Result<()> {
    let address = wallet::derive_address(paths, 0)
        .context("failed to load wallet before starting server")?;
    println!("serve is not implemented yet");
    println!("wallet ready: {address}");
    Ok(())
}

fn exists_marker(path: &std::path::Path) -> &'static str {
    if path.exists() {
        "exists"
    } else {
        "missing"
    }
}
