//! The stdio line loop (`Server::serve_on`), on injected streams so no child process is needed.

use serde_json::{Value, json};
use std::cell::RefCell;
use std::io::{self, Cursor, Write};
use sunrise_mcp::Server;

const SERVER: Server = Server::new("test-server", "1.2.3", "Do the thing.");

fn app(method: &str, _params: &Value) -> Result<Value, (i64, String)> {
    match method {
        "tools/list" => Ok(json!({ "tools": [{ "name": "state" }] })),
        other => Err((-32601, format!("Method not found: {other}"))),
    }
}

fn serve(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    SERVER
        .serve_on(Cursor::new(input), &mut out, app)
        .expect("in-memory streams");
    out
}

fn request(id: i64) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/list",
        "params": {},
    })
    .to_string()
}

#[test]
fn two_requests_produce_two_lines() {
    let input = format!("{}\n{}\n", request(1), request(2));
    let out = serve(input.as_bytes());
    let text = std::str::from_utf8(&out).unwrap();
    assert_eq!(text.matches('\n').count(), 2, "{text:?}");
    let replies: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies[0]["id"], 1);
    assert_eq!(replies[1]["id"], 2);
}

#[test]
fn a_notification_produces_nothing() {
    let input = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }).to_string();
    assert!(serve(format!("{input}\n").as_bytes()).is_empty());
}

#[test]
fn blank_lines_are_skipped() {
    let input = format!("\n   \n\t\n{}\n\n", request(1));
    let out = serve(input.as_bytes());
    let text = std::str::from_utf8(&out).unwrap();
    assert_eq!(text.matches('\n').count(), 1, "{text:?}");
    assert_eq!(serde_json::from_str::<Value>(text.trim()).unwrap()["id"], 1);
}

#[test]
fn crlf_is_accepted_like_lf() {
    let lf = serve(format!("{}\n", request(1)).as_bytes());
    let crlf = serve(format!("{}\r\n", request(1)).as_bytes());
    assert_eq!(crlf, lf);
    assert_eq!(serde_json::from_slice::<Value>(&crlf).unwrap()["id"], 1);
}

#[test]
fn a_broken_line_is_one_parse_error_and_the_loop_continues() {
    let input = format!("{{not json\n{}\n", request(1));
    let out = serve(input.as_bytes());
    let text = std::str::from_utf8(&out).unwrap();
    assert_eq!(text.matches('\n').count(), 2, "{text:?}");
    let replies: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies[0]["error"]["code"], json!(-32700));
    assert_eq!(replies[0]["id"], Value::Null);
    assert_eq!(replies[1]["id"], 1);
}

#[test]
fn a_write_error_is_returned() {
    struct Failing;

    impl Write for Failing {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "no reader"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    let err = SERVER
        .serve_on(Cursor::new(format!("{}\n", request(1))), &mut Failing, app)
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
}

#[test]
fn a_flush_error_is_returned() {
    struct FailingFlush;

    impl Write for FailingFlush {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "flush failed"))
        }
    }

    let err = SERVER
        .serve_on(
            Cursor::new(format!("{}\n", request(1))),
            &mut FailingFlush,
            app,
        )
        .unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
}

#[test]
fn each_reply_is_flushed_before_the_next_request_is_handled() {
    struct Recorder<'a> {
        pending: Vec<u8>,
        flushed: &'a RefCell<Vec<Vec<u8>>>,
    }

    impl Write for Recorder<'_> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.pending.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushed
                .borrow_mut()
                .push(std::mem::take(&mut self.pending));
            Ok(())
        }
    }

    let flushed = RefCell::new(Vec::new());
    let mut recorder = Recorder {
        pending: Vec::new(),
        flushed: &flushed,
    };
    let input = format!("{}\n{}\n", request(1), request(2));
    let mut handled = 0;
    SERVER
        .serve_on(
            Cursor::new(input.as_bytes()),
            &mut recorder,
            |method, params| {
                assert_eq!(
                    flushed.borrow().len(),
                    handled,
                    "prior replies must already be visible"
                );
                handled += 1;
                app(method, params)
            },
        )
        .unwrap();

    let flushed = flushed.borrow();
    assert_eq!(flushed.len(), 2);
    for (bytes, id) in flushed.iter().zip([1, 2]) {
        let body = bytes
            .strip_suffix(b"\n")
            .expect("a complete reply at each flush");
        let reply: Value = serde_json::from_slice(body).expect("one reply per flush");
        assert_eq!(
            reply,
            json!({ "jsonrpc": "2.0", "id": id, "result": { "tools": [{ "name": "state" }] } })
        );
    }
    assert!(recorder.pending.is_empty(), "unflushed output");
}

#[test]
fn invalid_utf8_stops_the_loop_silently() {
    let mut input = format!("{}\n", request(1)).into_bytes();
    input.extend_from_slice(b"\xff\xfe\n");
    input.extend_from_slice(format!("{}\n", request(2)).as_bytes());
    let out = serve(&input);
    let text = std::str::from_utf8(&out).unwrap();
    assert_eq!(text.matches('\n').count(), 1, "{text:?}");
    assert_eq!(serde_json::from_str::<Value>(text.trim()).unwrap()["id"], 1);
}
