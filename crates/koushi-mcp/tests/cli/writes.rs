//! Write tools through the CLI: the host's consent decides, the caller's
//! `client_request_id` passes through untouched, and a deadline after
//! submission is reported as a timeout, never as a result.
use crate::support::cli;
use std::process::Stdio;

const SEND: &str = r#"{"account_id":"account:@member1:example.invalid","room_id":"!room:example.invalid","body":"SYNTHETIC_PRIVATE_PAYLOAD","client_request_id":"synthetic-request-1"}"#;

#[test]
fn a_write_without_its_required_request_id_is_refused_before_connecting() {
    use std::io::Write;
    let dir = tempfile::tempdir().expect("tempdir");
    let mut child = cli()
        .args(["send_message", "--args-stdin", "--endpoint"])
        .arg(dir.path().join("endpoint.json"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    let _ = child.stdin.take().expect("stdin").write_all(
        br#"{"account_id":"account:@member1:example.invalid","room_id":"!room:example.invalid","body":"SYNTHETIC_PRIVATE_PAYLOAD"}"#,
    );
    let output = child.wait_with_output().expect("output");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"koushi: usage_error\n");
}

#[test]
fn a_message_body_option_is_refused_without_echo() {
    let output = cli()
        .args([
            "send_message",
            "--account-id",
            "account:@member1:example.invalid",
            "--room-id",
            "!room:example.invalid",
            "--client-request-id",
            "synthetic-request-1",
            "--body",
            "SYNTHETIC_PRIVATE_PAYLOAD",
        ])
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"koushi: usage_error\n");
}

