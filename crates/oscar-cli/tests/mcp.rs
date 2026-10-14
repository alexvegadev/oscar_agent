use std::process::Command;

#[test]
fn list_is_offline_redacted_and_describes_disabled_examples() {
    let result = Command::new(env!("CARGO_BIN_EXE_oscar"))
        .args(["mcp", "list", "--config", "examples/mcp-config.toml"])
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.contains("files: Stdio, enabled=false"));
    assert!(output.contains("docs: Http, enabled=false"));
    assert!(!output.contains("mcp.example.com"));
    assert!(!output.contains("OSCAR_DOCS_MCP_TOKEN"));
}
#[test]
fn check_respects_feature_gating_and_never_launches_disabled_servers() {
    let result = Command::new(env!("CARGO_BIN_EXE_oscar"))
        .args(["mcp", "check", "--config", "examples/mcp-config.toml"])
        .current_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
        .output()
        .unwrap();
    if cfg!(feature = "mcp") {
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains("verified: 0 tools"));
    } else {
        assert!(!result.status.success());
        assert!(String::from_utf8_lossy(&result.stderr).contains("requires --features mcp"));
    }
}
