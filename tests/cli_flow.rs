use std::process::Command;

use serde_json::Value;
use tempfile::TempDir;

fn ssaw_cmd(home: &std::path::Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_ssaw"));
    cmd.env("HOME", home);
    cmd
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

    let output = ssaw_cmd(home.path())
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
            stdin
                .write_all(br#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#)
                .expect("write tools/list");
            stdin.write_all(b"\n").expect("write newline");
            stdin
                .write_all(
                    br#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"get_address","arguments":{"project":"dex","alias":"deployer"}}}"#,
                )
                .expect("write tools/call");
            stdin.write_all(b"\n").expect("write newline");
            drop(stdin);
            child.wait_with_output().expect("wait output")
        })
        .expect("run server");
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
    assert_eq!(structured["alias"], "deployer");
    assert_eq!(structured["aliases"], serde_json::json!(["deployer"]));
    assert!(
        structured["address"]
            .as_str()
            .expect("address")
            .starts_with("0x")
    );
}
