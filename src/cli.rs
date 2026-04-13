use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};

use crate::chain;
use crate::config::Paths;
use crate::server;
use crate::wallet;

#[derive(Debug, Parser)]
#[command(name = "ssaw", version, about = "Shark's Secure Agent Wallet")]
pub struct Cli {
    #[arg(long, global = true)]
    project: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Init,
    Import(ImportArgs),
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    Alias {
        #[command(subcommand)]
        command: AliasCommand,
    },
    Address(AddressArgs),
    AddChain(AddChainArgs),
    ListChains,
    SignMessage(SignMessageArgs),
    SignTypedData(SignTypedDataArgs),
    SendTransaction(SendTransactionArgs),
    ReadContract(ReadContractArgs),
    WriteContract(WriteContractArgs),
    Doctor,
    Serve(ServeArgs),
}

#[derive(Debug, Subcommand)]
enum ProjectCommand {
    Init(ProjectInitArgs),
    Import(ProjectImportArgs),
    Use(ProjectUseArgs),
    List,
    Current,
}

#[derive(Debug, Subcommand)]
enum AliasCommand {
    Set(AliasSetArgs),
    List,
    Show(AliasShowArgs),
}

#[derive(Debug, Args)]
struct ProjectInitArgs {
    name: String,
}

#[derive(Debug, Args)]
struct ProjectImportArgs {
    name: String,
    #[command(flatten)]
    passphrase: PromptPassphraseArgs,
}

#[derive(Debug, Args)]
struct ProjectUseArgs {
    name: String,
}

#[derive(Debug, Args)]
struct ImportArgs {
    #[command(flatten)]
    passphrase: PromptPassphraseArgs,
}

#[derive(Debug, Args)]
struct AddressArgs {
    #[command(flatten)]
    target: AddressTargetArgs,
    #[command(flatten)]
    passphrase: PromptPassphraseArgs,
}

#[derive(Debug, Args, Default, Clone)]
struct AddressTargetArgs {
    #[arg(long, conflicts_with = "alias")]
    index: Option<u32>,
    #[arg(long, conflicts_with = "index")]
    alias: Option<String>,
}

#[derive(Debug, Args)]
struct AliasSetArgs {
    name: String,
    #[arg(long)]
    index: u32,
    #[arg(long = "label")]
    labels: Vec<String>,
}

#[derive(Debug, Args)]
struct AliasShowArgs {
    name: String,
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
    #[command(flatten)]
    target: AddressTargetArgs,
    #[command(flatten)]
    passphrase: PromptPassphraseArgs,
}

#[derive(Debug, Args)]
struct SignTypedDataArgs {
    #[command(flatten)]
    target: AddressTargetArgs,
    #[command(flatten)]
    passphrase: PromptPassphraseArgs,
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
    #[arg(long, default_value_t = false)]
    wait: bool,
    #[arg(long, default_value_t = 60)]
    timeout_secs: u64,
    #[command(flatten)]
    target: AddressTargetArgs,
    #[command(flatten)]
    passphrase: PromptPassphraseArgs,
}

#[derive(Debug, Args)]
struct ReadContractArgs {
    #[arg(long)]
    chain: String,
    #[arg(long)]
    address: String,
    #[arg(long)]
    function: String,
    #[arg(long, default_value_t = false)]
    abi_stdin: bool,
    #[arg(long = "arg")]
    args: Vec<String>,
}

#[derive(Debug, Args)]
struct WriteContractArgs {
    #[arg(long)]
    chain: String,
    #[arg(long)]
    address: String,
    #[arg(long)]
    function: String,
    #[arg(long, default_value_t = false)]
    abi_stdin: bool,
    #[arg(long = "arg")]
    args: Vec<String>,
    #[arg(long)]
    value_wei: Option<String>,
    #[arg(long, default_value_t = false)]
    wait: bool,
    #[arg(long, default_value_t = 60)]
    timeout_secs: u64,
    #[command(flatten)]
    target: AddressTargetArgs,
    #[command(flatten)]
    passphrase: PromptPassphraseArgs,
}