#[test]
fn an_attachment_path_option_is_refused_without_echo() {
    // A local path in argv reaches shell history and process listings.
    let output = cli()
        .args([
            "send_message",
            "--account-id",
            "account:@member1:example.invalid",
            "--room-id",
            "!room:example.invalid",
            "--client-request-id",
            "synthetic-request-1",
            "--attachments",
            "/synthetic/private-path.txt",
        ])
        .output()
        .expect("run");
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
    use std::process::Output;
    use std::time::{Duration, Instant};

    fn run_with_stdin(args: &[&str], endpoint: &std::path::Path, input: &str) -> Output {
        let mut child = cli()
            .args(args)
            .arg("--endpoint")
            .arg(endpoint)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(input.as_bytes())
            .expect("arguments");
        child.wait_with_output().expect("output")
    }

    #[test]
    fn the_largest_accepted_argument_object_still_fits_one_frame() {
        // The object shares the bridge frame with its request envelope, so an
        // object at the local bound must reach the app rather than come back as
        // `unsupported` after connecting; one byte past it is a usage error
        // before connecting.
        const HEAD: &str = r#"{"account_id":"account:@member1:example.invalid","query":""#;
        const TAIL: &str = r#""}"#;
        let pad = |size: usize| size - HEAD.len() - TAIL.len();
        let argument = |size: usize| format!("{HEAD}{}{TAIL}", "x".repeat(pad(size)));
        let bound = koushi_mcp::cli::MAX_ARGUMENT_BYTES;
        let (_dir, endpoint, server, _ready) = app(json!({"isError":false,"structuredContent":{}}));
        let output = run_with_stdin(
            &["search_messages", "--args-stdin", "--json"],
            &endpoint,
            &argument(bound),
        );
        assert!(output.status.success(), "{:?}", output.status);
        assert_eq!(
            server.join().expect("server")["arguments"]["query"]
                .as_str()
                .expect("query")
                .len(),
            pad(bound)
        );
        let output = run_with_stdin(
            &["search_messages", "--args-stdin", "--json"],
            &endpoint,
            &argument(bound + 1),
        );
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, b"koushi: usage_error\n");
    }

    #[test]
    fn a_write_the_host_has_not_approved_exits_5_with_no_result() {
        let (_dir, endpoint, server, _ready) = app(
            json!({"isError":true,"structuredContent":{"error":{"code":"consent_required","message":"SYNTHETIC_PRIVATE_PAYLOAD"}}}),
        );
        let output = run_with_stdin(&["send_message", "--args-stdin"], &endpoint, SEND);
        assert_eq!(output.status.code(), Some(5));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, b"koushi: consent_required\n");
        let request = server.join().expect("server");
        assert_eq!(
            request,
            json!({"name":"send_message","arguments":serde_json::from_str::<Value>(SEND).expect("send json")})
        );
    }

    #[test]
    fn a_retry_sends_the_same_request_id_and_the_cli_adds_none() {
        // First attempt: the host still holds the id as running or ambiguous.
        let (_dir, endpoint, first, _ready) = app(
            json!({"isError":true,"structuredContent":{"error":{"code":"busy","message":"duplicate_pending"}}}),
        );
        let output = run_with_stdin(&["send_message", "--args-stdin"], &endpoint, SEND);
        assert_eq!(output.status.code(), Some(6));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, b"koushi: busy\n");
        let first = first.join().expect("first");

        // The retry with the same id gets the recorded outcome.
        let outcome =
            json!({"event_id":"$sent:example.invalid","transaction_id":"mcp-synthetic-request-1"});
        let (_dir, endpoint, second, _ready) =
            app(json!({"isError":false,"structuredContent":outcome}));
        let output = run_with_stdin(&["send_message", "--args-stdin", "--json"], &endpoint, SEND);
        assert!(output.status.success());
        assert!(output.stderr.is_empty());
        let value: Value = serde_json::from_slice(&output.stdout).expect("result json");
        assert_eq!(value["tool"], "send_message");
        assert_eq!(value["account_id"], "account:@member1:example.invalid");
        assert_eq!(value["result"], outcome);
        assert_eq!(second.join().expect("second"), first);
        assert_eq!(
            first["arguments"]["client_request_id"],
            "synthetic-request-1"
        );

        // A write without a retry id in its schema gets none added.
        let (_dir, endpoint, server, _ready) = app(
            json!({"isError":false,"structuredContent":{"redaction_event_id":null,"already_redacted":true}}),
        );
        let output = cli()
            .args([
                "delete_message",
                "--account-id",
                "account:@member1:example.invalid",
                "--room-id",
                "!room:example.invalid",
                "--event-id",
                "$event:example.invalid",
                "--endpoint",
            ])
            .arg(endpoint)
            .output()
            .expect("run");
        assert!(output.status.success());
        assert_eq!(
            server.join().expect("server")["arguments"],
            json!({"account_id":"account:@member1:example.invalid","room_id":"!room:example.invalid","event_id":"$event:example.invalid"})
        );
    }

    #[test]
    fn a_deadline_after_submission_is_a_timeout_and_closes_the_connection() {
        // A null reply keeps the request unanswered after the app received it.
        let (_dir, endpoint, server, ready) = app(Value::Null);
        let mut child = cli()
            .args([
                "send_thread_reply",
                "--args-stdin",
                "--timeout",
                "1",
                "--endpoint",
            ])
            .arg(endpoint)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(
                br#"{"account_id":"account:@member1:example.invalid","room_id":"!room:example.invalid","thread_root_event_id":"$root:example.invalid","body":"SYNTHETIC_PRIVATE_PAYLOAD","client_request_id":"synthetic-request-2"}"#,
            )
            .expect("arguments");
        ready
            .recv_timeout(Duration::from_secs(5))
            .expect("the app received the write");
        let start = Instant::now();
        while child.try_wait().expect("poll").is_none() {
            if start.elapsed() > Duration::from_secs(5) {
                child.kill().expect("kill");
                child.wait().expect("reap");
                panic!("deadline did not end the write");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output().expect("output");
        assert_eq!(output.status.code(), Some(4));
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, b"koushi: timeout\n");
        // The fake app sees the CLI close the connection.
        let request = server.join().expect("server observed EOF");
        assert_eq!(
            request["arguments"]["client_request_id"],
            "synthetic-request-2"
        );
    }
}
