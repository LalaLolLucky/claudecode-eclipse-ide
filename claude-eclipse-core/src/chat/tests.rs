use super::build_user_content;
use serde_json::json;

#[test]
fn no_images_is_plain_string() {
    // Unchanged wire format when there are no images.
    assert_eq!(build_user_content("hello", ""), json!("hello"));
    assert_eq!(build_user_content("hello", "  "), json!("hello"));
    assert_eq!(build_user_content("hello", "[]"), json!("hello"));
}

#[test]
fn text_plus_image_becomes_content_blocks() {
    let imgs = r#"[{"media_type":"image/png","data":"QUJD"}]"#;
    assert_eq!(
        build_user_content("look", imgs),
        json!([
            { "type": "text", "text": "look" },
            { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "QUJD" } }
        ])
    );
}

#[test]
fn empty_message_omits_text_block() {
    let imgs = r#"[{"media_type":"image/jpeg","data":"eHl6"}]"#;
    assert_eq!(
        build_user_content("", imgs),
        json!([
            { "type": "image", "source": { "type": "base64", "media_type": "image/jpeg", "data": "eHl6" } }
        ])
    );
}

#[test]
fn media_type_defaults_to_png() {
    let imgs = r#"[{"data":"QQ=="}]"#;
    let v = build_user_content("", imgs);
    assert_eq!(v[0]["source"]["media_type"], "image/png");
}

