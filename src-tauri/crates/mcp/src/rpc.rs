//! The MCP wire protocol: JSON-RPC 2.0, one message per line, over stdio.
//!
//! Only the three methods a tool server needs are implemented — `initialize`,
//! `tools/list`, `tools/call` — plus the notifications a client sends and
//! expects no answer to. Anything else gets a "method not found", which is
//! what the specification asks for.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

/// The newest protocol revision this server was written against. Clients that
/// ask for an older one get their own version back, which is how the
/// specification says to negotiate.
const PROTOCOL_VERSION: &str = "2025-06-18";

pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
    /// JSON Schema for the arguments.
    pub schema: Value,
}

/// What a server must provide to be served over stdio.
pub trait Handler {
    fn tools(&self) -> Vec<Tool>;
    /// Returns the text to show, or an error message to show as a failure.
    fn call(&mut self, name: &str, args: &Value) -> Result<String, String>;
    fn server_name(&self) -> &'static str;
    fn server_version(&self) -> &'static str;
}

/// Read requests until the client closes stdin.
pub fn serve(handler: &mut impl Handler, input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Some(response) = handle_line(handler, &line) else {
            continue; // a notification: nothing to send back
        };
        writeln!(output, "{response}")?;
        output.flush()?;
    }
    Ok(())
}

fn handle_line(handler: &mut impl Handler, line: &str) -> Option<String> {
    let request: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        // No id to answer with, so the best we can do is say so once.
        Err(e) => return Some(error_response(Value::Null, -32700, &format!("parse error: {e}"))),
    };
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));

    // A message without an id is a notification: act on it, answer nothing.
    let id = id?;

    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(PROTOCOL_VERSION),
            "capabilities": { "tools": {} },
            "serverInfo": {
                "name": handler.server_name(),
                "version": handler.server_version(),
            },
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({
            "tools": handler
                .tools()
                .into_iter()
                .map(|t| json!({
                    "name": t.name,
                    "description": t.description,
                    "inputSchema": t.schema,
                }))
                .collect::<Vec<_>>(),
        })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            // A failing tool is a result with isError, not a protocol error:
            // that way the model sees the message and can correct itself.
            let (text, failed) = match handler.call(name, &args) {
                Ok(text) => (text, false),
                Err(message) => (message, true),
            };
            Ok(json!({ "content": [{ "type": "text", "text": text }], "isError": failed }))
        }
        other => Err((-32601, format!("method not found: {other}"))),
    };

    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string(),
        Err((code, message)) => error_response(id, code, &message),
    })
}

fn error_response(id: Value, code: i32, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

/* ---------- argument helpers ---------- */

pub fn need_i64(args: &Value, key: &str) -> Result<i64, String> {
    args.get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("{key} は必須です（整数）"))
}

pub fn opt_i64(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(Value::as_i64)
}

pub fn need_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} は必須です（文字列）"))
}

pub fn opt_str(args: &Value, key: &str) -> Option<String> {
    args.get(key).and_then(Value::as_str).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;

    impl Handler for Fake {
        fn tools(&self) -> Vec<Tool> {
            vec![Tool { name: "echo", description: "says it back", schema: json!({"type": "object"}) }]
        }
        fn call(&mut self, name: &str, args: &Value) -> Result<String, String> {
            match name {
                "echo" => Ok(args.get("text").and_then(Value::as_str).unwrap_or("").to_owned()),
                _ => Err(format!("unknown tool: {name}")),
            }
        }
        fn server_name(&self) -> &'static str {
            "fake"
        }
        fn server_version(&self) -> &'static str {
            "0"
        }
    }

    fn exchange(input: &str) -> Vec<Value> {
        let mut out = Vec::new();
        serve(&mut Fake, input.as_bytes(), &mut out).unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[test]
    fn negotiates_and_lists_tools() {
        let replies = exchange(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05"}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
"#,
        );
        assert_eq!(replies.len(), 2, "a notification gets no reply");
        assert_eq!(replies[0]["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(replies[0]["result"]["serverInfo"]["name"], "fake");
        assert_eq!(replies[1]["result"]["tools"][0]["name"], "echo");
    }

    #[test]
    fn a_failing_tool_is_a_result_not_a_protocol_error() {
        let replies = exchange(
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"echo","arguments":{"text":"hi"}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"nope"}}
{"jsonrpc":"2.0","id":3,"method":"wat"}
"#,
        );
        assert_eq!(replies[0]["result"]["content"][0]["text"], "hi");
        assert_eq!(replies[0]["result"]["isError"], false);
        assert_eq!(replies[1]["result"]["isError"], true);
        assert!(replies[1]["result"]["content"][0]["text"].as_str().unwrap().contains("nope"));
        assert_eq!(replies[2]["error"]["code"], -32601);
    }

    #[test]
    fn broken_json_does_not_stop_the_server() {
        let replies = exchange("not json\n{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"ping\"}\n");
        assert_eq!(replies[0]["error"]["code"], -32700);
        assert_eq!(replies[1]["id"], 9);
    }
}