#[derive(Debug, Args, Clone, Default)]
struct PromptPassphraseArgs {
    #[arg(
        short = 'p',
        long = "prompt-passphrase",
        visible_alias = "pp",
        default_value_t = false
    )]
    prompt_passphrase: bool,
}

#[derive(Debug, Args)]
struct ServeArgs {
    #[command(flatten)]
    passphrase: PromptPassphraseArgs,
}

pub async fn run() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::discover_with_project(cli.project.as_deref())?;

    match cli.command {
        Command::Init => cmd_init(&paths),
        Command::Import(args) => cmd_import(&paths, args),
        Command::Project { command } => cmd_project(&paths, command),
        Command::Alias { command } => cmd_alias(&paths, command),
        Command::Address(args) => cmd_address(&paths, args),
        Command::AddChain(args) => cmd_add_chain(&paths, args),
        Command::ListChains => cmd_list_chains(&paths),
        Command::SignMessage(args) => cmd_sign_message(&paths, args),
        Command::SignTypedData(args) => cmd_sign_typed_data(&paths, args),
        Command::SendTransaction(args) => cmd_send_transaction(&paths, args).await,
        Command::ReadContract(args) => cmd_read_contract(&paths, args).await,
        Command::WriteContract(args) => cmd_write_contract(&paths, args).await,
        Command::Doctor => cmd_doctor(&paths),
        Command::Serve(args) => cmd_serve(&paths, args).await,
    }
}

fn cmd_alias(paths: &Paths, command: AliasCommand) -> Result<()> {
    match command {
        AliasCommand::Set(args) => cmd_alias_set(paths, args),
        AliasCommand::List => cmd_alias_list(paths),
        AliasCommand::Show(args) => cmd_alias_show(paths, &args.name),
    }
}

fn cmd_project(paths: &Paths, command: ProjectCommand) -> Result<()> {
    match command {
        ProjectCommand::Init(args) => cmd_project_init(paths, &args.name),
        ProjectCommand::Import(args) => cmd_project_import(paths, args),
        ProjectCommand::Use(args) => cmd_project_use(paths, &args.name),
        ProjectCommand::List => cmd_project_list(paths),
        ProjectCommand::Current => {
            println!("{}", paths.project_name);
            Ok(())
        }
    }
}

fn cmd_project_init(paths: &Paths, project_name: &str) -> Result<()> {
    crate::config::validate_project_name(project_name)?;
    let project_paths = Paths::discover_with_project(Some(project_name))?;
    project_paths.ensure_parent_dirs()?;
    paths.write_current_project(project_name)?;
    let (identity_path, recipient) = wallet::ensure_identity(&project_paths)?;
    let (mnemonic, summary) = wallet::init(&project_paths)?;
    println!("Project: {project_name}");
    println!("SSAW identity: {}", identity_path.display());
    println!("SSAW recipient: {recipient}");
    println!();
    println!("Mnemonic:");
    println!("{mnemonic}");
    println!();
    println!("Address[0]: {}", summary.address);
    println!();
    println!("Selected project `{project_name}`");
    Ok(())
}

fn cmd_project_import(paths: &Paths, args: ProjectImportArgs) -> Result<()> {
    crate::config::validate_project_name(&args.name)?;
    let project_paths = Paths::discover_with_project(Some(&args.name))?;
    project_paths.ensure_parent_dirs()?;
    paths.write_current_project(&args.name)?;
    let phrase = wallet::read_secret_from_stdin()?;
    let passphrase = if args.passphrase.prompt_passphrase {
        Some(wallet::read_secret_line("BIP-39 passphrase: ")?)
    } else {
        None
    };
    let summary = wallet::import(
        &project_paths,
        phrase.as_str(),
        passphrase.as_ref().map(|value| value.as_str()),
    )?;
    println!("Project: {}", project_paths.project_name);
    println!("Address[0]: {}", summary.address);
    println!("Selected project `{}`", project_paths.project_name);
    Ok(())
}

