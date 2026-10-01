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
    let progress = String::from_utf8_lossy(&result.stderr);
    assert!(progress.contains("Generating and validating heuristic plan"));
    assert!(progress.contains("Executing proposal workers (5 planned tasks)"));
    assert!(progress.contains("Saving run report and artifacts"));
    assert!(!progress.contains("JWT"));
    assert!(!progress.contains("still working") && !progress.contains('\u{1b}'));
    let again = Command::new(env!("CARGO_BIN_EXE_oscar"))
        .args(args)
        .output()
        .unwrap();
    assert!(!again.status.success());

    let quiet_dir = output_dir();
    let quiet = Command::new(env!("CARGO_BIN_EXE_oscar"))
        .args([
            "run",
            "Document a parser",
            "--quiet",
            "--config",
            config.to_str().unwrap(),
            "--out",
            quiet_dir.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(quiet.status.success());
    assert!(quiet.stderr.is_empty());
    assert!(String::from_utf8_lossy(&quiet.stdout).contains("Outcome: Completed"));
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

#[test]
fn inference_failure_is_reported_without_a_saved_plan_and_can_be_overridden() {
    let parent = output_dir();
    fs::create_dir(&parent).unwrap();
    let config = parent.join("config.toml");
    fs::write(
        &config,
        include_str!("../../../example_config.toml")
            .replace("mode = \"heuristic\"", "mode = \"inference\""),
    )
    .unwrap();
    for command in ["plan", "run"] {
        let out = parent.join(command);
        let result = Command::new(env!("CARGO_BIN_EXE_oscar"))
            .args([
                command,
                "Document a parser",
                "--config",
                config.to_str().unwrap(),
                "--out",
                out.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(!result.status.success()); // Demo mock emits prose, not a plan.
        assert!(
            String::from_utf8_lossy(&result.stderr)
                .contains("Generating and validating plan with inference: failed")
        );
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join("planning.json")).unwrap()).unwrap();
        assert_eq!(report["outcome"], "failed");
        assert_eq!(report["call_started"], true);
        assert_eq!(report["simulation"], true);
        assert!(!out.join(".plan/plan.json").exists() && !out.join("run.json").exists());
    }
    let out = parent.join("override");
    let result = Command::new(env!("CARGO_BIN_EXE_oscar"))
        .args([
            "plan",
            "Document a parser",
            "--planner",
            "heuristic",
            "--config",
            config.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(!out.join("planning.json").exists());
    let loaded = parent.join("loaded");
    let result = Command::new(env!("CARGO_BIN_EXE_oscar"))
        .args([
            "run",
            "--plan",
            out.join(".plan/plan.json").to_str().unwrap(),
            "--config",
            config.to_str().unwrap(),
            "--out",
            loaded.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(!loaded.join("planning.json").exists());
}

#[cfg(feature = "http")]
#[test]
fn plan_and_run_use_http_inference_then_execute_the_generated_dag() {
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        process::Stdio,
        sync::mpsc,
        time::{Duration, Instant},
    };
    for command in ["plan", "run"] {
        let parent = output_dir();
        fs::create_dir(&parent).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let config = parent.join("config.toml");
        // plan exercises the CLI override; run exercises TOML activation.
        fs::write(
            &config,
            format!(
                r#"work_mode = "local"
[planning]
mode = "{}"
[providers.local]
enabled = true
provider = "openai_compatible"
model = "fixture"
api_url = "http://{addr}/v1/chat/completions"
context_window = 32768
capabilities = ["planning", "documentation"]
[limits]
call_timeout_ms = 15000
run_timeout_ms = 20000
"#,
                if command == "plan" {
                    "heuristic"
                } else {
                    "inference"
                }
            ),
        )
        .unwrap();
        let calls = if command == "run" { 2 } else { 1 };
        let (progress_tx, progress_rx) = mpsc::channel();
        let check_live_progress = command == "plan";
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for call in 0..calls {
                let deadline = Instant::now() + Duration::from_secs(10);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e)
                            if e.kind() == std::io::ErrorKind::WouldBlock
                                && Instant::now() < deadline =>
                        {
                            std::thread::sleep(Duration::from_millis(5))
                        }
                        Err(e) => panic!("fixture accept: {e}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut data = Vec::new();
                let mut buf = [0u8; 4096];
                let (header_end, length) = loop {
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    data.extend_from_slice(&buf[..n]);
                    assert!(data.len() < 65536);
                    if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&data[..end]);
                        let length = header
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|n| n.trim().parse::<usize>().unwrap())
                            })
                            .unwrap();
                        break (end + 4, length);
                    }
                };
                assert!(length < 65536);
                while data.len() < header_end + length {
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    data.extend_from_slice(&buf[..n]);
                }
                requests.push(
                    serde_json::from_slice::<serde_json::Value>(
                        &data[header_end..header_end + length],
                    )
                    .unwrap(),
                );
                if check_live_progress && call == 0 {
                    // Piped stderr emits the start immediately, then stays quiet
                    // while inference is pending (no animation or heartbeat).
                    assert_eq!(
                        progress_rx.recv_timeout(Duration::from_secs(8)).unwrap(),
                        "started"
                    );
                    assert!(matches!(
                        progress_rx.recv_timeout(Duration::from_millis(5200)),
                        Err(mpsc::RecvTimeoutError::Timeout)
                    ));
                }
                let content = if call == 0 {
                    serde_json::json!({"tasks":[{"id":"custom_schema","title":"Model-selected schema documentation","description":"Describe CSV column constraints and provide acceptance examples.","kind":"document","difficulty":"low","risk":"low","dependencies":[],"required_capabilities":["documentation"],"context_requirements":[],"expected_outputs":["Column specification"]}]}).to_string()
                } else {
                    "Generated schema proposal from fixture".into()
                };
                let body=serde_json::json!({"choices":[{"message":{"content":content}}],"usage":{"prompt_tokens":100,"completion_tokens":200}}).to_string();
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
            }
            requests
        });
        let out = parent.join("result");
        let mut process = Command::new(env!("CARGO_BIN_EXE_oscar"));
        process.args([
            command,
            "Document CSV constraints",
            "--config",
            config.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ]);
        if command == "plan" {
            process.args(["--planner", "inference"]);
        }
        process.stderr(Stdio::piped()).stdout(Stdio::piped());
        let mut child = process.spawn().unwrap();
        let stderr = child.stderr.take().unwrap();
        let reader = std::thread::spawn(move || {
            let mut collected = String::new();
            for line in BufReader::new(stderr).lines() {
                let line = line.unwrap();
                if check_live_progress {
                    if line == "[oscar] Generating and validating plan with inference" {
                        progress_tx.send("started").unwrap();
                    } else if line.contains("with inference") {
                        let _ = progress_tx.send("unexpected update");
                    }
                }
                collected.push_str(&line);
                collected.push('\n');
            }
            collected
        });
        let result = child.wait_with_output().unwrap();
        let stderr = reader.join().unwrap();
        assert!(result.status.success(), "{}", stderr);
        assert!(!stderr.contains("still working") && !stderr.contains('\u{1b}'));
        let requests = server.join().unwrap();
        assert_eq!(requests.len(), calls);
        assert!(
            requests[0]["messages"][1]["content"]
                .as_str()
                .unwrap()
                .contains("Create the FULL development plan")
        );
        let plan: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join(".plan/plan.json")).unwrap()).unwrap();
        assert_eq!(plan["tasks"][0]["id"], "custom_schema");
        assert!(
            fs::read_to_string(out.join("PLAN.md"))
                .unwrap()
                .contains("Model-selected schema documentation")
        );
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(out.join("planning.json")).unwrap()).unwrap();
        assert_eq!(report["outcome"], "completed");
        assert_eq!(report["usage"]["output"], 200);
        assert_eq!(report["simulation"], false);
        if command == "run" {
            let run: serde_json::Value =
                serde_json::from_slice(&fs::read(out.join("run.json")).unwrap()).unwrap();
            assert_eq!(run["outcome"], "completed");
            assert_eq!(run["calls"][0]["task"], "custom_schema");
            assert!(out.join("artifacts/custom_schema.md").exists());
            assert!(
                requests[1]["messages"][1]["content"]
                    .as_str()
                    .unwrap()
                    .contains("Describe CSV column constraints")
            );
        } else {
            assert!(!out.join("run.json").exists());
        }
        // Listener has closed: reuse must fail before making another call.
        let again = process.output().unwrap();
        assert!(!again.status.success());
        assert!(String::from_utf8_lossy(&again.stderr).contains("new directory"));
    }
}
