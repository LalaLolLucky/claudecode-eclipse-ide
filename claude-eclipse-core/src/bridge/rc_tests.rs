use super::*;

#[test]
fn request_line_is_exact() {
    let (id, line) = rc_request_line(7, true);
    assert_eq!(id, "eclipse-rc-7");
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["type"], "control_request");
    assert_eq!(v["request_id"], "eclipse-rc-7");
    assert_eq!(v["request"]["subtype"], "remote_control");
    assert_eq!(v["request"]["enabled"], true);

    let (_, off) = rc_request_line(8, false);
    let v2: serde_json::Value = serde_json::from_str(&off).unwrap();
    assert_eq!(v2["request"]["enabled"], false);
}

#[test]
fn ids_are_unique_per_sequence() {
    assert_ne!(rc_request_line(1, true).0, rc_request_line(2, true).0);
}

#[test]
fn reattach_hands_the_old_bridge_session_to_the_new_process() {
    let (id, line) = rc_reattach_line(3, "cse_01YS1GZJiryQChtiFb2D94ei");
    let v: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert_eq!(v["request"]["subtype"], "remote_control");
    assert_eq!(v["request"]["enabled"], true);
    assert_eq!(v["request"]["reattach_session_id"], "cse_01YS1GZJiryQChtiFb2D94ei");
    assert!(rc_owns_response(&id) && rc_is_reattach(&id));
    assert!(!rc_is_reattach(&rc_request_line(3, true).0));

    let (_, fresh) = rc_reattach_line(4, "");
    let v: serde_json::Value = serde_json::from_str(&fresh).unwrap();
    assert!(v["request"].get("reattach_session_id").is_none());
}

#[test]
fn bridge_epoch_rides_along_to_the_page() {
    let inner: serde_json::Value = serde_json::from_str(
        r#"{"subtype":"success","request_id":"eclipse-rc-1","response":{
                 "session_url":"https://claude.ai/code/session_01","bridge_epoch":3,
                 "bridge_session_id":"cse_01"}}"#,
    )
    .unwrap();
    let j: serde_json::Value = serde_json::from_str(&rc_reply_json(&rc_parse_reply(&inner))).unwrap();
    assert_eq!(j["bridgeEpoch"], 3);

    let s: serde_json::Value = serde_json::from_str(&rc_bridge_state_json("failed", Some(3))).unwrap();
    assert_eq!(s["bridgeState"], "failed");
    assert_eq!(s["bridgeEpoch"], 3);
    let bare: serde_json::Value = serde_json::from_str(&rc_bridge_state_json("failed", None)).unwrap();
    assert!(bare.get("bridgeEpoch").is_none());
}

#[test]
fn only_our_own_ids_are_claimed() {
    assert!(rc_owns_response("eclipse-rc-1"));
    assert!(!rc_owns_response("eclipse-ren-1"));
    assert!(!rc_owns_response("eclipse-mode-1"));
    assert!(!rc_owns_response(""));
}

/// The live reply, verbatim from claude 2.1.251.
#[test]
fn parses_the_real_reply() {
    let inner: serde_json::Value = serde_json::from_str(
        r#"{"subtype":"success","request_id":"eclipse-rc-1","response":{
                 "session_url":"https://claude.ai/code/session_01DAFk4EDDmKpExCkaKCCosU",
                 "connect_url":"https://claude.ai/code?environment=",
                 "environment_id":"","bridge_epoch":1,
                 "bridge_session_id":"cse_01DAFk4EDDmKpExCkaKCCosU"}}"#,
    )
    .unwrap();
    let r = rc_parse_reply(&inner);
    assert!(r.enabled);
    assert_eq!(r.url, "https://claude.ai/code/session_01DAFk4EDDmKpExCkaKCCosU");
    assert_eq!(r.bridge_session_id, "cse_01DAFk4EDDmKpExCkaKCCosU");

    let j: serde_json::Value = serde_json::from_str(&rc_reply_json(&r)).unwrap();
    assert_eq!(j["enabled"], true);
    assert_eq!(j["url"], "https://claude.ai/code/session_01DAFk4EDDmKpExCkaKCCosU");
    assert_eq!(j["bridgeSessionId"], "cse_01DAFk4EDDmKpExCkaKCCosU");
    assert!(j["error"].is_null());
}

