use std::path::{Path, PathBuf};
use std::time::Duration;

use alloy::signers::local::PrivateKeySigner;
use anyhow::{Context, Result, bail};
use rand::Rng;
use serde::Serialize;
use tokio::process::Command;

use crate::chain::{self, ChainSelector};
use crate::config::Paths;
use crate::wallet;

const MAX_OUTPUT_BYTES: usize = 64 * 1024;
const DEFAULT_TIMEOUT_SECS: u64 = 300;

#[derive(Debug, Clone, Serialize)]
pub struct ForgeOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

pub struct ForgeOptions<'a> {
    pub args: &'a [String],
    pub chain: Option<&'a str>,
    pub index: u32,
    pub timeout_secs: u64,
    pub runtime_passphrase: Option<&'a str>,
}

struct TransientKeystore {
    _dir: tempfile::TempDir,
    keystore_path: PathBuf,
    password_file_path: PathBuf,
}

impl TransientKeystore {
    fn create(signer: &PrivateKeySigner) -> Result<Self> {
        let dir = tempfile::TempDir::new().context("failed to create temp directory for transient keystore")?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
                .context("failed to set temp directory permissions")?;
        }

        let password: String = rand::thread_rng()
            .sample_iter(&rand::distributions::Alphanumeric)
            .take(32)
            .map(char::from)
            .collect();

        let keystore_path = dir.path().join("key.json");
        let password_file_path = dir.path().join("password");

        alloy_signer_local::PrivateKeySigner::encrypt_keystore(
            dir.path(),
            &mut rand::thread_rng(),
            signer.credential().to_bytes(),
            &password,
            Some("key"),
        )
        .context("failed to encrypt keystore")?;

        std::fs::write(&password_file_path, password.as_bytes())
            .context("failed to write keystore password file")?;

        Ok(Self {
            _dir: dir,
            keystore_path,
            password_file_path,
        })
    }
}

pub async fn run(paths: &Paths, options: ForgeOptions<'_>) -> Result<ForgeOutput> {
    which_forge()?;

    let _guard = crate::wallet::WriteGuard::acquire(paths)?;
    let signer = wallet::signer_for_index(paths, options.index, options.runtime_passphrase)?;
    let sender = format!("{:#x}", signer.address());
    let keystore = TransientKeystore::create(&signer)?;

    let rpc_url = match options.chain {
        Some(chain_value) => {
            let selector = ChainSelector::parse(chain_value);
            let chain = chain::resolve(paths, &selector)?;
            Some(chain.rpc_url)
        }
        None => None,
    };

    let args = build_forge_args(
        options.args,
        &keystore.keystore_path,
        &keystore.password_file_path,
        &sender,
        rpc_url.as_deref(),
    );

    let timeout = Duration::from_secs(if options.timeout_secs == 0 {
        DEFAULT_TIMEOUT_SECS
    } else {
        options.timeout_secs
    });

    let child = Command::new("forge")
        .args(&args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("failed to spawn forge process")?;

    let child_id = child.id();
    let result = tokio::time::timeout(timeout, child.wait_with_output()).await;

    match result {
        Ok(Ok(output)) => Ok(ForgeOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: truncate_output(&output.stdout),
            stderr: truncate_output(&output.stderr),
        }),
        Ok(Err(error)) => bail!("forge process failed: {error}"),
        Err(_) => {
            if let Some(pid) = child_id {
                let _ = std::process::Command::new("kill")
                    .arg(pid.to_string())
                    .status();
            }
            bail!(
                "forge process timed out after {} seconds",
                timeout.as_secs()
            );
        }
    }
}

fn which_forge() -> Result<()> {
    match std::process::Command::new("forge")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(status) if status.success() => Ok(()),
        _ => bail!(
            "forge not found; install Foundry with `curl -L https://foundry.paradigm.xyz | bash && foundryup`"
        ),
    }
}

