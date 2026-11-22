//! The wire contract, as whole replies: JSON-RPC envelope plus `result` or `error`.
//!
//! Objects are compared after parsing, so key order is not part of the contract. Each exchange
//! also checks the framing: exactly one compact line, terminated by `\n`.
//!
//! The tests below are deliberately split between the positive contract (P1) and the input
//! tolerances the server pins without validating (P4).

use serde_json::{Value, json};
use std::io::Cursor;
use sunrise_mcp::Server;

const SERVER: Server = Server::new("test-server", "1.2.3", "Do the thing.");

fn app(method: &str, params: &Value) -> Result<Value, (i64, String)> {
    match method {
        "tools/list" => Ok(json!({ "tools": [{ "name": "state" }] })),
        "tools/call" if params["name"] == "broken" => Err((-32602, "name is required".to_string())),
        "tools/call" => Ok(json!({
            "content": [{ "type": "text", "text": "ok" }],
            "isError": false,
        })),
        other => Err((-32601, format!("Method not found: {other}"))),
    }
}

/// The bytes one request line produces on the wire.
fn wire(line: &str) -> Vec<u8> {
    let mut out = Vec::new();
    SERVER
        .serve_on(Cursor::new(format!("{line}\n")), &mut out, app)
        .expect("in-memory wire");
    out
}

/// One request in, one parsed reply out; asserts the one-line framing.
fn reply(line: &str) -> Value {
    let out = wire(line);
    let text = std::str::from_utf8(&out).expect("wire output is UTF-8");
    let body = text.strip_suffix('\n').expect("reply ends with LF");
    let parsed = serde_json::from_str(body).expect("one JSON object");

    // Only whitespace outside strings would make the JSON non-compact. Do not pin key order.
    let mut in_string = false;
    let mut bytes = body.bytes();
    while let Some(byte) = bytes.next() {
        match byte {
            b'\\' if in_string => {
                let _ = bytes.next(); // An escaped quote does not end the string.
            }
            b'"' => in_string = !in_string,
            b' ' | b'\t' | b'\r' | b'\n' if !in_string => {
                panic!("expected compact JSON followed by one LF: {text:?}");
            }
            _ => {}
        }
    }
    parsed
}

fn request(id: i64, method: &str, params: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
}

/// The `_meta` a modern request must carry.
fn modern(mut params: Value, version: &str, capabilities: Value) -> Value {
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion": version,
        "io.modelcontextprotocol/clientCapabilities": capabilities,
    });
    params
}

fn server_info() -> Value {
    json!({ "name": "test-server", "version": "1.2.3" })
}

#[test]
fn legacy_initialize_goldens() {
    let expected = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "protocolVersion": "2025-06-18",
            "capabilities": { "tools": {} },
            "serverInfo": server_info(),
            "instructions": "Do the thing.",
        },
    });
    for params in [
        json!({ "protocolVersion": "2025-06-18" }),
        json!({ "protocolVersion": "2026-07-28" }),
        json!({ "protocolVersion": 7 }),
        json!({}),
    ] {
        let line = request(1, "initialize", params).to_string();
        assert_eq!(reply(&line), expected, "{line}");
    }
}

#[test]
fn modern_tools_list_golden() {
    let line = request(1, "tools/list", modern(json!({}), "2026-07-28", json!({}))).to_string();
    let expected = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "resultType": "complete",
            "tools": [{ "name": "state" }],
            "ttlMs": 30_000,
            "cacheScope": "public",
            "_meta": { "io.modelcontextprotocol/serverInfo": server_info() },
        },
    });
    assert_eq!(reply(&line), expected);
}

#[test]
fn modern_tools_call_golden() {
    let line = request(
        1,
        "tools/call",
        modern(json!({ "name": "state" }), "2026-07-28", json!({})),
    )
    .to_string();
    let expected = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "resultType": "complete",
            "content": [{ "type": "text", "text": "ok" }],
            "isError": false,
            "_meta": { "io.modelcontextprotocol/serverInfo": server_info() },
        },
    });
    assert_eq!(reply(&line), expected);
}

