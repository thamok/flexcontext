//! Synchronous, newline-delimited MCP stdio server (2025-11-25).
use crate::SearchSession;
use anyhow::Result;
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{BufRead, Write};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QueryArgs {
    query: String,
    #[serde(default = "default_budget")]
    budget: usize,
    #[serde(default = "default_results")]
    max_results: usize,
}
fn default_budget() -> usize {
    4096
}
fn default_results() -> usize {
    12
}

pub fn serve(
    session: &mut SearchSession,
    input: impl BufRead,
    mut output: impl Write,
) -> Result<()> {
    let mut initialized = false;
    let mut ready = false;
    for line in input.lines() {
        let line = line?;
        let request: Value = match serde_json::from_str(&line) {
            Ok(value) => value,
            Err(_) => {
                write_response(&mut output, &error(Value::Null, -32700, "Parse error"))?;
                continue;
            }
        };
        let id = request.get("id").cloned();
        let method = request.get("method").and_then(Value::as_str);
        if request.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
            || method.is_none()
            || id
                .as_ref()
                .is_some_and(|id| !id.is_string() && !id.is_i64())
        {
            write_response(&mut output, &error(Value::Null, -32600, "Invalid Request"))?;
            continue;
        }
        let method = method.unwrap();
        if id.is_none() {
            if method == "notifications/initialized" && initialized {
                ready = true;
            }
            continue;
        }
        let id = id.unwrap();
        let params = &request["params"];
        let result = match method {
            "initialize" if !initialized => {
                if !params["protocolVersion"].is_string()
                    || !params["capabilities"].is_object()
                    || !params["clientInfo"].is_object()
                {
                    Err((-32602, "Invalid initialization parameters"))
                } else {
                    initialized = true;
                    Ok(
                        json!({"protocolVersion":"2025-11-25", "capabilities":{"tools":{}},
                        "serverInfo":{"name":"flexcontext", "version":env!("CARGO_PKG_VERSION")},
                        "instructions":"Searches a resident snapshot. Call refresh_index after repository edits."}),
                    )
                }
            }
            "ping" => Ok(json!({})),
            _ if !ready => Err((-32002, "Complete initialization first")),
            "tools/list" => Ok(json!({"tools":[
                {"name":"code_search", "description":"Retrieve diverse structural code context from the resident repository snapshot. Budget estimates source tokens only (bytes/4); metadata adds overhead.",
                 "inputSchema":{"type":"object", "properties":{"query":{"type":"string"},"budget":{"type":"integer","minimum":1,"default":4096},"max_results":{"type":"integer","minimum":1,"maximum":100,"default":12}},"required":["query"],"additionalProperties":false}},
                {"name":"refresh_index", "description":"Reload the repository snapshot after files are edited, added or deleted.", "inputSchema":{"type":"object","properties":{},"additionalProperties":false}}
            ]})),
            "tools/call" => match params["name"].as_str() {
                Some("code_search") => {
                    match serde_json::from_value::<QueryArgs>(params["arguments"].clone()) {
                        Ok(args)
                            if args.budget > 0
                                && (1..=100).contains(&args.max_results)
                                && args.budget.checked_mul(4).is_some() =>
                        {
                            match session.query(&args.query, args.budget * 4, args.max_results) {
                                Ok(response) => Ok(
                                    json!({"content":[{"type":"text","text":serde_json::to_string(&response)?}], "structuredContent":response, "isError":false}),
                                ),
                                Err(err) => Ok(tool_error(&err.to_string())),
                            }
                        }
                        _ => Err((-32602, "Invalid code_search arguments")),
                    }
                }
                Some("refresh_index")
                    if params.get("arguments").is_none()
                        || params["arguments"]
                            .as_object()
                            .is_some_and(|args| args.is_empty()) =>
                {
                    Ok(match session.refresh() {
                        Ok(()) => {
                            json!({"content":[{"type":"text","text":"Repository snapshot refreshed."}],"isError":false})
                        }
                        Err(err) => tool_error(&err.to_string()),
                    })
                }
                Some("refresh_index") => Err((-32602, "refresh_index takes no arguments")),
                _ => Err((-32602, "Unknown tool")),
            },
            _ => Err((-32601, "Method not found")),
        };
        let response = match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err((code, message)) => error(id, code, message),
        };
        write_response(&mut output, &response)?;
    }
    Ok(())
}
fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
fn tool_error(message: &str) -> Value {
    json!({"content":[{"type":"text","text":message}],"isError":true})
}
fn write_response(output: &mut impl Write, response: &Value) -> Result<()> {
    serde_json::to_writer(&mut *output, response)?;
    writeln!(output)?;
    output.flush()?;
    Ok(())
}
