use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
fn output_dir() -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!(
            "cli-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    assert!(!dir.exists());
    dir
}
#[test]
fn cli_runs_offline_and_refuses_to_overwrite_artifacts() {
    let dir = output_dir();
    let config = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../example_config.toml");
    let args = [
        "run",
        "Add JWT authentication and protect admin endpoints",
        "--config",
        config.to_str().unwrap(),
        "--out",
        dir.to_str().unwrap(),
    ];
    let result = Command::new(env!("CARGO_BIN_EXE_oscar"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(dir.join("PLAN.md").is_file() && dir.join(".plan/plan.json").is_file());
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("run.json")).unwrap()).unwrap();
    assert_eq!(report["outcome"], "completed");
    assert_eq!(report["calls"].as_array().unwrap().len(), 5);
    assert!(
        report["calls"]
            .as_array()
            .unwrap()
            .iter()
            .all(|c| c["simulation"] == true)
    );
    assert!(dir.join("artifacts/T03.md").exists());
    let again = Command::new(env!("CARGO_BIN_EXE_oscar"))
        .args(args)
        .output()
        .unwrap();
    assert!(!again.status.success());
}
#[test]
fn cli_plan_reloads_canonical_json_and_test_needs_no_config() {
    let first = output_dir();
    let second = output_dir();
    let config = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../example_config.toml");
    let result = Command::new(env!("CARGO_BIN_EXE_oscar"))
        .args([
            "plan",
            "Document a parser",
            "--config",
            config.to_str().unwrap(),
            "--out",
            first.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(!first.join("run.json").exists());
    let result = Command::new(env!("CARGO_BIN_EXE_oscar"))
        .args([
            "plan",
            "--plan",
            first.join(".plan/plan.json").to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "--out",
            second.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(result.status.success());
    assert_eq!(
        fs::read(first.join("PLAN.md")).unwrap(),
        fs::read(second.join("PLAN.md")).unwrap()
    );
    assert!(
        Command::new(env!("CARGO_BIN_EXE_oscar"))
            .args(["test", "--name", "smoke"])
            .current_dir(first)
            .status()
            .unwrap()
            .success()
    );
}
