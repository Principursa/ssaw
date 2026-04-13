use std::{
    net::TcpListener,
    process::{Child, Command, Stdio},
    time::Duration,
};

use alloy::{
    network::TransactionBuilder,
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
    signers::local::PrivateKeySigner,
};
use alloy_signer_local::{MnemonicBuilder, coins_bip39::English};
use tempfile::TempDir;

const TEST_MNEMONIC: &str = "test test test test test test test test test test test junk";
const COUNTER_ABI: &str = r#"[{"inputs":[],"name":"counter","outputs":[{"internalType":"uint256","name":"","type":"uint256"}],"stateMutability":"view","type":"function"},{"inputs":[],"name":"increment","outputs":[],"stateMutability":"nonpayable","type":"function"}]"#;
const COUNTER_BYTECODE: &str = "6080806040523460135760b2908160188239f35b5f80fdfe60808060405260043610156011575f80fd5b5f3560e01c90816361bc221a146065575063d09de08a14602f575f80fd5b346061575f3660031901126061575f5460018101809111604d575f55005b634e487b7160e01b5f52601160045260245ffd5b5f80fd5b346061575f3660031901126061576020905f548152f3fea2646970667358221220d802267a5f574e54a87a63d0ff8d733fdb275e6e6c502831d9e14f957bbcd7a264736f6c634300081a0033";

fn ssaw_cmd(home: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ssaw"));
    cmd.env("HOME", home);
    cmd
}

fn run_server_requests(home: &std::path::Path, requests: &[&str]) -> std::process::Output {
    ssaw_cmd(home)
        .args(["serve"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map(|mut child| {
            use std::io::Write;

            let mut stdin = child.stdin.take().expect("stdin");
            stdin
                .write_all(
                    br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"0.1.0"}}}"#,
                )
                .expect("write initialize");
            stdin.write_all(b"\n").expect("write newline");
            stdin
                .write_all(br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .expect("write initialized notification");
            stdin.write_all(b"\n").expect("write newline");

            for request in requests {
                stdin
                    .write_all(request.as_bytes())
                    .expect("write request");
                stdin.write_all(b"\n").expect("write newline");
            }

            drop(stdin);
            child.wait_with_output().expect("wait output")
        })
        .expect("run server")
}