fn build_forge_args(
    user_args: &[String],
    keystore_path: &Path,
    password_file_path: &Path,
    sender: &str,
    rpc_url: Option<&str>,
) -> Vec<String> {
    let mut args: Vec<String> = user_args.to_vec();

    args.push("--keystore".to_owned());
    args.push(keystore_path.display().to_string());
    args.push("--password-file".to_owned());
    args.push(password_file_path.display().to_string());

    if !user_args.iter().any(|arg| arg == "--sender") {
        args.push("--sender".to_owned());
        args.push(sender.to_owned());
    }

    if let Some(rpc_url) = rpc_url {
        if !user_args.iter().any(|arg| arg == "--rpc-url") {
            args.push("--rpc-url".to_owned());
            args.push(rpc_url.to_owned());
        }
    }

    args
}

fn truncate_output(bytes: &[u8]) -> String {
    if bytes.len() <= MAX_OUTPUT_BYTES {
        return String::from_utf8_lossy(bytes).into_owned();
    }

    let truncated = &bytes[..MAX_OUTPUT_BYTES];
    let mut output = String::from_utf8_lossy(truncated).into_owned();
    output.push_str("\n[truncated]");
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_forge_args_injects_keystore_and_sender() {
        let user_args: Vec<String> = vec![
            "script".to_owned(),
            "Deploy.s.sol".to_owned(),
            "--broadcast".to_owned(),
        ];
        let args = build_forge_args(
            &user_args,
            Path::new("/tmp/ssaw-xxxx/key.json"),
            Path::new("/tmp/ssaw-xxxx/password"),
            "0xdead",
            None,
        );
        assert_eq!(
            args,
            vec![
                "script",
                "Deploy.s.sol",
                "--broadcast",
                "--keystore",
                "/tmp/ssaw-xxxx/key.json",
                "--password-file",
                "/tmp/ssaw-xxxx/password",
                "--sender",
                "0xdead",
            ]
        );
    }

    #[test]
    fn build_forge_args_injects_rpc_url_when_provided() {
        let user_args: Vec<String> = vec!["script".to_owned(), "Deploy.s.sol".to_owned()];
        let args = build_forge_args(
            &user_args,
            Path::new("/tmp/key.json"),
            Path::new("/tmp/password"),
            "0xdead",
            Some("http://127.0.0.1:8545"),
        );
        assert!(args.contains(&"--rpc-url".to_owned()));
        assert!(args.contains(&"http://127.0.0.1:8545".to_owned()));
    }

    #[test]
    fn build_forge_args_skips_sender_when_user_provides_it() {
        let user_args: Vec<String> = vec![
            "script".to_owned(),
            "--sender".to_owned(),
            "0xcafe".to_owned(),
        ];
        let args = build_forge_args(
            &user_args,
            Path::new("/tmp/key.json"),
            Path::new("/tmp/password"),
            "0xdead",
            None,
        );
        let sender_count = args.iter().filter(|a| *a == "--sender").count();
        assert_eq!(sender_count, 1);
        assert!(args.contains(&"0xcafe".to_owned()));
        assert!(!args.contains(&"0xdead".to_owned()));
    }

    #[test]
    fn build_forge_args_skips_rpc_url_when_user_provides_it() {
        let user_args: Vec<String> = vec![
            "script".to_owned(),
            "--rpc-url".to_owned(),
            "http://custom:8545".to_owned(),
        ];
        let args = build_forge_args(
            &user_args,
            Path::new("/tmp/key.json"),
            Path::new("/tmp/password"),
            "0xdead",
            Some("http://injected:8545"),
        );
        let rpc_count = args.iter().filter(|a| *a == "--rpc-url").count();
        assert_eq!(rpc_count, 1);
        assert!(args.contains(&"http://custom:8545".to_owned()));
        assert!(!args.contains(&"http://injected:8545".to_owned()));
    }

    #[test]
    fn truncate_output_leaves_short_output_intact() {
        let short = b"hello world";
        assert_eq!(truncate_output(short), "hello world");
    }

    #[test]
    fn truncate_output_truncates_large_output() {
        let large = vec![b'x'; MAX_OUTPUT_BYTES + 1000];
        let result = truncate_output(&large);
        assert!(result.ends_with("[truncated]"));
        assert!(result.len() < large.len());
    }
}
