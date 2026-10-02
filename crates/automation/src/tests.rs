//! MCP protocol round-trips, in-process (headless) and against a fake control channel (bridge).

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use serde_json::{Value, json};

use crate::{Backend, McpServer, Session, base64, png_size};

fn server() -> McpServer {
    McpServer::new(Backend::headless(Session::default()))
}

/// Send a request, return its `result` (panics on a JSON-RPC error).
fn rpc(s: &mut McpServer, id: u64, method: &str, params: Value) -> Value {
    let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
    let reply: Value = serde_json::from_str(&s.handle_line(&line).expect("reply")).unwrap();
    assert_eq!(reply["jsonrpc"], "2.0");
    assert_eq!(reply["id"], id);
    assert!(reply.get("error").is_none(), "{method}: {reply}");
    reply["result"].clone()
}

/// Call a tool; returns (content, isError).
fn call(s: &mut McpServer, name: &str, args: Value) -> (Vec<Value>, bool) {
    let r = rpc(s, 99, "tools/call", json!({"name": name, "arguments": args}));
    (r["content"].as_array().unwrap().clone(), r["isError"].as_bool().unwrap())
}

/// Call a tool that returns JSON text; panics if it failed.
fn call_json(s: &mut McpServer, name: &str, args: Value) -> Value {
    let (c, err) = call(s, name, args.clone());
    assert!(!err, "{name} {args}: {c:?}");
    assert_eq!(c[0]["type"], "text");
    serde_json::from_str(c[0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn initialize_and_list_tools() {
    let mut s = server();
    let r = rpc(&mut s, 1, "initialize", json!({"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "t", "version": "0"}}));
    assert_eq!(r["protocolVersion"], "2025-03-26");
    assert_eq!(r["serverInfo"]["name"], "effectcraft");
    assert!(r["capabilities"]["tools"].is_object());
    // Unknown revision: we answer with our latest.
    let r = rpc(&mut s, 2, "initialize", json!({"protocolVersion": "1999-01-01"}));
    assert_eq!(r["protocolVersion"], crate::server::PROTOCOL_VERSIONS[0]);
    // Notifications get no reply.
    assert!(s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
    assert_eq!(rpc(&mut s, 3, "ping", json!({})), json!({}));

    let tools = rpc(&mut s, 4, "tools/list", json!({}))["tools"].as_array().unwrap().clone();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for want in [
        "list_commands",
        "execute_command",
        "get_project",
        "get_comp",
        "get_layer",
        "get_property",
        "set_property",
        "add_keyframe",
        "render_frame",
        "open_project",
        "save_project",
        "undo",
        "redo",
    ] {
        assert!(names.contains(&want), "missing {want}");
    }
    assert!(!names.contains(&"screenshot"), "bridge-only tools are hidden headless");
    for t in &tools {
        assert!(t["description"].as_str().unwrap().len() > 20);
        assert_eq!(t["inputSchema"]["type"], "object");
    }
}

#[test]
fn protocol_errors() {
    let mut s = server();
    let r: Value = serde_json::from_str(&s.handle_line("{not json").unwrap()).unwrap();
    assert_eq!(r["error"]["code"], -32700);
    let r: Value = serde_json::from_str(&s.handle_line(r#"{"jsonrpc":"2.0","id":5,"method":"nope"}"#).unwrap()).unwrap();
    assert_eq!(r["error"]["code"], -32601);
    let r: Value = serde_json::from_str(&s.handle_line(r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"screenshot"}}"#).unwrap()).unwrap();
    assert_eq!(r["error"]["code"], -32602);
    // Batches.
    let r: Value =
        serde_json::from_str(&s.handle_line(r#"[{"jsonrpc":"2.0","id":7,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/x"}]"#).unwrap()).unwrap();
    assert_eq!(r.as_array().unwrap().len(), 1);
    // Tool failures are in-band.
    let (c, err) = call(&mut s, "execute_command", json!({"command": "no.such"}));
    assert!(err);
    assert!(c[0]["text"].as_str().unwrap().contains("unknown command"));
    let (_, err) = call(&mut s, "render_frame", json!({}));
    assert!(err, "no comp yet");
    // Unknown params are rejected with the accepted keys, not silently ignored.
    let (c, err) = call(&mut s, "execute_command", json!({"command": "comp.new", "params": {"nmae": "X"}}));
    assert!(err);
    assert!(c[0]["text"].as_str().unwrap().contains("accepted: name, width"), "{c:?}");
}

#[test]
fn headless_workflow() {
    let mut s = server();
    rpc(&mut s, 1, "initialize", json!({"protocolVersion": "2025-06-18"}));

    let cmds = call_json(&mut s, "list_commands", json!({"filter": "layer.new"}));
    assert!(cmds.as_array().unwrap().iter().any(|c| c["id"] == "layer.newSolid"));

    call_json(&mut s, "execute_command", json!({"command": "comp.new", "params": {"name": "Main", "width": 320, "height": 180, "duration": 4}}));
    let l =
        call_json(&mut s, "execute_command", json!({"command": "layer.newSolid", "params": "{\"color\":\"#ff0000\",\"width\":100,\"height\":100}"}))["layer"]
            .clone();
    assert!(l.is_u64());

    let comp = call_json(&mut s, "get_comp", json!({"comp": "Main"}));
    assert_eq!(comp["width"], 320);
    assert_eq!(comp["layers"][0]["id"], l);

    // Static set + read back.
    let p = call_json(&mut s, "set_property", json!({"layer": l, "path": "transform/opacity", "value": 50}));
    assert_eq!(p["value"], 50.0);
    assert_eq!(call_json(&mut s, "get_property", json!({"layer": "#1", "path": "transform/opacity"}))["value"], 50.0);

    // Keyframes with easing.
    let p = call_json(
        &mut s,
        "add_keyframe",
        json!({"layer": l, "path": "transform/position", "keys": [{"time": 0, "value": [0, 90]}, {"time": 2, "value": [320, 90]}], "interpolation": "easyEase"}),
    );
    assert_eq!(p["keys"].as_array().unwrap().len(), 2);
    assert_eq!(p["keys"][0]["out"], "Bezier");
    let mid = call_json(&mut s, "get_property", json!({"layer": l, "path": "transform/position", "time": 1.0}));
    assert!((mid["value"][0].as_f64().unwrap() - 160.0).abs() < 1.0, "{mid}");

    // Expression via set_property.
    let p = call_json(&mut s, "set_property", json!({"layer": l, "path": "transform/rotation", "expression": "time * 90"}));
    assert_eq!(p["expression"], "time * 90");

    // Property tree carries paths; flat mode too.
    let tree = call_json(&mut s, "get_layer", json!({"layer": l, "flat": true}));
    let props = tree["properties"].as_array().unwrap();
    assert!(props.iter().any(|p| p["path"] == "transform/position" && p["keys"] == 2));

    // Render: an inline PNG at the requested size.
    let (c, err) = call(&mut s, "render_frame", json!({"time": 1.0, "max_side": 160}));
    assert!(!err, "{c:?}");
    assert_eq!(c[0]["type"], "image");
    assert_eq!(c[0]["mimeType"], "image/png");
    let png = base64::decode(c[0]["data"].as_str().unwrap()).unwrap();
    assert_eq!(png_size(&png), Some((160, 90)));
    let info: Value = serde_json::from_str(c[1]["text"].as_str().unwrap()).unwrap();
    assert_eq!(info["width"], 160);

    // Undo / redo.
    let before = call_json(&mut s, "get_project", json!({}))["undo"].as_array().unwrap().len();
    let u = call_json(&mut s, "undo", json!({"steps": 2}));
    assert_eq!(u["steps"], 2);
    assert_eq!(u["undoStack"].as_array().unwrap().len(), before - 2);
    call_json(&mut s, "redo", json!({"steps": 2}));
    assert_eq!(call_json(&mut s, "get_project", json!({}))["undo"].as_array().unwrap().len(), before);

    // Save + reopen.
    let dir = std::env::temp_dir().join(format!("ec-mcp-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.ecproj").to_string_lossy().into_owned();
    let saved = call_json(&mut s, "save_project", json!({"path": path}));
    assert_eq!(saved["dirty"], false);
    call_json(&mut s, "open_project", json!({"new": true}));
    assert!(call(&mut s, "get_comp", json!({})).1, "empty project has no comp");
    let proj = call_json(&mut s, "open_project", json!({"path": path}));
    assert_eq!(proj["items"].as_array().unwrap().len(), 3, "comp + Solids folder + solid: {proj}");
    let p = call_json(&mut s, "get_property", json!({"comp": "Main", "layer": "#1", "path": "transform/position"}));
    assert_eq!(p["keys"].as_array().unwrap().len(), 2);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn stdio_loop() {
    let mut s = server();
    let input = [
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "open_project", "arguments": {"demo": true}}}),
    ]
    .iter()
    .map(|v| v.to_string() + "\n")
    .collect::<String>();
    let mut out = Vec::new();
    s.serve(input.as_bytes(), &mut out).unwrap();
    let lines: Vec<Value> = String::from_utf8(out).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1]["id"], 2);
    assert_eq!(lines[1]["result"]["isError"], false);
}

/// A fake desktop control channel: answers `engine.execute` from a real session, and canned
/// `render.frame` / `ui.screenshot` replies.
fn fake_app() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        let (stream, _) = l.accept().unwrap();
        let mut out = stream.try_clone().unwrap();
        let mut session = Session::default();
        for line in BufReader::new(stream).lines() {
            let msg: Value = serde_json::from_str(&line.unwrap()).unwrap();
            let p = &msg["params"];
            let reply = match msg["method"].as_str().unwrap() {
                "engine.execute" => match session.execute(p["command"].as_str().unwrap(), p["params"].clone()) {
                    Ok(v) => json!({"ok": true, "result": v}),
                    Err(e) => json!({"ok": false, "error": e.to_string()}),
                },
                "render.frame" => {
                    let png = crate::encode_png(4, 2, vec![255; 32], 0).unwrap();
                    json!({"ok": true, "result": {"comp": 1, "time": 0.5, "width": 4, "height": 2, "png": base64::encode(&png)}})
                }
                "ui.screenshot" => {
                    let png = crate::encode_png(8, 8, vec![128; 256], 0).unwrap();
                    std::fs::write(p["path"].as_str().unwrap(), png).unwrap();
                    json!({"ok": true, "result": {"path": p["path"]}})
                }
                "ui.key" => json!({"ok": true, "result": null}),
                m => json!({"ok": false, "error": format!("unknown method `{m}`")}),
            };
            let mut reply = reply;
            reply["id"] = msg["id"].clone();
            writeln!(out, "{reply}").unwrap();
        }
    });
    port
}

#[test]
fn bridge_forwards_to_control_channel() {
    let port = fake_app();
    let mut s = McpServer::new(Backend::bridge(&port.to_string()).unwrap());
    let r = rpc(&mut s, 1, "initialize", json!({}));
    assert!(r["serverInfo"]["title"].as_str().unwrap().contains("bridge"));
    let names: Vec<String> =
        rpc(&mut s, 2, "tools/list", json!({}))["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    assert!(names.iter().any(|n| n == "screenshot") && names.iter().any(|n| n == "ui_click"));

    call_json(&mut s, "execute_command", json!({"command": "comp.new", "params": {"name": "B", "width": 64, "height": 64}}));
    assert_eq!(call_json(&mut s, "get_comp", json!({}))["name"], "B");

    let (c, err) = call(&mut s, "render_frame", json!({}));
    assert!(!err, "{c:?}");
    assert_eq!(png_size(&base64::decode(c[0]["data"].as_str().unwrap()).unwrap()), Some((4, 2)));

    let (c, err) = call(&mut s, "screenshot", json!({"max_side": 4}));
    assert!(!err, "{c:?}");
    assert_eq!(png_size(&base64::decode(c[0]["data"].as_str().unwrap()).unwrap()), Some((4, 4)));

    assert!(!call(&mut s, "ui_key", json!({"key": "Space"})).1);
    let (c, err) = call(&mut s, "control", json!({"method": "ui.nope"}));
    assert!(err && c[0]["text"].as_str().unwrap().contains("unknown method"));
}

#[test]
fn bridge_rejects_non_loopback() {
    assert!(Backend::bridge("10.0.0.1:9877").is_err());
    assert!(Backend::bridge("localhost:9877").is_ok());
}