#[test]
fn malformed_or_all_invalid_falls_back_to_string() {
    assert_eq!(build_user_content("hi", "not json"), json!("hi"));
    // images present but every one lacks data → plain string, not an empty array
    assert_eq!(build_user_content("hi", r#"[{"media_type":"image/png"}]"#), json!("hi"));
    // a document without data, an empty text block, an unknown type → nothing attached
    let junk = r#"[{"type":"document","source":{"type":"text","data":""}},{"type":"text","text":""},{"type":"video"}]"#;
    assert_eq!(build_user_content("hi", junk), json!("hi"));
}

#[test]
fn only_what_a_live_process_cannot_change_replaces_it() {
    use super::{spawn_signature, thinking_request, LiveSettings};
    // On a CLI that takes the allow flag, NO launch setting is in the signature:
    // every one of them is sent live, Auto included.
    let live = |perm: &str| spawn_signature("claude", "C:\\ws", 1, "tok", perm, true);
    assert_eq!(live("default"), live("plan"));
    assert_eq!(live("acceptEdits"), live("bypassPermissions"));
    // On one too old for it (or with the preference off), Auto is a launch-time
    // choice again, and going into or out of it replaces the process.
    let old = |perm: &str| spawn_signature("claude", "C:\\ws", 1, "tok", perm, false);
    assert_eq!(old("default"), old("plan"));
    assert_ne!(old("default"), old("bypassPermissions"));
    // Where it runs and what it talks to always part them.
    assert_ne!(live("default"), spawn_signature("claude", "C:\\other", 1, "tok", "default", true));
    assert_ne!(live("default"), spawn_signature("claude", "C:\\ws", 2, "tok", "default", true));

    // Model and effort are sent live (max included).
    let live = |effort: &str, model: &str| LiveSettings::new("default", effort, model, "2");
    assert!(live("max", "opus[1m]").reachable_from(&live("high", "sonnet"), false));
    // A Default tab adopting the model its first turn ran on keeps its process.
    assert!(live("high", "claude-sonnet-5").reachable_from(&live("high", ""), false));
    assert!(live("high", "sonnet").reachable_from(&live("", "sonnet"), false));
    // Back to Default: only when the CLI has no model setting of its own.
    assert!(live("high", "").reachable_from(&live("high", "sonnet"), true));
    assert!(!live("high", "").reachable_from(&live("high", "sonnet"), false));
    // Effort has no such reset, either way.
    assert!(!live("", "sonnet").reachable_from(&live("high", "sonnet"), true));

    assert_eq!(thinking_request("0")["max_thinking_tokens"], 0);
    let on = thinking_request("2");
    assert!(on["max_thinking_tokens"].is_null());
    assert_eq!(on["thinking_display"], "summarized");
    assert!(thinking_request("1").get("thinking_display").is_none());
}

#[test]
fn a_process_is_judged_by_the_session_it_resumes_until_init() {
    use super::serves_conversation as serves;
    // Before init (a process ensure_process has just started).
    assert!(serves(None, "s1", "s1"));
    assert!(serves(None, "", ""));
    assert!(!serves(None, "s1", ""));
    assert!(!serves(None, "", "s1"));
    // After init, by the session the CLI reported.
    assert!(serves(Some("s1"), "", "s1"));
    assert!(serves(Some("s1"), "s1", "s1"));
    assert!(!serves(Some("s1"), "s1", "s2"));
    assert!(!serves(Some("s1"), "", ""));
}

#[test]
fn chrome_set_reply_errors_are_worded_like_the_extension() {
    assert_eq!(super::chrome_set_error(&json!({"subtype":"success","response":{"added":["claude-in-chrome"],"removed":[],"errors":{}}})), None);
    assert_eq!(
        super::chrome_set_error(&json!({"subtype":"success","response":{"errors":{"claude-in-chrome":"spawn ENOENT","x":{"code":1}}}})),
        Some("claude-in-chrome: spawn ENOENT, x: {\"code\":1}".to_string())
    );
    assert_eq!(super::chrome_set_error(&json!({"subtype":"error","error":"bad request"})), Some("bad request".to_string()));
    assert_eq!(super::chrome_set_error(&json!({"subtype":"error"})), Some("Unknown error".to_string()));
    // A reply with no errors object at all is not a failure.
    assert_eq!(super::chrome_set_error(&json!({"subtype":"success","response":{}})), None);
}

#[test]
fn documents_and_text_blocks_pass_through_in_order() {
    let items = r#"[
            {"type":"document","source":{"type":"text","media_type":"text/plain","data":"abc"},"title":"a.txt"},
            {"media_type":"image/png","data":"QUJD"},
            {"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"JVBE"},"title":"b.pdf"},
            {"type":"text","text":"<browser tabGroupId=\"1\" tabId=\"2\"></browser>"}
        ]"#;
    assert_eq!(
        build_user_content("look", items),
        json!([
            { "type": "text", "text": "look" },
            { "type": "document", "source": { "type": "text", "media_type": "text/plain", "data": "abc" }, "title": "a.txt" },
            { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": "QUJD" } },
            { "type": "document", "source": { "type": "base64", "media_type": "application/pdf", "data": "JVBE" }, "title": "b.pdf" },
            { "type": "text", "text": "<browser tabGroupId=\"1\" tabId=\"2\"></browser>" }
        ])
    );
}

// ---- /usage parsing -------------------------------------------------
// The sample below is the VERBATIM stdout of `claude -p "/usage"` captured
// from the CLI on 2026-08-28; keep it byte-exact so a format change is
// caught here rather than in the status bar.
use super::{usage_json_from_text, percent_used_in};

const REAL_USAGE_OUTPUT: &str = "\
You are currently using your subscription to power your Claude Code usage

Current session: 44% used · resets Aug 29, 2:10am (Asia/Irkutsk)
Current week (all models): 64% used · resets Aug 31, 8am (Asia/Irkutsk)

What's contributing to your limits usage?
Approximate, based on local sessions on this machine — does not include other devices or claude.ai. Behaviors are independent characteristics, not a breakdown.

Last 24h · 485 requests · 16 sessions
  64% of your usage was at >150k context
  Top MCP servers: eclipse 2%

Last 7d · 1048 requests · 17 sessions
  87% of your usage was at >150k context
  75% of your usage came from sessions active for 8+ hours
  Top MCP servers: eclipse 1%";

#[test]
fn parses_both_windows_from_real_output() {
    let v: serde_json::Value =
        serde_json::from_str(&usage_json_from_text(REAL_USAGE_OUTPUT).unwrap()).unwrap();
    assert_eq!(v["rate_limits"]["five_hour"]["used_percentage"], 44);
    assert_eq!(v["rate_limits"]["seven_day"]["used_percentage"], 64);
}

#[test]
fn ignores_the_contributing_breakdown_percentages() {
    // "64% of your usage was at >150k context" must never be read as a
    // window value, and "Top MCP servers: eclipse 2%" must not either.
    let v: serde_json::Value =
        serde_json::from_str(&usage_json_from_text(REAL_USAGE_OUTPUT).unwrap()).unwrap();
    assert_eq!(v["rate_limits"].as_object().unwrap().len(), 2);
}

#[test]
fn weekly_requires_the_all_models_qualifier() {
    // A per-model weekly line must not be mistaken for the account weekly.
    let txt = "Current session: 10% used\nCurrent week (Opus): 90% used";
    let v: serde_json::Value = serde_json::from_str(&usage_json_from_text(txt).unwrap()).unwrap();
    assert_eq!(v["rate_limits"]["five_hour"]["used_percentage"], 10);
    assert!(v["rate_limits"].get("seven_day").is_none());
}

#[test]
fn one_window_alone_still_reports() {
    let txt = "Current session: 7% used · resets later";
    let v: serde_json::Value = serde_json::from_str(&usage_json_from_text(txt).unwrap()).unwrap();
    assert_eq!(v["rate_limits"]["five_hour"]["used_percentage"], 7);
    assert!(v["rate_limits"].get("seven_day").is_none());
}

#[test]
fn unrecognized_output_yields_none_not_wrong_numbers() {
    assert!(usage_json_from_text("").is_none());
    assert!(usage_json_from_text("Login required to view usage.").is_none());
    // Format drift: the labels changed → report nothing rather than guess.
    assert!(usage_json_from_text("5h window: 44% used\n7d window: 64% used").is_none());
}

#[test]
fn percent_scanner_handles_edges() {
    assert_eq!(percent_used_in("Current session: 0% used"), Some(0));
    assert_eq!(percent_used_in("Current session: 100% used"), Some(100));
    assert_eq!(percent_used_in("no digits % here"), None);
    assert_eq!(percent_used_in("nothing at all"), None);
}

// ---- isApiErrorMessage detection (assistant-branch dedup) -----------
// The object below is a VERBATIM capture of the synthetic "assistant" event
// the CLI emits for a session-limit-hit error (2.1.220, 2026-08-26) — it
// carries isApiErrorMessage:true so process_event_value can skip streaming
// it as ordinary text (the turn's is_error result already renders it once,
// via onError). If the CLI ever stops marking these, this test breaks
// instead of the duplicate line silently coming back.
const REAL_API_ERROR_EVENT: &str = r#"{
        "type": "assistant",
        "message": {
            "model": "<synthetic>",
            "role": "assistant",
            "content": [
                { "type": "text", "text": "You've hit your session limit · resets 2:10am (Asia/Irkutsk)" }
            ]
        },
        "error": "rate_limit",
        "isApiErrorMessage": true,
        "apiErrorStatus": 429
    }"#;

#[test]
fn is_api_error_flag_detected_on_real_event() {
    let v: serde_json::Value = serde_json::from_str(REAL_API_ERROR_EVENT).unwrap();
    assert_eq!(v["isApiErrorMessage"].as_bool().unwrap_or(false), true);
}

#[test]
fn is_api_error_flag_absent_on_ordinary_assistant_text() {
    let v: serde_json::Value = serde_json::json!({
        "type": "assistant",
        "message": { "content": [{ "type": "text", "text": "Sure, here's the fix." }] }
    });
    assert_eq!(v["isApiErrorMessage"].as_bool().unwrap_or(false), false);
}
