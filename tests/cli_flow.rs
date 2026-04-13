use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

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

#[test]
fn project_and_alias_flow() {
    let home = TempDir::new().expect("temp home");

    let init = ssaw_cmd(home.path())
        .args(["project", "init", "dex"])
        .output()
        .expect("project init");
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    assert!(String::from_utf8_lossy(&init.stdout).contains("Selected project `dex`"));

    let set_alias = ssaw_cmd(home.path())
        .args([
            "alias", "set", "deployer", "--index", "0", "--label", "deployer", "--label", "admin",
        ])
        .output()
        .expect("set alias");
    assert!(
        set_alias.status.success(),
        "{}",
        String::from_utf8_lossy(&set_alias.stderr)
    );

    let address = ssaw_cmd(home.path())
        .args(["address", "--alias", "deployer"])
        .output()
        .expect("address");
    assert!(
        address.status.success(),
        "{}",
        String::from_utf8_lossy(&address.stderr)
    );
    let address_stdout = String::from_utf8_lossy(&address.stdout);
    assert!(address_stdout.trim_start().starts_with("0x"));

    let alias_list = ssaw_cmd(home.path())
        .args(["alias", "list"])
        .output()
        .expect("alias list");
    assert!(
        alias_list.status.success(),
        "{}",
        String::from_utf8_lossy(&alias_list.stderr)
    );
    let alias_list_stdout = String::from_utf8_lossy(&alias_list.stdout);
    assert!(alias_list_stdout.contains("deployer"));
    assert!(alias_list_stdout.contains("admin"));
}

#[test]
fn list_chains_cli_does_not_echo_rpc_url() {
    let home = TempDir::new().expect("temp home");

    let init = ssaw_cmd(home.path())
        .args(["project", "init", "dex"])
        .output()
        .expect("project init");
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );

    let add_chain = ssaw_cmd(home.path())
        .args(["add-chain", "local", "31337", "--rpc-url-stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map(|mut child| {
            use std::io::Write;

            let mut stdin = child.stdin.take().expect("stdin");
            stdin
                .write_all(b"http://127.0.0.1:8545")
                .expect("write rpc url");
            drop(stdin);
            child.wait_with_output().expect("wait output")
        })
        .expect("spawn add-chain");
    assert!(
        add_chain.status.success(),
        "{}",
        String::from_utf8_lossy(&add_chain.stderr)
    );

    let list = ssaw_cmd(home.path())
        .args(["list-chains"])
        .output()
        .expect("list chains");
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );

    let stdout = String::from_utf8_lossy(&list.stdout);
    assert!(stdout.contains("local\t31337"));
    assert!(!stdout.contains("http://127.0.0.1:8545"));
}

#[test]
fn serve_supports_mcp_tools_and_alias_metadata() {
    let home = TempDir::new().expect("temp home");

    let init = ssaw_cmd(home.path())
        .args(["project", "init", "dex"])
        .output()
        .expect("project init");
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );

    let set_alias = ssaw_cmd(home.path())
        .args([
            "alias", "set", "deployer", "--index", "0", "--label", "admin",
        ])
        .output()
        .expect("set alias");
    assert!(
        set_alias.status.success(),
        "{}",
        String::from_utf8_lossy(&set_alias.stderr)
    );

    let output = run_server_requests(
        home.path(),
        &[
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_address","arguments":{"project":"dex","alias":"deployer"}}}"#,
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let responses: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("parse response line"))
        .collect();
    assert_eq!(responses.len(), 3);
    assert_eq!(responses[0]["id"], 1);
    assert_eq!(
        responses[0]["result"]["capabilities"]["tools"]["listChanged"],
        false
    );
    assert_eq!(responses[1]["id"], 2);
    assert!(
        responses[1]["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .any(|tool| tool["name"] == "get_address")
    );
    assert_eq!(responses[2]["id"], 3);
    let structured = &responses[2]["result"]["structuredContent"];
    assert_eq!(structured["project"], "dex");
    assert_eq!(structured["alias"], "deployer");
    assert_eq!(structured["aliases"], serde_json::json!(["deployer"]));
    assert!(
        structured["address"]
            .as_str()
            .expect("address")
            .starts_with("0x")
    );
}

#[test]
fn serve_supports_chain_management_and_doctor_tools() {
    let home = TempDir::new().expect("temp home");

    let init = ssaw_cmd(home.path())
        .args(["project", "init", "dex"])
        .output()
        .expect("project init");
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );

    let output = run_server_requests(
        home.path(),
        &[
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_chains","arguments":{"project":"dex"}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"add_chain","arguments":{"project":"dex","name":"local","chain_id":31337,"rpc_url":"http://127.0.0.1:8545"}}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"list_chains","arguments":{"project":"dex"}}}"#,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"doctor","arguments":{"project":"dex"}}}"#,
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let responses: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("parse response line"))
        .collect();
    assert_eq!(responses.len(), 6);
    assert!(
        responses[1]["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .any(|tool| tool["name"] == "add_chain")
    );
    assert!(
        responses[1]["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .any(|tool| tool["name"] == "doctor")
    );
    assert_eq!(
        responses[2]["result"]["structuredContent"]["chains"],
        serde_json::json!([])
    );
    assert_eq!(
        responses[3]["result"]["structuredContent"]["project"],
        serde_json::json!("dex")
    );
    assert_eq!(
        responses[3]["result"]["structuredContent"]["name"],
        serde_json::json!("local")
    );
    assert!(
        responses[3]["result"]["structuredContent"]
            .get("rpc_url")
            .is_none()
    );
    assert_eq!(
        responses[4]["result"]["structuredContent"]["chains"][0]["name"],
        serde_json::json!("local")
    );
    assert!(
        responses[4]["result"]["structuredContent"]["chains"][0]
            .get("rpc_url")
            .is_none()
    );
    assert_eq!(
        responses[5]["result"]["structuredContent"]["project"],
        serde_json::json!("dex")
    );
    assert_eq!(
        responses[5]["result"]["structuredContent"]["seed_exists"],
        serde_json::json!(true)
    );
    assert_eq!(
        responses[5]["result"]["structuredContent"]["chains"][0]["chain_id"],
        serde_json::json!(31337)
    );
    assert!(
        responses[5]["result"]["structuredContent"]["chains"][0]
            .get("rpc_url")
            .is_none()
    );
}

#[test]
fn serve_unknown_chain_error_includes_project_context_and_hint() {
    let home = TempDir::new().expect("temp home");

    let init = ssaw_cmd(home.path())
        .args(["project", "init", "dex"])
        .output()
        .expect("project init");
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );

    let output = run_server_requests(
        home.path(),
        &[
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"send_transaction","arguments":{"project":"dex","chain":"local","to":"0x000000000000000000000000000000000000dead","value_wei":"1"}}}"#,
        ],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let responses: Vec<Value> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| serde_json::from_str(line).expect("parse response line"))
        .collect();
    assert_eq!(responses.len(), 2);
    let error = responses[1]["result"]["structuredContent"]["error"]
        .as_str()
        .expect("error");
    assert!(error.contains("unknown chain `local` in project `dex`"));
    assert!(error.contains("configured chains: []"));
    assert!(error.contains("project-local"));
}