fn cmd_project_use(paths: &Paths, project_name: &str) -> Result<()> {
    crate::config::validate_project_name(project_name)?;
    let project_paths = Paths::discover_with_project(Some(project_name))?;
    if project_name != "default" && !project_paths.project_dir.exists() {
        bail!("unknown project `{project_name}`");
    }

    paths.write_current_project(project_name)?;
    println!("Selected project `{project_name}`");
    Ok(())
}

fn cmd_project_list(paths: &Paths) -> Result<()> {
    for project_name in paths.list_projects()? {
        let marker = if project_name == paths.project_name {
            "*"
        } else {
            " "
        };
        println!("{marker} {project_name}");
    }
    Ok(())
}

fn cmd_alias_set(paths: &Paths, args: AliasSetArgs) -> Result<()> {
    crate::alias::set_alias(paths, &args.name, args.index, args.labels)?;
    println!("Set alias `{}` -> index {}", args.name, args.index);
    Ok(())
}

fn cmd_alias_list(paths: &Paths) -> Result<()> {
    let aliases = crate::alias::list_aliases(paths)?;
    if aliases.is_empty() {
        println!("No aliases configured.");
        return Ok(());
    }

    for alias in aliases {
        let labels = if alias.labels.is_empty() {
            "-".to_owned()
        } else {
            alias.labels.join(",")
        };
        println!("{}\t{}\t{}", alias.name, alias.index, labels);
    }
    Ok(())
}

fn cmd_alias_show(paths: &Paths, name: &str) -> Result<()> {
    let alias = crate::alias::get_alias(paths, name)?
        .with_context(|| format!("unknown alias `{name}` in project `{}`", paths.project_name))?;
    println!("name\t{name}");
    println!("index\t{}", alias.index);
    println!(
        "labels\t{}",
        if alias.labels.is_empty() {
            "-".to_owned()
        } else {
            alias.labels.join(",")
        }
    );
    Ok(())
}

fn cmd_init(paths: &Paths) -> Result<()> {
    let (identity_path, recipient) = wallet::ensure_identity(paths)?;
    let (mnemonic, summary) = wallet::init(paths)?;

    println!("Project: {}", paths.project_name);
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
    let phrase = wallet::read_secret_from_stdin()?;
    let passphrase = if args.passphrase.prompt_passphrase {
        Some(wallet::read_secret_line("BIP-39 passphrase: ")?)
    } else {
        None
    };
    let summary = wallet::import(
        paths,
        phrase.as_str(),
        passphrase.as_ref().map(|value| value.as_str()),
    )?;
    println!("Project: {}", paths.project_name);
    println!("Address[0]: {}", summary.address);
    Ok(())
}

fn cmd_address(paths: &Paths, args: AddressArgs) -> Result<()> {
    let target =
        wallet::resolve_address_target(paths, args.target.index, args.target.alias.as_deref())?;
    let passphrase = maybe_prompt_passphrase(&args.passphrase)?;
    let address = wallet::derive_address(
        paths,
        target.index,
        passphrase.as_ref().map(|value| value.as_str()),
    )?;
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
        println!("{name}\t{}", entry.chain_id);
    }
    Ok(())
}

fn cmd_sign_message(paths: &Paths, args: SignMessageArgs) -> Result<()> {
    let target =
        wallet::resolve_address_target(paths, args.target.index, args.target.alias.as_deref())?;
    let passphrase = maybe_prompt_passphrase(&args.passphrase)?;
    let output = wallet::sign_message(
        paths,
        &args.message,
        target.index,
        passphrase.as_ref().map(|value| value.as_str()),
    )?;
    println!("{}", output.signature);
    Ok(())
}

fn cmd_sign_typed_data(paths: &Paths, args: SignTypedDataArgs) -> Result<()> {
    let passphrase = maybe_prompt_passphrase(&args.passphrase)?;
    let typed_data_json =
        wallet::read_phrase_from_stdin().context("failed to read typed data JSON from stdin")?;
    let target =
        wallet::resolve_address_target(paths, args.target.index, args.target.alias.as_deref())?;
    let output = wallet::sign_typed_data(
        paths,
        &typed_data_json,
        target.index,
        passphrase.as_ref().map(|value| value.as_str()),
    )?;
    println!("{}", output.signature);
    Ok(())
}

