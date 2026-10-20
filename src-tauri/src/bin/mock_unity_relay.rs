use std::io::{BufRead, Write};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Validate that --mcp was passed
    if !args.contains(&"--mcp".to_string()) {
        eprintln!("Missing required --mcp flag");
        std::process::exit(2);
    }

    let exe_name = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
        .unwrap_or_default()
        .to_lowercase();

    if exe_name.contains("exit") {
        std::process::exit(1);
    }
    if exe_name.contains("hang") {
        std::thread::sleep(std::time::Duration::from_secs(30));
        return;
    }

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }

        if exe_name.contains("malformed") {
            let _ = writeln!(stdout, "not valid json");
            let _ = stdout.flush();
        } else if exe_name.contains("oversize") {
            let huge = "x".repeat(1024 * 1024 + 100);
            let _ = writeln!(stdout, "{huge}");
            let _ = stdout.flush();
        } else if exe_name.contains("mismatched_id") {
            let _ = writeln!(
                stdout,
                r#"{{"jsonrpc":"2.0","id":9999,"result":{{"protocolVersion":"2024-11-05"}}}}"#
            );
            let _ = stdout.flush();
        } else if exe_name.contains("unsupported_version") {
            let _ = writeln!(
                stdout,
                r#"{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":"99.0.0"}}}}"#
            );
            let _ = stdout.flush();
        } else {
            // Normal mode: echo the request ID per standard JSON-RPC 2.0
            let parsed = serde_json::from_str::<serde_json::Value>(&line).ok();
            let req_id = parsed
                .as_ref()
                .and_then(|v| v.get("id").cloned())
                .unwrap_or(serde_json::json!(1));
            let method = parsed.as_ref().and_then(|v| {
                v.get("method")
                    .and_then(|m| m.as_str())
                    .map(|s| s.to_string())
            });

            match method.as_deref() {
                Some("initialize") => {
                    let _ = writeln!(
                        stdout,
                        r#"{{"jsonrpc":"2.0","id":{},"result":{{"protocolVersion":"2024-11-05","serverInfo":{{"name":"Unity AI Assistant Relay","version":"2.7.0"}},"capabilities":{{"tools":{{}},"resources":{{}}}}}}}}"#,
                        req_id
                    );
                    let _ = stdout.flush();
                }
                Some("notifications/initialized") => {
                    // Standard notification, send no response
                }
                Some("tools/list") => {
                    let _ = writeln!(
                        stdout,
                        r#"{{"jsonrpc":"2.0","id":{},"result":{{"tools":[{{"name":"unity_get_scene","description":"Get current scene hierarchy","inputSchema":{{"type":"object"}}}},{{"name":"unity_refresh","description":"Refresh assets","inputSchema":{{"type":"object"}}}}]}}}}"#,
                        req_id
                    );
                    let _ = stdout.flush();
                }
                Some("resources/list") => {
                    let _ = writeln!(
                        stdout,
                        r#"{{"jsonrpc":"2.0","id":{},"result":{{"resources":[{{"uri":"unity://scene/hierarchy","name":"Scene Hierarchy","description":"Current scene game objects","mimeType":"application/json"}}]}}}}"#,
                        req_id
                    );
                    let _ = stdout.flush();
                }
                _ => {}
            }
        }
    }
}
