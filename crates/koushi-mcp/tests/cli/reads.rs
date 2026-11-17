//! Reads, input bounds, deadlines, output safety, and cancellation.
use crate::support::cli;
use std::process::Stdio;
use std::time::{Duration, Instant};

#[test]
fn rejects_writes_missing_accounts_and_sensitive_query_argv_without_echo() {
    for args in [
        vec!["send", "--body", "SYNTHETIC_PRIVATE_PAYLOAD"],
        vec!["rooms"],
        vec![
            "search",
            "--account",
            "account:@member1:example.invalid",
            "SYNTHETIC_PRIVATE_PAYLOAD",
        ],
        vec!["accounts", "--client-name", "codex"],
        vec![
            "rooms",
            "--account",
            "account:@member1:example.invalid",
            "--limit",
            "201",
        ],
    ] {
        let output = cli().args(args).output().expect("rejected process");
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, b"koushi: usage_error\n");
    }
}

#[test]
fn unavailable_app_has_a_stable_code_and_no_raw_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = cli()
        .args(["accounts", "--json", "--endpoint"])
        .arg(dir.path().join("SYNTHETIC_PRIVATE_PATH"))
        .output()
        .expect("unavailable process");
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"koushi: koushi_unavailable\n");
}

#[test]
fn a_deadline_exits_even_when_stdin_is_open_without_eof() {
    let mut child = cli()
        .args([
            "search",
            "--account",
            "account:@member1:example.invalid",
            "--query-stdin",
            "--timeout",
            "1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let input = child.stdin.take().expect("keep stdin open");
    let start = Instant::now();
    loop {
        if child.try_wait().expect("poll").is_some() {
            break;
        }
        if start.elapsed() > Duration::from_secs(5) {
            child.kill().expect("kill stuck process");
            child.wait().expect("reap");
            panic!("stdin blocked the command deadline");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(input);
    let output = child.wait_with_output().expect("output");
    assert_eq!(output.status.code(), Some(4));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"koushi: timeout\n");
}

#[test]
fn oversized_query_input_is_rejected_without_output_or_connection() {
    use std::io::Write;
    let mut child = cli()
        .args([
            "search",
            "--account",
            "account:@member1:example.invalid",
            "--query-stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let _ = child
        .stdin
        .take()
        .expect("stdin")
        .write_all(&vec![b'q'; 16 * 1024 + 1]);
    let output = child.wait_with_output().expect("output");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"koushi: usage_error\n");
}

#[cfg(unix)]
mod unix {
    use super::*;
    use crate::support::app;
    use serde_json::{Value, json};
    use std::io::Write;
    use std::process::Command;

    #[test]
    fn reads_use_the_explicit_account_and_all_strings_are_terminal_safe() {
        let private = "Synthetic \u{1b}]52;c;payload\u{7}\u{9b}31m\u{202e}\u{2066}\u{85}";
        for json_mode in [false, true] {
            let result = json!({"rooms":[{"display_name":private,"sender":private,"body":private}],"cursor":"next"});
            let (_dir, endpoint, server, _ready) =
                app(json!({"isError":false,"structuredContent":result}));
            let mut command = cli();
            command
                .args([
                    "rooms",
                    "--account",
                    "account:@member2:example.invalid",
                    "--limit",
                    "5",
                    "--cursor",
                    "next",
                    "--endpoint",
                ])
                .arg(endpoint);
            if json_mode {
                command.arg("--json");
            }
            let output = command.output().expect("run");
            assert!(output.status.success());
            assert!(output.stderr.is_empty());
            let text = String::from_utf8(output.stdout).expect("utf8");
            assert!(!text.chars().any(|c| matches!(
                c,
                '\u{1b}' | '\u{7}' | '\u{9b}' | '\u{202e}' | '\u{2066}' | '\u{85}'
            )));
            let value: Value = serde_json::from_str(&text).expect("result json");
            assert_eq!(value["schema_version"], 1);
            assert_eq!(value["account_id"], "account:@member2:example.invalid");
            assert_eq!(value["result"], result);
            let request = server.join().expect("server");
            assert_eq!(
                request,
                json!({"name":"list_rooms","arguments":{"account_id":"account:@member2:example.invalid","limit":5,"cursor":"next"}})
            );
        }
    }

    #[test]
    fn search_reads_query_from_stdin_and_remote_error_details_never_print() {
        let (_dir, endpoint, server, _ready) = app(
            json!({"isError":true,"structuredContent":{"error":{"code":"not_permitted","message":"SYNTHETIC_PRIVATE_PAYLOAD"}}}),
        );
        let mut child = cli()
            .args([
                "search",
                "--account",
                "account:@member1:example.invalid",
                "--room",
                "!room:example.invalid",
                "--query-stdin",
                "--endpoint",
            ])
            .arg(endpoint)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(b"SYNTHETIC_PRIVATE_QUERY\n")
            .expect("query");
        let output = child.wait_with_output().expect("output");
        assert_eq!(output.status.code(), Some(5));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, b"koushi: not_permitted\n");
        assert_eq!(
            server.join().expect("server")["arguments"],
            json!({"account_id":"account:@member1:example.invalid","query":"SYNTHETIC_PRIVATE_QUERY","room_ids":["!room:example.invalid"]})
        );
    }

    #[test]
    fn pretty_output_expansion_is_bounded_before_any_stdout() {
        let mut result = Value::Array(vec![Value::Null; 100000]);
        for _ in 0..100 {
            result = Value::Array(vec![result]);
        }
        let (_dir, endpoint, server, _ready) =
            app(json!({"isError":false,"structuredContent":result}));
        let output = cli()
            .args(["accounts", "--endpoint"])
            .arg(endpoint)
            .output()
            .expect("run");
        // #1902: the local output bound is an output failure (1), not an
        // account/room/event/operation failure (6).
        assert_eq!(output.status.code(), Some(1));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, b"koushi: output_too_large\n");
        server.join().expect("server");
    }

    #[test]
    fn a_closed_output_pipe_exits_cleanly() {
        let (_dir, endpoint, server, _ready) =
            app(json!({"isError":false,"structuredContent":{"accounts":[]}}));
        let mut child = cli()
            .args(["accounts", "--endpoint"])
            .arg(endpoint)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        drop(child.stdout.take());
        let output = child.wait_with_output().expect("output");
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        server.join().expect("server");
    }

    #[test]
    fn an_unread_output_pipe_does_not_block_deadline_or_ctrl_c() {
        use std::io::Read;
        for interrupt in [false, true] {
            let (_dir, endpoint, server, _ready) =
                app(json!({"isError":false,"structuredContent":{"body":"x".repeat(500000)}}));
            let mut command = cli();
            command
                .args(["accounts", "--json", "--endpoint"])
                .arg(endpoint);
            if !interrupt {
                command.args(["--timeout", "1"]);
            }
            let mut child = command
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .expect("spawn");
            let mut unread = child.stdout.take().expect("output pipe");
            // One byte synchronizes with actual output delivery; retain the
            // read end but do not drain it, so the writer must backpressure.
            let mut byte = [0u8; 1];
            unread.read_exact(&mut byte).expect("output began");
            if interrupt {
                assert!(
                    Command::new("kill")
                        .args(["-INT", &child.id().to_string()])
                        .status()
                        .expect("interrupt")
                        .success()
                );
            }
            let start = Instant::now();
            while child.try_wait().expect("poll").is_none() {
                if start.elapsed() > Duration::from_secs(5) {
                    child.kill().expect("kill");
                    child.wait().expect("reap");
                    panic!("unread stdout blocked exit");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            drop(unread);
            let output = child.wait_with_output().expect("output");
            assert_eq!(output.status.code(), Some(if interrupt { 130 } else { 4 }));
            assert_eq!(
                output.stderr,
                if interrupt {
                    b"koushi: cancelled\n".as_slice()
                } else {
                    b"koushi: timeout\n".as_slice()
                }
            );
            server.join().expect("server");
        }
    }

    #[test]
    fn ctrl_c_closes_the_active_socket_and_terminates() {
        let (_dir, endpoint, server, ready) = app(Value::Null);
        let mut child = cli()
            .args(["accounts", "--endpoint"])
            .arg(endpoint)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        ready
            .recv_timeout(Duration::from_secs(5))
            .expect("handler installed and request sent");
        assert!(
            Command::new("kill")
                .args(["-INT", &child.id().to_string()])
                .status()
                .expect("interrupt")
                .success()
        );
        let start = Instant::now();
        while child.try_wait().expect("poll").is_none() {
            if start.elapsed() > Duration::from_secs(5) {
                child.kill().expect("kill");
                child.wait().expect("reap");
                panic!("interrupt hung");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().expect("output");
        assert_eq!(output.status.code(), Some(130));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, b"koushi: cancelled\n");
        server.join().expect("server observed EOF");
    }
}
