use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
fn fixture() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!(
            "agent-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir(&path).unwrap();
    fs::write(path.join("source.rs"), "fn main() {}\n").unwrap();
    fs::write(path.join("large.txt"), "x".repeat(20_000)).unwrap();
    fs::write(path.join("binary"), [255, 254]).unwrap();
    path
}
fn invoke(root: &Path, input: &str, extra: &[&str]) -> std::process::Output {
    let config = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../settings.toml");
    let mut child = Command::new(env!("CARGO_BIN_EXE_oscar"))
        .args([
            "agent",
            "--config",
            config.to_str().unwrap(),
            "--workspace",
            root.to_str().unwrap(),
        ])
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}
#[test]
fn interactive_session_reads_files_proposes_edits_and_resets_without_persistence() {
    let root = fixture();
    let output = invoke(
        &root,
        "/help\n/read source.rs\nPropose a test\n/status\n/reset\n/status\n/exit\n",
        &[],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("File loaded")
            && stdout.contains("Mock proposal")
            && stdout.contains("Turns: 1/32")
    );
    assert!(stdout.contains("Context cleared") && stdout.contains("local-only: true"));
    assert_eq!(
        fs::read_to_string(root.join("source.rs")).unwrap(),
        "fn main() {}\n"
    );
    assert!(!root.join("run.json").exists());
}
#[test]
fn traversal_absolute_paths_directories_and_oversized_or_binary_files_are_rejected() {
    let root = fixture();
    let output = invoke(
        &root,
        "/read ../Cargo.toml\n/read C:/Windows/win.ini\n/read large.txt\n/read binary\n/read missing\n/exit\n",
        &[],
    );
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("File loaded"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("denied")
            && stderr.contains("output exceeds")
            && stderr.contains("execution failed")
    );
}
#[test]
fn initial_request_and_eof_complete_the_session() {
    let output = invoke(&fixture(), "", &["Propose tests"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Mock proposal"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("completed"));
}
#[cfg(unix)]
#[test]
fn symlink_escape_is_rejected() {
    let root = fixture();
    let outside = fixture();
    std::os::unix::fs::symlink(&outside, root.join("escape")).unwrap();
    let output = invoke(&root, "/read escape/source.rs\n/exit\n", &[]);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("File loaded"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("denied"));
}

#[cfg(windows)]
#[test]
fn junction_escape_is_rejected() {
    let root = fixture().canonicalize().unwrap();
    let outside = fixture().canonicalize().unwrap();
    // Junction creation does not require the symbolic-link privilege on Windows.
    let link = root.join("escape");
    let creation = Command::new("cmd")
        .args([
            "/C",
            "mklink",
            "/J",
            link.to_str().unwrap(),
            outside.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        creation.status.success(),
        "{}",
        String::from_utf8_lossy(&creation.stderr)
    );
    let output = invoke(&root, "/read escape/source.rs\n/exit\n", &[]);
    assert!(!String::from_utf8_lossy(&output.stdout).contains("File loaded"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("denied"));
}
