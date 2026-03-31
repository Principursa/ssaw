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
fn serve_supports_project_override_and_alias_metadata() {
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
        .args([
            "serve",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map(|mut child| {
            use std::io::Write;

            let mut stdin = child.stdin.take().expect("stdin");
            stdin
                .write_all(
                    br#"{"id":1,"method":"get_address","params":{"project":"dex","alias":"deployer"}}"#,
                )
                .expect("write request");
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

    let response: Value = serde_json::from_slice(&output.stdout).expect("parse response");
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["alias"], "deployer");
    assert_eq!(
        response["result"]["aliases"],
        serde_json::json!(["deployer"])
    );
    assert!(
        response["result"]["address"]
            .as_str()
            .expect("address")
            .starts_with("0x")
    );
}