#[tokio::test]
async fn send_and_contract_wait_flow() {
    let home = TempDir::new().expect("temp home");
    let port = free_port();
    let mut anvil = spawn_anvil(port);
    let endpoint = format!("http://127.0.0.1:{port}");
    wait_for_anvil(port);

    let import = ssaw_cmd(home.path())
        .args(["project", "import", "local"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn import");
    feed_stdin_and_wait(import, TEST_MNEMONIC);

    let add_chain = ssaw_cmd(home.path())
        .args(["add-chain", "local", "31337", "--rpc-url-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn add-chain");
    feed_stdin_and_wait(add_chain, &endpoint);

    let send = ssaw_cmd(home.path())
        .args([
            "send-transaction",
            "--chain",
            "local",
            "--to",
            "0x000000000000000000000000000000000000dead",
            "--value-wei",
            "1",
            "--wait",
            "--timeout-secs",
            "30",
        ])
        .output()
        .expect("send transaction");
    assert!(
        send.status.success(),
        "{}",
        String::from_utf8_lossy(&send.stderr)
    );
    let send_stdout = String::from_utf8_lossy(&send.stdout);
    assert!(send_stdout.contains("\"confirmed\":true"));

    let signer: PrivateKeySigner = MnemonicBuilder::<English>::default()
        .phrase(TEST_MNEMONIC)
        .derivation_path("m/44'/60'/0'/0/0")
        .expect("derivation path")
        .build()
        .expect("build signer");
    let provider = ProviderBuilder::new()
        .wallet(signer)
        .connect_http(endpoint.parse().expect("endpoint url"));

    let deploy_tx = TransactionRequest::default()
        .with_deploy_code(hex::decode(COUNTER_BYTECODE).expect("bytecode"));
    let receipt = provider
        .send_transaction(deploy_tx)
        .await
        .expect("deploy")
        .get_receipt()
        .await
        .expect("deploy receipt");
    let contract = receipt.contract_address.expect("contract address");

    let read_before = ssaw_cmd(home.path())
        .args([
            "read-contract",
            "--chain",
            "local",
            "--address",
            &format!("{:#x}", contract),
            "--function",
            "counter",
            "--abi-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn read before");
    let read_before = feed_stdin_and_wait(read_before, COUNTER_ABI);
    assert!(String::from_utf8_lossy(&read_before.stdout).contains("[\"0\"]"));

    let write = ssaw_cmd(home.path())
        .args([
            "write-contract",
            "--chain",
            "local",
            "--address",
            &format!("{:#x}", contract),
            "--function",
            "increment",
            "--abi-stdin",
            "--wait",
            "--timeout-secs",
            "30",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn write");
    let write = feed_stdin_and_wait(write, COUNTER_ABI);
    assert!(
        write.status.success(),
        "{}",
        String::from_utf8_lossy(&write.stderr)
    );
    assert!(String::from_utf8_lossy(&write.stdout).contains("\"confirmed\":true"));

    let read_after = ssaw_cmd(home.path())
        .args([
            "read-contract",
            "--chain",
            "local",
            "--address",
            &format!("{:#x}", contract),
            "--function",
            "counter",
            "--abi-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn read after");
    let read_after = feed_stdin_and_wait(read_after, COUNTER_ABI);
    assert!(
        read_after.status.success(),
        "{}",
        String::from_utf8_lossy(&read_after.stderr)
    );
    assert!(String::from_utf8_lossy(&read_after.stdout).contains("[\"1\"]"));

    let _ = anvil.kill();
    let _ = anvil.wait();
}

#[tokio::test]
async fn server_transaction_tools_include_signer_alias_metadata() {
    let home = TempDir::new().expect("temp home");
    let port = free_port();
    let mut anvil = spawn_anvil(port);
    let endpoint = format!("http://127.0.0.1:{port}");
    wait_for_anvil(port);

    let import = ssaw_cmd(home.path())
        .args(["project", "import", "local"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn import");
    feed_stdin_and_wait(import, TEST_MNEMONIC);

    let alias = ssaw_cmd(home.path())
        .args([
            "alias", "set", "deployer", "--index", "0", "--label", "deployer",
        ])
        .output()
        .expect("set alias");
    assert!(
        alias.status.success(),
        "{}",
        String::from_utf8_lossy(&alias.stderr)
    );

    let add_chain = ssaw_cmd(home.path())
        .args(["add-chain", "local", "31337", "--rpc-url-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn add-chain");
    feed_stdin_and_wait(add_chain, &endpoint);

    let output = run_server_requests(
        home.path(),
        &[
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"send_transaction","arguments":{"project":"local","chain":"local","to":"0x000000000000000000000000000000000000dead","value_wei":"1","alias":"deployer","wait":true,"timeout_secs":30}}}"#,
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let responses: Vec<serde_json::Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("parse response line"))
        .collect();
    assert_eq!(responses.len(), 2);

    let structured = &responses[1]["result"]["structuredContent"];
    assert_eq!(structured["project"], "local");
    assert_eq!(structured["alias"], "deployer");
    assert_eq!(structured["index"], serde_json::json!(0));
    assert_eq!(structured["aliases"], serde_json::json!(["deployer"]));
    assert!(
        structured["address"]
            .as_str()
            .expect("signer address")
            .starts_with("0x")
    );
    assert_eq!(structured["confirmed"], serde_json::json!(true));

    let _ = anvil.kill();
    let _ = anvil.wait();
}

fn feed_stdin_and_wait(mut child: std::process::Child, input: &str) -> std::process::Output {
    use std::io::Write;

    let mut stdin = child.stdin.take().expect("stdin");
    stdin.write_all(input.as_bytes()).expect("write stdin");
    drop(stdin);
    child.wait_with_output().expect("wait output")
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind port")
        .local_addr()
        .expect("local addr")
        .port()
}

fn spawn_anvil(port: u16) -> Child {
    Command::new("anvil")
        .args([
            "--port",
            &port.to_string(),
            "--chain-id",
            "31337",
            "--mnemonic",
            TEST_MNEMONIC,
            "--silent",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn anvil")
}

fn wait_for_anvil(port: u16) {
    for _ in 0..50 {
        if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }

        std::thread::sleep(Duration::from_millis(100));
    }

    panic!("anvil did not become ready on port {port}");
}