/// A success with no url is not Remote Control being on — the indicator
/// would otherwise light with nothing to link to.
#[test]
fn a_success_without_a_url_is_not_enabled() {
    let inner: serde_json::Value =
        serde_json::from_str(r#"{"subtype":"success","response":{}}"#).unwrap();
    assert!(!rc_parse_reply(&inner).enabled);
}

#[test]
fn a_failure_carries_its_error_through() {
    let inner: serde_json::Value = serde_json::from_str(
        r#"{"subtype":"error","error":"policy denied","response":{}}"#,
    )
    .unwrap();
    let r = rc_parse_reply(&inner);
    assert!(!r.enabled);
    let j: serde_json::Value = serde_json::from_str(&rc_reply_json(&r)).unwrap();
    assert_eq!(j["error"], "policy denied");
}

#[test]
fn state_json_is_exact() {
    let j: serde_json::Value = serde_json::from_str(&rc_state_json("connected")).unwrap();
    assert_eq!(j["bridgeState"], "connected");
    assert_eq!(j.as_object().unwrap().len(), 1, "one key only");
}

/// A locally sent message never comes back on stdout (verified against
/// claude 2.1.251), so anything this returns is somebody else typing.
/// Everything the CLI routes through a `user` event for its own reasons has
/// to be filtered out here, or plumbing renders as speech.
#[test]
fn plain_text_from_another_device_is_a_message() {
    let e: serde_json::Value = serde_json::from_str(
        r#"{"type":"user","message":{"role":"user","content":"hello from my phone"}}"#).unwrap();
    assert_eq!(rc_incoming_text(&e).as_deref(), Some("hello from my phone"));
}

#[test]
fn text_blocks_are_joined() {
    let e: serde_json::Value = serde_json::from_str(
        r#"{"type":"user","message":{"role":"user","content":[
                 {"type":"text","text":"first"},{"type":"text","text":"second"}]}}"#).unwrap();
    assert_eq!(rc_incoming_text(&e).as_deref(), Some("first
second"));
}

#[test]
fn tool_results_are_not_messages() {
    // The CLI feeds tool output back as a user turn; onToolEnd already owns it.
    let e: serde_json::Value = serde_json::from_str(
        r#"{"type":"user","message":{"role":"user","content":[
                 {"type":"tool_result","tool_use_id":"x","content":"out"}]}}"#).unwrap();
    assert!(rc_incoming_text(&e).is_none());
}

/// The dangerous shape: a tool_result with a text block riding along would
/// otherwise surface half a tool result as if someone had said it.
#[test]
fn a_tool_result_with_text_alongside_is_still_not_a_message() {
    let e: serde_json::Value = serde_json::from_str(
        r#"{"type":"user","message":{"role":"user","content":[
                 {"type":"text","text":"here you go"},
                 {"type":"tool_result","tool_use_id":"x","content":"out"}]}}"#).unwrap();
    assert!(rc_incoming_text(&e).is_none());
}

#[test]
fn synthetic_and_meta_turns_are_not_messages() {
    // Compact summaries arrive synthetic; the CLI injects meta notices such as
    // "This session is being continued from another machine".
    let syn: serde_json::Value = serde_json::from_str(
        r#"{"type":"user","isSynthetic":true,"message":{"role":"user","content":"summary"}}"#).unwrap();
    assert!(rc_incoming_text(&syn).is_none());
    let meta: serde_json::Value = serde_json::from_str(
        r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"continued elsewhere"}}"#).unwrap();
    assert!(rc_incoming_text(&meta).is_none());
}

#[test]
fn empty_and_malformed_turns_yield_nothing() {
    for raw in [
        r#"{"type":"user","message":{"role":"user","content":""}}"#,
        r#"{"type":"user","message":{"role":"user","content":"   "}}"#,
        r#"{"type":"user","message":{"role":"user","content":[]}}"#,
        r#"{"type":"user","message":{"role":"user","content":[{"type":"image"}]}}"#,
        r#"{"type":"user"}"#,
        r#"{}"#,
    ] {
        let e: serde_json::Value = serde_json::from_str(raw).unwrap();
        assert!(rc_incoming_text(&e).is_none(), "should be ignored: {}", raw);
    }
}

