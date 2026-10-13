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

#[cfg(feature = "rag")]
#[test]
fn local_embeddings_are_rejected_in_strict_remote_mode_before_network_work() {
    let root = fixture();
    let config = root.join("remote.toml");
    fs::write(
        &config,
        include_str!("../../../settings.toml")
            .replace("work_mode = \"mixed\"", "work_mode = \"full_remote\""),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_oscar"))
        .args([
            "agent",
            "--config",
            config.to_str().unwrap(),
            "--workspace",
            root.to_str().unwrap(),
            "--rag",
            "--allow-remote-context",
        ])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("local RAG embeddings require local or mixed")
    );
}

#[cfg(feature = "rag")]
#[test]
fn rig_rag_session_uses_local_embeddings_and_clears_index_on_reset() {
    use std::{
        io::Read,
        time::{Duration, Instant},
    };
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let started = Instant::now();
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && started.elapsed() < Duration::from_secs(5) =>
                    {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(e) => panic!("embedding fixture: {e}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 8192];
            let _ = socket.read(&mut request).unwrap();
            let body = r#"{"embeddings":[[1.0,0.0]]}"#;
            let wire = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(wire.as_bytes()).unwrap();
        }
    });
    let output = invoke(
        &fixture(),
        "/read source.rs\nPropose tests\n/status\n/reset\n/status\n/exit\n",
        &[
            "--rag",
            "--embedding-url",
            &format!("http://{address}/api/embed"),
        ],
    );
    server.join().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("RAG chunks: 1; embedding calls: 2"));
    assert!(stdout.contains("RAG chunks: 0; embedding calls: 2"));
    assert!(stdout.contains("Mock proposal"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("RAG retrieved 1 chunks"));
    assert!(!stderr.contains("fn main"));
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

#[cfg(feature = "http")]
#[test]
fn native_reply_is_flushed_before_completion_and_failed_partial_is_reported() {
    use std::{
        io::{BufRead, BufReader, Read},
        sync::mpsc,
        time::Duration,
    };
    for fail in [false, true] {
        let root = fixture();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let config = root.join("stream.toml");
        fs::write(
            &config,
            format!(
                r#"
work_mode = "local"
[providers.local]
enabled = true
provider = "openai_compatible"
model = "fixture"
api_url = "http://{address}/v1/chat/completions"
context_window = 32768
capabilities = ["code_analysis", "code_generation"]
[limits]
call_timeout_ms = 5000
run_timeout_ms = 10000
"#
            ),
        )
        .unwrap();
        let (release, wait) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut data = Vec::new();
            let mut buf = [0; 4096];
            let (start, length) = loop {
                let count = socket.read(&mut buf).unwrap();
                assert!(count > 0);
                data.extend_from_slice(&buf[..count]);
                assert!(data.len() < 65_536);
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
            while data.len() < start + length {
                let count = socket.read(&mut buf).unwrap();
                assert!(count > 0);
                data.extend_from_slice(&buf[..count]);
            }
            let request: serde_json::Value =
                serde_json::from_slice(&data[start..start + length]).unwrap();
            assert_eq!(request["stream"], true);
            let first = serde_json::json!({"choices":[{"index":0,"delta":{"content":"early hé🦀\u{1b}\n"}}]});
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {first}\n\n").unwrap();
            // The terminal must flush the first delta before the server can finish.
            wait.recv_timeout(Duration::from_secs(5)).unwrap();
            let last = if fail {
                serde_json::json!({"choices":[{"index":0,"delta":{"tool_calls":[{"name":"shell"}]}}]})
            } else {
                serde_json::json!({"choices":[{"index":0,"delta":{"content":"final"},"finish_reason":"stop"}]})
            };
            write!(socket, "data: {last}\n\ndata: [DONE]\n\n").unwrap();
        });
        let mut child = Command::new(env!("CARGO_BIN_EXE_oscar"))
            .args([
                "agent",
                "Propose a change",
                "--config",
                config.to_str().unwrap(),
                "--workspace",
                root.to_str().unwrap(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (seen, observed) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut text = String::new();
            for line in BufReader::new(stdout).lines() {
                let line = line.unwrap();
                if line.starts_with("early") {
                    seen.send(line.clone()).unwrap();
                }
                text.push_str(&line);
                text.push('\n');
            }
            text
        });
        let early = observed.recv_timeout(Duration::from_secs(6));
        if early.is_err() {
            let _ = child.kill();
        }
        assert_eq!(early.unwrap(), "early hé🦀");
        release.send(()).unwrap();
        let result = child.wait_with_output().unwrap();
        let stdout = reader.join().unwrap();
        server.join().unwrap();
        assert_eq!(
            result.status.success(),
            !fail,
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(stdout.contains("final"), !fail);
        if fail {
            assert!(String::from_utf8_lossy(&result.stderr).contains("Partial reply discarded"));
        }
        assert!(!stdout.contains('\u{1b}'));
    }
}