#[test]
fn discover_golden() {
    let line = request(1, "server/discover", json!({})).to_string();
    let expected = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": {
            "resultType": "complete",
            "supportedVersions": ["2026-07-28", "2025-06-18"],
            "capabilities": { "tools": {} },
            "_meta": { "io.modelcontextprotocol/serverInfo": server_info() },
            "instructions": "Do the thing.",
            "ttlMs": 3_600_000,
            "cacheScope": "public",
        },
    });
    assert_eq!(reply(&line), expected);
}

#[test]
fn legacy_tool_results_stay_unframed() {
    let list = reply(&request(1, "tools/list", json!({})).to_string());
    assert_eq!(
        list,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "tools": [{ "name": "state" }] },
        })
    );
    let call = reply(&request(2, "tools/call", json!({ "name": "state" })).to_string());
    assert_eq!(
        call,
        json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "content": [{ "type": "text", "text": "ok" }],
                "isError": false,
            },
        })
    );
}

#[test]
fn error_goldens() {
    let unknown = request(1, "tools/list", modern(json!({}), "1900-01-01", json!({}))).to_string();
    assert_eq!(
        reply(&unknown),
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32022,
                "message": "Unsupported protocol version",
                "data": { "supported": ["2026-07-28", "2025-06-18"], "requested": "1900-01-01" },
            },
        })
    );

    let mut missing = json!({});
    missing["_meta"] = json!({ "io.modelcontextprotocol/protocolVersion": "2026-07-28" });
    assert_eq!(
        reply(&request(1, "tools/list", missing).to_string()),
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32602,
                "message": "_meta.io.modelcontextprotocol/clientCapabilities is required",
            },
        })
    );

    let null_id = json!({ "jsonrpc": "2.0", "id": null, "method": "tools/list" }).to_string();
    assert_eq!(
        reply(&null_id),
        json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": {
                "code": -32600,
                "message": "A request's id is a string or an integer, not null",
            },
        })
    );

    assert_eq!(
        reply(&request(1, "nope/nope", json!({})).to_string()),
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32601, "message": "Method not found: nope/nope" },
        })
    );

    let app_error = request(1, "tools/call", json!({ "name": "broken" })).to_string();
    assert_eq!(
        reply(&app_error),
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32602, "message": "name is required" },
        })
    );
}

#[test]
fn ping_goldens_per_revision() {
    assert_eq!(
        reply(&request(1, "ping", json!({})).to_string()),
        json!({ "jsonrpc": "2.0", "id": 1, "result": {} })
    );
    assert_eq!(
        reply(&request(1, "ping", modern(json!({}), "2026-07-28", json!({}))).to_string()),
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32601, "message": "Method not found: ping" },
        })
    );
}

#[test]
fn parse_error_is_one_line_with_code_and_null_id() {
    // `serde_json`'s message is not ours to freeze; the code and id are.
    let reply = reply("{not json");
    assert_eq!(reply["jsonrpc"], "2.0");
    assert_eq!(reply["id"], Value::Null);
    assert_eq!(reply["error"]["code"], json!(-32700));
    assert!(reply["error"]["message"].is_string());
    assert!(reply["error"].get("data").is_none());
}

#[test]
fn notifications_get_no_line() {
    for line in [
        json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }),
        json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" }),
    ] {
        assert!(wire(&line.to_string()).is_empty(), "{line}");
    }
}

// P4: the tolerances below are compatibility decisions, pinned but not validated.

#[test]
fn non_object_messages_and_batches_are_ignored() {
    for line in [
        r#"[{"jsonrpc":"2.0","id":1,"method":"tools/list"}]"#,
        r#""a string""#,
        "7",
        "true",
        "null",
    ] {
        assert!(wire(line).is_empty(), "{line}");
    }
}

