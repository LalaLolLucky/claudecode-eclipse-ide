use super::*;
use serde_json::json;

fn args(op: serde_json::Value) -> Result<Vec<String>, String> {
    config_args(&op)
}

#[test]
fn stdio_add_matches_the_extension() {
    let a = args(json!({"op":"add","name":"tools","scope":"local","config":{
        "transport":"stdio","command":"npx","args":["-y","pkg"," "],"env":["A=1","B = two"]}}))
    .unwrap();
    assert_eq!(a, ["mcp", "add", "--scope", "local", "--transport", "stdio",
                   "--env", "A=1", "--env", "B=two", "--", "tools", "npx", "-y", "pkg"]);
}

#[test]
fn remote_adds_carry_headers_and_url() {
    for t in ["http", "sse"] {
        let a = args(json!({"op":"add","name":"r","scope":"project","config":{
            "transport":t,"url":" https://x.test/mcp ","headers":["Authorization: Bearer k"]}}))
        .unwrap();
        assert_eq!(a, ["mcp", "add", "--scope", "project", "--transport", t,
                       "--header", "Authorization: Bearer k", "--", "r", "https://x.test/mcp"]);
    }
}

#[test]
fn remove_takes_any_configured_name() {
    let a = args(json!({"op":"remove","name":"odd name","scope":"user"})).unwrap();
    assert_eq!(a, ["mcp", "remove", "--scope", "user", "--", "odd name"]);
}

#[test]
fn bad_input_is_refused_with_the_extension_wording() {
    let add = |name: &str, cfg: serde_json::Value| {
        args(json!({"op":"add","name":name,"scope":"local","config":cfg})).unwrap_err()
    };
    let stdio = json!({"transport":"stdio","command":"x"});
    assert_eq!(add("", stdio.clone()), "Server name is required.");
    assert!(add("a b", stdio.clone()).starts_with("Invalid name a b."));
    assert!(add("eclipse", stdio).contains("reserved"));
    assert_eq!(add("n", json!({"transport":"stdio","command":" "})), "Command is required.");
    assert_eq!(add("n", json!({"transport":"stdio","command":"x","env":["ok=1","=v"]})),
               "Environment variables must be KEY=value (line 2).");
    assert_eq!(add("n", json!({"transport":"http","url":""})), "URL is required.");
    assert_eq!(add("n", json!({"transport":"http","url":"u","headers":[":v"]})),
               "Headers must be \"Header-Name: value\" (line 1).");
    assert!(add("n", json!({"transport":"ws","url":"u"})).starts_with("Unknown transport"));
    assert!(args(json!({"op":"add","name":"n","scope":"global","config":{}})).unwrap_err()
        .starts_with("Unknown scope"));
}

#[test]
fn only_the_windows_requests_go_out() {
    let line = request_line("t1", &json!({"subtype":"mcp_toggle","serverName":"s","enabled":false})).unwrap();
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["type"], "control_request");
    assert_eq!(v["request_id"], "eclipse-mcp-t1");
    assert_eq!(v["request"]["enabled"], false);
    assert!(request_line("t1", &json!({"subtype":"mcp_set_servers","servers":{}})).is_none());
    assert!(request_line("t1", &json!({"subtype":"interrupt"})).is_none());
    assert!(request_line("bad\"id", &json!({"subtype":"mcp_status"})).is_none());
    assert!(request_line("", &json!({"subtype":"mcp_status"})).is_none());
}

#[test]
fn replies_are_matched_by_token() {
    let ok = reply_json(&json!({"subtype":"success","request_id":"eclipse-mcp-7",
                                "response":{"mcpServers":[]}})).unwrap();
    let v: serde_json::Value = serde_json::from_str(&ok).unwrap();
    assert_eq!(v, json!({"token":"7","ok":true,"response":{"mcpServers":[]}}));
    let err = reply_json(&json!({"subtype":"error","request_id":"eclipse-mcp-8",
                                 "error":"Server not found: nope"})).unwrap();
    let v: serde_json::Value = serde_json::from_str(&err).unwrap();
    assert_eq!(v, json!({"token":"8","ok":false,"error":"Server not found: nope"}));
    assert!(reply_json(&json!({"subtype":"success","request_id":"eclipse-live-1"})).is_none());
}

#[test]
fn debug_argv_hides_credentials() {
    let a = args(json!({"op":"add","name":"n","scope":"local","config":{
        "transport":"stdio","command":"run","args":["--token","s3cret"],"env":["KEY=s3cret"]}}))
    .unwrap();
    let r = redacted(&a);
    assert!(!r.contains("s3cret"), "{r}");
    assert_eq!(r, "mcp add --scope local --transport stdio --env KEY=<redacted> -- n …");
    let h = args(json!({"op":"add","name":"n","scope":"local","config":{
        "transport":"http","url":"https://u","headers":["Authorization: Bearer s3cret"]}}))
    .unwrap();
    assert_eq!(redacted(&h), "mcp add --scope local --transport http --header Authorization: <redacted> -- n …");
}

#[test]
fn the_server_view_log_is_redacted() {
    let op = json!({"op":"add","name":"n","scope":"local","config":{
        "transport":"stdio","command":"run","args":["s3cret"],"env":["KEY=s3cret"]}});
    let cwd = std::env::temp_dir();
    let (res, log) = edit_config_logged("no-such-claude-cli-x7", cwd.to_str().unwrap(), "t", &op.to_string());
    assert_eq!(res["ok"], false);
    assert_eq!(log.len(), 1, "{log:?}");
    assert!(log[0].starts_with("[mcp] claude mcp add --scope local --transport stdio --env KEY=<redacted> -- n …"),
            "{}", log[0]);
    assert!(!log.concat().contains("s3cret"));
}