async fn cmd_send_transaction(paths: &Paths, args: SendTransactionArgs) -> Result<()> {
    let selector = chain::ChainSelector::parse(&args.chain);
    let target =
        wallet::resolve_address_target(paths, args.target.index, args.target.alias.as_deref())?;
    let passphrase = maybe_prompt_passphrase(&args.passphrase)?;
    let sent = wallet::send_transaction(
        paths,
        &selector,
        &args.to,
        &args.value_wei,
        args.data.as_deref(),
        wallet::WaitOptions::from_flag(args.wait, args.timeout_secs),
        target.index,
        passphrase.as_ref().map(|value| value.as_str()),
    )
    .await?;
    if sent.confirmed {
        println!(
            "{}",
            serde_json::to_string(&sent).context("failed to render send result")?
        );
    } else {
        println!("{}", sent.tx_hash);
    }
    Ok(())
}

async fn cmd_read_contract(paths: &Paths, args: ReadContractArgs) -> Result<()> {
    if !args.abi_stdin {
        bail!("`ssaw read-contract` requires --abi-stdin");
    }

    let abi_json =
        wallet::read_phrase_from_stdin().context("failed to read ABI JSON from stdin")?;
    let selector = chain::ChainSelector::parse(&args.chain);
    let output = wallet::read_contract(
        paths,
        &selector,
        &args.address,
        &abi_json,
        &args.function,
        &args.args,
    )
    .await?;
    println!(
        "{}",
        serde_json::to_string(&output.outputs).context("failed to render contract output")?
    );
    Ok(())
}

async fn cmd_write_contract(paths: &Paths, args: WriteContractArgs) -> Result<()> {
    if !args.abi_stdin {
        bail!("`ssaw write-contract` requires --abi-stdin");
    }

    let passphrase = maybe_prompt_passphrase(&args.passphrase)?;
    let abi_json =
        wallet::read_phrase_from_stdin().context("failed to read ABI JSON from stdin")?;
    let selector = chain::ChainSelector::parse(&args.chain);
    let target =
        wallet::resolve_address_target(paths, args.target.index, args.target.alias.as_deref())?;
    let sent = wallet::write_contract(
        paths,
        &selector,
        &args.address,
        &abi_json,
        &args.function,
        &args.args,
        args.value_wei.as_deref(),
        wallet::WaitOptions::from_flag(args.wait, args.timeout_secs),
        target.index,
        passphrase.as_ref().map(|value| value.as_str()),
    )
    .await?;
    if sent.confirmed {
        println!(
            "{}",
            serde_json::to_string(&sent).context("failed to render contract send result")?
        );
    } else {
        println!("{}", sent.tx_hash);
    }
    Ok(())
}

fn cmd_doctor(paths: &Paths) -> Result<()> {
    let identity = paths.identity_file()?;
    println!("project\t{}", paths.project_name);
    println!("state_dir\t{}", paths.state_dir.display());
    println!("project_dir\t{}", paths.project_dir.display());
    println!(
        "current_project_file\t{}\t{}",
        paths.current_project_file.display(),
        exists_marker(&paths.current_project_file)
    );
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
        "addresses_file\t{}\t{}",
        paths.addresses_file.display(),
        exists_marker(&paths.addresses_file)
    );
    println!(
        "identity_file\t{}\t{}",
        identity.display(),
        exists_marker(&identity)
    );
    Ok(())
}

async fn cmd_serve(paths: &Paths, args: ServeArgs) -> Result<()> {
    let passphrase = maybe_prompt_passphrase(&args.passphrase)?;
    server::run(paths, passphrase).await
}

fn exists_marker(path: &std::path::Path) -> &'static str {
    if path.exists() { "exists" } else { "missing" }
}

fn maybe_prompt_passphrase(
    args: &PromptPassphraseArgs,
) -> Result<Option<zeroize::Zeroizing<String>>> {
    if args.prompt_passphrase {
        return wallet::read_secret_line("BIP-39 passphrase: ").map(Some);
    }

    Ok(None)
}