#[test]
fn surrounding_whitespace_is_trimmed() {
    let e: serde_json::Value = serde_json::from_str(
        r#"{"type":"user","message":{"role":"user","content":"  spaced  "}}"#).unwrap();
    assert_eq!(rc_incoming_text(&e).as_deref(), Some("spaced"));
}

/// The inbound path, pinned against the real trace. When a message arrives
/// over the bridge the CLI emits `command_lifecycle` and NOT a user event,
/// so the text is looked up by uuid; these guard the matching.
#[test]
fn a_lifecycle_announcement_carries_no_text() {
    // Verbatim from the live trace, so the shape cannot drift unnoticed.
    let e: serde_json::Value = serde_json::from_str(
        r#"{"type":"command_lifecycle","command_uuid":"11111111-2222-4333-8444-555555555555",
                 "state":"queued","uuid":"9de9f4d4","session_id":"d4bca867"}"#).unwrap();
    assert_eq!(e["state"], "queued");
    assert_eq!(e["command_uuid"], "11111111-2222-4333-8444-555555555555");
    // Nothing message-shaped to read — hence the lookup.
    assert!(e["message"].is_null());
    assert!(rc_incoming_text(&e).is_none());
}

#[test]
fn a_looked_up_payload_yields_its_text() {
    // The payload as the events log stores it for a client-typed message.
    let payload: serde_json::Value = serde_json::from_str(
        r#"{"type":"user","client_platform":"claude_code_cli",
                 "message":{"role":"user","content":"ping from a client"},
                 "session_id":"cse_x","uuid":"11111111-2222-4333-8444-555555555555",
                 "parent_tool_use_id":null}"#).unwrap();
    assert_eq!(rc_incoming_text(&payload).as_deref(), Some("ping from a client"));
}

#[test]
fn lookup_refuses_to_call_out_with_nothing_to_match() {
    assert!(rc_lookup_message("", "", "some-uuid").is_none());
    assert!(rc_lookup_message("", "cse_x", "").is_none());
}

/// The QR has to actually scan, so the properties decoders rely on are
/// asserted rather than eyeballed: real modules, the mandatory quiet zone,
/// and hard black-on-white contrast.
#[test]
fn renders_a_scannable_qr() {
    let svg = rc_qr_svg("https://claude.ai/code/session_01DAFk4EDDmKpExCkaKCCosU");
    assert!(svg.starts_with("<svg"), "{}", &svg[..svg.len().min(80)]);
    assert!(svg.contains("viewBox=\"0 0 "));
    assert!(svg.contains("fill=\"#ffffff\""), "needs a light background");
    assert!(svg.contains("fill=\"#000000\""), "needs dark modules");
    assert!(svg.contains("crispEdges"), "must not blur at the module edges");
    // Plenty of modules — a path with only a handful would mean it encoded
    // nothing useful.
    assert!(svg.matches('M').count() > 100, "too few modules: {}", svg.matches('M').count());
}

#[test]
fn the_quiet_zone_is_present_on_every_side() {
    // The viewBox is the code plus 4 modules of margin each side, and no
    // module may be drawn inside that margin.
    let svg = rc_qr_svg("https://claude.ai/code/session_01ABC");
    let vb = svg.split("viewBox=\"0 0 ").nth(1).unwrap();
    let side: usize = vb.split(' ').next().unwrap().parse().unwrap();
    // Every module coordinate lies within [4, side-5].
    for seg in svg.split('M').skip(1) {
        let coords: Vec<&str> = seg.split('h').next().unwrap().split(' ').collect();
        let x: usize = coords[0].parse().unwrap();
        let y: usize = coords[1].parse().unwrap();
        assert!(x >= 4 && y >= 4, "module inside the quiet zone at {},{}", x, y);
        assert!(x < side - 4 && y < side - 4, "module past the quiet zone at {},{}", x, y);
    }
}

#[test]
fn a_longer_url_still_encodes() {
    // Version scales with length; a long session url must not fall over.
    let long = format!("https://claude.ai/code/session_{}", "0123456789".repeat(8));
    assert!(rc_qr_svg(&long).starts_with("<svg"));
}

#[test]
fn nothing_to_encode_yields_nothing_to_show() {
    assert_eq!(rc_qr_svg(""), "");
}