#[test]
fn a_response_object_is_not_special_cased() {
    // Its `id` makes it a request; the missing method is what `-32601` reports.
    let line = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "result": { "content": [] },
    });
    assert_eq!(
        reply(&line.to_string()),
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": { "code": -32601, "message": "Method not found: " },
        })
    );
}

#[test]
fn jsonrpc_is_not_required() {
    let line = json!({ "id": 1, "method": "ping", "params": {} }).to_string();
    assert_eq!(
        reply(&line),
        json!({ "jsonrpc": "2.0", "id": 1, "result": {} })
    );
}

#[test]
fn invalid_jsonrpc_values_are_ignored() {
    for version in [json!("1.0"), json!(false), Value::Null] {
        let line = json!({ "jsonrpc": version, "id": 1, "method": "ping", "params": {} });
        assert_eq!(
            reply(&line.to_string()),
            json!({ "jsonrpc": "2.0", "id": 1, "result": {} }),
            "{line}"
        );
    }
}

#[test]
fn a_non_null_id_is_echoed_unchanged() {
    for id in [
        json!("abc"),
        json!(true),
        json!({ "nested": 1 }),
        json!([1, 2]),
        json!(1.5),
    ] {
        let line = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "ping",
            "params": {},
        })
        .to_string();
        assert_eq!(reply(&line)["id"], id, "{line}");
    }
}

#[test]
fn client_capabilities_are_checked_for_presence_only() {
    for capabilities in [Value::Null, json!(0), json!("x")] {
        let line = request(
            1,
            "tools/list",
            modern(json!({}), "2026-07-28", capabilities.clone()),
        )
        .to_string();
        let reply = reply(&line);
        assert_eq!(
            reply["result"]["resultType"], "complete",
            "capabilities {capabilities}"
        );
    }
}

#[test]
fn omitted_params_reach_the_application_as_an_empty_object() {
    let mut app = |_: &str, params: &Value| Ok(json!({ "echo": params }));
    let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
    let reply = SERVER.handle_line(line, &mut app).unwrap();
    assert_eq!(
        reply,
        json!({ "jsonrpc": "2.0", "id": 1, "result": { "echo": {} } })
    );
}

#[test]
fn non_object_params_are_passed_through() {
    let mut app = |_: &str, params: &Value| Ok(json!({ "echo": params }));
    let line = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list",
        "params": [1, 2],
    })
    .to_string();
    let reply = SERVER.handle_line(&line, &mut app).unwrap();
    assert_eq!(reply["result"]["echo"], json!([1, 2]));
}

#[test]
fn initialize_answers_legacy_even_with_modern_meta() {
    let line = request(
        1,
        "initialize",
        modern(
            json!({ "protocolVersion": "2026-07-28" }),
            "2026-07-28",
            json!({}),
        ),
    )
    .to_string();
    let result = &reply(&line)["result"];
    assert_eq!(result["protocolVersion"], "2025-06-18");
    assert!(result.get("resultType").is_none());
}

#[test]
fn discover_answers_modern_even_with_an_unknown_version() {
    let line = request(
        1,
        "server/discover",
        modern(json!({}), "1900-01-01", json!({})),
    )
    .to_string();
    let result = &reply(&line)["result"];
    assert_eq!(result["resultType"], "complete");
    assert_eq!(
        result["supportedVersions"],
        json!(["2026-07-28", "2025-06-18"])
    );
}

#[test]
fn a_non_object_application_result_is_left_unframed() {
    let mut app = |_: &str, _: &Value| Ok(json!("not an object"));
    let line = request(1, "tools/call", modern(json!({}), "2026-07-28", json!({}))).to_string();
    let reply = SERVER.handle_line(&line, &mut app).unwrap();
    assert_eq!(reply["result"], "not an object");
    assert!(reply["result"].get("resultType").is_none());
}
