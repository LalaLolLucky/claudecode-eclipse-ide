use std::fs;
use std::sync::Mutex;

/// The tests below all repoint the home env var (USERPROFILE / HOME) at a
/// per-test fake home; serialize them so parallel test threads don't clobber
/// each other's environment.
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn set_home(home: &std::path::Path) {
    #[cfg(windows)]
    std::env::set_var("USERPROFILE", home);
    #[cfg(not(windows))]
    std::env::set_var("HOME", home);
}

/// Hermetic fixture (same one the PHP reader was verified against): builds a
/// fake home + ~/.claude/projects/<hash>/ under a temp dir and points the
/// home env var at it, so the test runs anywhere. Asserts: custom-title beats
/// ai-title (LAST custom-title wins), ai-title-only stubs are listed (with an
/// mtime-derived sort key), untitled sessions fall back to the stripped first
/// user message, and ordering is last-activity descending.
/// The Remote Control inbound lookup, against the shape a real bridge session
/// leaves on disk (taken from one: a message typed on a phone is an ordinary
/// `user` line stamped with the HOST's entrypoint/promptSource, so the uuid is
/// the only thing that identifies it).
///
/// Also pins the three kinds that are not somebody talking - tool results,
/// synthetic echoes and meta notices - because rendering any of them as an
/// inbound bubble would put the CLI's own plumbing in the transcript.
#[test]
fn message_text_by_uuid_reads_a_bridge_message_from_the_transcript() {
    let _env = ENV_LOCK.lock().unwrap();
    let home = std::env::temp_dir().join("claude-eclipse-inbound-test-home");
    let root = r"C:\inbound";
    let dir = home.join(".claude").join("projects").join("C--inbound");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&dir).unwrap();

    fs::write(dir.join("sess1.jsonl"), concat!(
        r#"{"type":"user","uuid":"local-1","promptSource":"sdk","entrypoint":"claude-eclipse-ide","message":{"role":"user","content":"typed here"}}"#, "\n",
        r#"{"type":"assistant","uuid":"a-1","message":{"content":[{"type":"text","text":"hi"}]}}"#, "\n",
        r#"{"type":"user","uuid":"tool-1","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}"#, "\n",
        r#"{"type":"user","uuid":"synth-1","isSynthetic":true,"message":{"role":"user","content":"compact summary"}}"#, "\n",
        r#"{"type":"user","uuid":"meta-1","isMeta":true,"message":{"role":"user","content":"a meta notice"}}"#, "\n",
        r#"{"type":"user","uuid":"phone-1","message":{"role":"user","content":"sent from my phone"}}"#, "\n",
        r#"{"type":"user","uuid":"phone-2","message":{"role":"user","content":[{"type":"text","text":"two"},{"type":"text","text":"lines"}]}}"#, "\n",
    )).unwrap();

    set_home(&home);

    let phone = super::message_text_by_uuid(root, "sess1", "phone-1");
    let blocks = super::message_text_by_uuid(root, "sess1", "phone-2");
    let local = super::message_text_by_uuid(root, "sess1", "local-1");
    let tool = super::message_text_by_uuid(root, "sess1", "tool-1");
    let synth = super::message_text_by_uuid(root, "sess1", "synth-1");
    let meta = super::message_text_by_uuid(root, "sess1", "meta-1");
    let missing = super::message_text_by_uuid(root, "sess1", "nope");
    let no_session = super::message_text_by_uuid(root, "gone", "phone-1");
    // The uuid appears as a SUBSTRING of a longer one - the cheap
    // line.contains() reject must not be mistaken for a match.
    let prefix = super::message_text_by_uuid(root, "sess1", "phone");
    let escape = super::message_text_by_uuid(root, "../sess1", "phone-1");

    let _ = fs::remove_dir_all(&home);

    assert_eq!(phone.as_deref(), Some("sent from my phone"));
    assert_eq!(blocks.as_deref(), Some("two\nlines"), "text blocks are joined");
    assert_eq!(local.as_deref(), Some("typed here"),
               "a locally typed line is readable too - the caller decides which uuids to ask about");
    assert!(tool.is_none(), "tool results are the CLI's plumbing, not a message");
    assert!(synth.is_none(), "synthetic echoes are not somebody talking");
    assert!(meta.is_none(), "meta notices are not somebody talking");
    assert!(missing.is_none());
    assert!(no_session.is_none());
    assert!(prefix.is_none(), "a uuid prefix is not a uuid");
    assert!(escape.is_none(), "a session id may not climb out of the projects dir");
}

#[test]
fn list_sessions_title_precedence_matches_php_reader() {
    let _env = ENV_LOCK.lock().unwrap();
    let home = std::env::temp_dir().join("claude-eclipse-session-test-home");
    let root = r"C:\histtest";
    let dir = home.join(".claude").join("projects").join("C--histtest");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&dir).unwrap();

    fs::write(dir.join("aaaa1111.jsonl"), concat!(
        r#"{"type":"user","message":{"role":"user","content":"first user words here"},"timestamp":"2026-07-01T10:00:00.000Z"}"#, "\n",
        r#"{"type":"ai-title","aiTitle":"AI generated title","sessionId":"aaaa1111"}"#, "\n",
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"hi"}]},"timestamp":"2026-07-01T10:00:05.000Z"}"#, "\n",
        r#"{"type":"custom-title","customTitle":"Old rename","sessionId":"aaaa1111"}"#, "\n",
        r#"{"type":"custom-title","customTitle":"USER RENAMED TITLE","sessionId":"aaaa1111"}"#, "\n",
    )).unwrap();
    fs::write(dir.join("bbbb2222.jsonl"), concat!(
        r#"{"type":"ai-title","aiTitle":"title-only stub","sessionId":"bbbb2222"}"#, "\n",
        r#"{"type":"agent-name","agentName":"title-only stub","sessionId":"bbbb2222"}"#, "\n",
    )).unwrap();
    fs::write(dir.join("cccc3333.jsonl"), concat!(
        r#"{"type":"user","message":{"role":"user","content":"<ide_selection a=\"b\">junk</ide_selection>real question text"},"timestamp":"2026-07-02T09:00:00.000Z"}"#, "\n",
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"answer"}]},"timestamp":"2026-07-02T09:00:04.000Z"}"#, "\n",
    )).unwrap();

    set_home(&home);

    let json = super::list_sessions(root);
    let _ = fs::remove_dir_all(&home);

    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 3, "all three fixture sessions listed: {json}");
    let displays: Vec<&str> = arr.iter().map(|s| s["display"].as_str().unwrap()).collect();
    // bbbb2222 has no event timestamps → sort key is its (fresh) mtime → newest.
    assert_eq!(
        displays,
        vec!["title-only stub", "real question text", "USER RENAMED TITLE"],
        "titles + last-activity order must match the PHP reader"
    );
}

/// Covers: a match on the first session found + snippet returned, no match on a
/// second, an id NOT in the search list skipped even though its file would match
/// (proving the caller-supplied subset is honored, not re-derived), and matching
/// is case-insensitive.
#[test]
fn search_session_content_finds_first_match_and_skips_others() {
    let _env = ENV_LOCK.lock().unwrap();
    let home = std::env::temp_dir().join("claude-eclipse-session-search-home");
    let root = r"C:\searchtest";
    let dir = home.join(".claude").join("projects").join("C--searchtest");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&dir).unwrap();

    fs::write(dir.join("aaaa1111.jsonl"), concat!(
        r#"{"type":"user","message":{"role":"user","content":"talking about the quilt patch system"},"timestamp":"2026-07-01T10:00:00.000Z"}"#, "\n",
    )).unwrap();
    fs::write(dir.join("bbbb2222.jsonl"), concat!(
        r#"{"type":"user","message":{"role":"user","content":"nothing relevant here"},"timestamp":"2026-07-01T10:00:00.000Z"}"#, "\n",
    )).unwrap();
    // Would match too, but deliberately left out of the search list below.
    fs::write(dir.join("cccc3333.jsonl"), concat!(
        r#"{"type":"user","message":{"role":"user","content":"QUILT also appears here"},"timestamp":"2026-07-01T10:00:00.000Z"}"#, "\n",
    )).unwrap();

    set_home(&home);
    let ids = vec!["aaaa1111".to_string(), "bbbb2222".to_string()];
    let json = super::search_session_content(root, &ids, "quilt", false, 1);
    let _ = fs::remove_dir_all(&home);

    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1, "only aaaa1111 matches within the requested subset: {json}");
    assert_eq!(arr[0]["sessionId"].as_str().unwrap(), "aaaa1111");
    assert!(arr[0]["snippet"].as_str().unwrap().to_lowercase().contains("quilt"));
}

/// A query that only appears in an assistant turn matches with the full-conversation
/// scope but not with own_messages_only — proving the scope actually excludes
/// assistant text rather than just being ignored.
#[test]
fn search_session_content_own_messages_only_excludes_assistant_text() {
    let _env = ENV_LOCK.lock().unwrap();
    let home = std::env::temp_dir().join("claude-eclipse-session-search-own-home");
    let root = r"C:\searchownt";
    let dir = home.join(".claude").join("projects").join("C--searchownt");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&dir).unwrap();

    fs::write(dir.join("aaaa1111.jsonl"), concat!(
        r#"{"type":"user","message":{"role":"user","content":"please help me"},"timestamp":"2026-07-01T10:00:00.000Z"}"#, "\n",
        r#"{"type":"assistant","message":{"content":[{"type":"text","text":"the quilt patch system works like this"}]},"timestamp":"2026-07-01T10:00:05.000Z"}"#, "\n",
    )).unwrap();

    set_home(&home);
    let ids = vec!["aaaa1111".to_string()];
    let full = super::search_session_content(root, &ids, "quilt", false, 1);
    let own_only = super::search_session_content(root, &ids, "quilt", true, 1);
    let _ = fs::remove_dir_all(&home);

    let full_v: serde_json::Value = serde_json::from_str(&full).unwrap();
    assert_eq!(full_v.as_array().unwrap().len(), 1, "full-conversation scope finds the assistant match: {full}");
    let own_v: serde_json::Value = serde_json::from_str(&own_only).unwrap();
    assert_eq!(own_v.as_array().unwrap().len(), 0, "own_messages_only must not match assistant text: {own_only}");
}

/// Regression test for a real crash: certain characters (German ẞ, Turkish İ, …)
/// change UTF-8 byte length when lowercased, so a match position found via
/// text.to_lowercase().find() does not correspond to the same byte offset in the
/// ORIGINAL text — slicing the original at that offset can land mid-character and
/// panic ("byte index N is not a char boundary"). Across the JNI boundary that
/// panic is undefined behavior (an unwind into a JVM-owned native frame), which
/// crashed a live user's whole Eclipse process with no JVM crash dump and nothing
/// in dmesg — exactly the kind of failure that looks like it isn't ours. Confirmed
/// via a standalone repro before this test existed: "ẞẞxquilt" searching "quilt"
/// panicked at the exact line this function now guards.
#[test]
fn search_session_content_snippet_survives_case_folding_byte_length_change() {
    let _env = ENV_LOCK.lock().unwrap();
    let home = std::env::temp_dir().join("claude-eclipse-session-search-unicode-home");
    let root = r"C:\searchunicode";
    let dir = home.join(".claude").join("projects").join("C--searchunicode");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&dir).unwrap();

    // ẞ (U+1E9E, LATIN CAPITAL LETTER SHARP S) lowercases to "ß" — same character
    // count but a different UTF-8 byte length, which is what desynchronizes the
    // lowercased string's match offset from the original string's byte layout.
    fs::write(dir.join("aaaa1111.jsonl"), concat!(
        r#"{"type":"user","message":{"role":"user","content":"ẞẞxquilt talk"},"timestamp":"2026-07-01T10:00:00.000Z"}"#, "\n",
    )).unwrap();

    set_home(&home);
    let ids = vec!["aaaa1111".to_string()];
    // Must not panic — that's the entire point of this test.
    let json = super::search_session_content(root, &ids, "quilt", false, 1);
    let _ = fs::remove_dir_all(&home);

    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let arr = v.as_array().unwrap();
    assert_eq!(arr.len(), 1, "the match is still found despite the preceding multibyte characters: {json}");
    assert!(arr[0]["snippet"].as_str().unwrap().to_lowercase().contains("quilt"));
}

/// Verified against the reference reader on 2026-07-10: the fixture below was
/// fed to it and the expected JSON here is its captured output, byte-for-byte
/// (compared as Values since key order differs). Covers: raw user content
/// (ide_selection kept), partial assistant skipped, thinking/text/tool_use
/// items with per-turn model, askUserQuestion answer surfacing with "The user
/// answered:" prefix stripping, empty text blocks dropped, and non-ask
/// tool_results ignored.
#[test]
fn load_session_render_items_match_php_reader() {
    let _env = ENV_LOCK.lock().unwrap();
    let home = std::env::temp_dir().join("claude-eclipse-session-load-home");
    let root = r"C:\phpfixws";
    let dir = home.join(".claude").join("projects").join("C--phpfixws");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&dir).unwrap();

    fs::write(dir.join("sess1.jsonl"), concat!(
        r#"{"type":"user","message":{"role":"user","content":"<ide_selection a=\"b\">sel junk</ide_selection>please fix the bug"},"timestamp":"2026-07-01T10:00:00.000Z"}"#, "\n",
        r#"{"type":"assistant","partial":true,"message":{"model":"claude-fable-5","content":[{"type":"text","text":"par"}]},"timestamp":"2026-07-01T10:00:01.000Z"}"#, "\n",
        r#"{"type":"assistant","message":{"model":"claude-fable-5","content":[{"type":"thinking","thinking":"hmm secret"},{"type":"text","text":"Here is my answer"},{"type":"tool_use","id":"toolu_01","name":"mcp__eclipse__askUserQuestion","input":{"q":"Which color?"}}]},"timestamp":"2026-07-01T10:00:05.000Z"}"#, "\n",
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_01","content":[{"type":"text","text":"  The user answered: Blue"}]}]},"timestamp":"2026-07-01T10:00:09.000Z"}"#, "\n",
        r#"{"type":"assistant","message":{"model":"claude-opus-4-8","content":[{"type":"tool_use","id":"toolu_02","name":"Edit","input":{"file_path":"C:\\x.java","old_string":"a","new_string":"b"}},{"type":"text","text":""}]},"timestamp":"2026-07-01T10:00:12.000Z"}"#, "\n",
        r#"{"type":"custom-title","customTitle":"My renamed session","sessionId":"sess1"}"#, "\n",
    )).unwrap();
    fs::write(dir.join("sess2.jsonl"), concat!(
        r#"{"type":"user","message":{"role":"user","content":"<command-name>/clear</command-name><command-message>clear</command-message><command-args>now</command-args>"},"timestamp":"2026-07-03T08:00:00.000Z"}"#, "\n",
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_99","content":"unrelated result"}]},"timestamp":"2026-07-03T08:00:02.000Z"}"#, "\n",
    )).unwrap();

    set_home(&home);

    let loaded1 = super::load_session_history(root, "sess1");
    let loaded2 = super::load_session_history(root, "sess2");
    let listed = super::list_sessions(root);
    let _ = fs::remove_dir_all(&home);

    let got1: serde_json::Value = serde_json::from_str(&loaded1).unwrap();
    // toolu_01 (askUserQuestion) has a non-error tool_result → status "done";
    // toolu_02 (Edit) has no tool_result in the fixture → status "interrupted".
    let want1: serde_json::Value = serde_json::from_str(r#"[
            {"t":"user","content":"<ide_selection a=\"b\">sel junk</ide_selection>please fix the bug","ts":"2026-07-01T10:00:00.000Z"},
            {"t":"thinking","model":"claude-fable-5","text":"hmm secret"},
            {"t":"text","text":"Here is my answer","model":"claude-fable-5"},
            {"t":"tool","name":"mcp__eclipse__askUserQuestion","input":{"q":"Which color?"},"model":"claude-fable-5","id":"toolu_01","status":"done","resultText":"  The user answered: Blue"},
            {"t":"answered","text":"Blue"},
            {"t":"tool","name":"Edit","input":{"file_path":"C:\\x.java","old_string":"a","new_string":"b"},"model":"claude-opus-4-8","id":"toolu_02","status":"interrupted"}
        ]"#).unwrap();
    assert_eq!(got1, want1, "sess1 render items must match the reference reader");

    let got2: serde_json::Value = serde_json::from_str(&loaded2).unwrap();
    let want2: serde_json::Value = serde_json::from_str(r#"[
            {"t":"user","content":"<command-name>/clear</command-name><command-message>clear</command-message><command-args>now</command-args>","ts":"2026-07-03T08:00:00.000Z"}
        ]"#).unwrap();
    assert_eq!(got2, want2, "sess2: raw user kept, non-ask tool_result ignored");

    // List titles: sess2's fallback title is the fully-unwrapped command text.
    let lv: serde_json::Value = serde_json::from_str(&listed).unwrap();
    let sess2 = lv.as_array().unwrap().iter()
        .find(|s| s["sessionId"] == "sess2").expect("sess2 listed");
    assert_eq!(sess2["display"], "/clear", "command wrappers stripped from title");
    assert_eq!(sess2["timestamp"], "2026-07-03T08:00:02.000Z");
}

/// Background-task notifications are injected into the transcript as ordinary
/// user lines. Reopening a conversation showed them as the user's own messages —
/// several raw <task-notification> XML blocks in a row where the conversation
/// should be.
#[test]
fn load_session_hides_background_task_notifications() {
    let _env = ENV_LOCK.lock().unwrap();
    let home = std::env::temp_dir().join("claude-eclipse-session-tasknote-home");
    let root = r"C:\tasknotews";
    let dir = home.join(".claude").join("projects").join("C--tasknotews");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&dir).unwrap();
    // Both shapes the CLI writes, copied from a real transcript.
    fs::write(dir.join("sessn.jsonl"), concat!(
        r#"{"type":"user","message":{"role":"user","content":"hi"},"timestamp":"2026-09-16T17:19:00.000Z"}"#, "\n",
        r#"{"type":"user","message":{"role":"user","content":"<task-notification>\n<task-id>b0mc0q2r4</task-id>\n<summary>Monitor event</summary>\n</task-notification>"},"timestamp":"2026-09-16T17:19:37.000Z"}"#, "\n",
        r#"{"type":"user","message":{"role":"user","content":"<task-notification>\n<status>completed</status>\n</task-notification>"},"commandMode":"task-notification","timestamp":"2026-09-16T17:19:41.000Z"}"#, "\n",
        r#"{"type":"assistant","message":{"model":"claude-opus-5","content":[{"type":"text","text":"noted."}]},"timestamp":"2026-09-16T17:19:45.000Z"}"#, "\n",
    )).unwrap();

    set_home(&home);
    let loaded = super::load_session_history(root, "sessn");
    let _ = fs::remove_dir_all(&home);

    let got: serde_json::Value = serde_json::from_str(&loaded).unwrap();
    let want: serde_json::Value = serde_json::from_str(r#"[
            {"t":"user","content":"hi","ts":"2026-09-16T17:19:00.000Z"},
            {"t":"text","text":"noted.","model":"claude-opus-5"}
        ]"#).unwrap();
    assert_eq!(got, want, "a notification nobody typed must not come back as a message");
}


#[test]
fn load_session_hides_the_browser_disconnected_notice() {
    let _env = ENV_LOCK.lock().unwrap();
    let home = std::env::temp_dir().join("claude-eclipse-session-notice-home");
    let root = r"C:\noticews";
    let dir = home.join(".claude").join("projects").join("C--noticews");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("sessn.jsonl"), concat!(
        r#"{"type":"user","message":{"role":"user","content":"hi"},"timestamp":"2026-09-15T08:40:00.000Z"}"#, "\n",
        r#"{"type":"user","message":{"role":"user","content":"[MESSAGE FROM NON-USER SOURCE - NOT USER INPUT]\n[Browser disconnected: The browser connection has been closed. Browser tools are no longer available.]"},"isMeta":true,"timestamp":"2026-09-15T08:40:23.282Z"}"#, "\n",
        r#"{"type":"assistant","message":{"model":"claude-opus-5","content":[{"type":"text","text":"Chrome has disconnected."}]},"timestamp":"2026-09-15T08:40:25.000Z"}"#, "\n",
    )).unwrap();

    set_home(&home);
    let loaded = super::load_session_history(root, "sessn");
    let _ = fs::remove_dir_all(&home);

    let got: serde_json::Value = serde_json::from_str(&loaded).unwrap();
    let want: serde_json::Value = serde_json::from_str(r#"[
            {"t":"user","content":"hi","ts":"2026-09-15T08:40:00.000Z"},
            {"t":"text","text":"Chrome has disconnected.","model":"claude-opus-5"}
        ]"#).unwrap();
    assert_eq!(got, want);
}

/// A compacted session reloads as a "Compacted chat" marker + expandable
/// summary: the compact_boundary system line becomes a t:"compact" item
/// (camelCase compactMetadata → trigger/preTokens/postTokens) and the
/// isCompactSummary user line becomes t:"compact_summary" — never a user
/// bubble. Fixture shapes captured from a real CLI 2.1.177 /compact run.
#[test]
fn load_session_surfaces_compact_boundary_and_summary() {
    let _env = ENV_LOCK.lock().unwrap();
    let home = std::env::temp_dir().join("claude-eclipse-session-compact-home");
    let root = r"C:\compactws";
    let dir = home.join(".claude").join("projects").join("C--compactws");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&dir).unwrap();

    fs::write(dir.join("sessc.jsonl"), concat!(
        r#"{"type":"user","message":{"role":"user","content":"tell me things"},"timestamp":"2026-07-27T02:00:00.000Z"}"#, "\n",
        r#"{"type":"assistant","message":{"model":"claude-haiku-4-5-20251001","content":[{"type":"text","text":"things"}]},"timestamp":"2026-07-27T02:00:05.000Z"}"#, "\n",
        r#"{"type":"system","subtype":"compact_boundary","content":"Conversation compacted","isMeta":false,"compactMetadata":{"trigger":"manual","preTokens":23670,"durationMs":10550,"postTokens":1682},"timestamp":"2026-07-27T02:37:08.042Z"}"#, "\n",
        r#"{"type":"user","isCompactSummary":true,"isVisibleInTranscriptOnly":true,"message":{"role":"user","content":"This session is being continued from a previous conversation. Summary: things were told."},"timestamp":"2026-07-27T02:37:08.100Z"}"#, "\n",
        r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"<local-command-caveat>Caveat: ...</local-command-caveat>"},"timestamp":"2026-07-27T02:37:08.120Z"}"#, "\n",
        r#"{"type":"user","message":{"role":"user","content":"<command-name>/compact</command-name>"},"timestamp":"2026-07-27T02:37:08.130Z"}"#, "\n",
    )).unwrap();

    set_home(&home);
    let loaded = super::load_session_history(root, "sessc");
    let _ = fs::remove_dir_all(&home);

    let got: serde_json::Value = serde_json::from_str(&loaded).unwrap();
    let want: serde_json::Value = serde_json::from_str(r#"[
            {"t":"user","content":"tell me things","ts":"2026-07-27T02:00:00.000Z"},
            {"t":"text","text":"things","model":"claude-haiku-4-5-20251001"},
            {"t":"compact","trigger":"manual","preTokens":23670,"postTokens":1682},
            {"t":"compact_summary","text":"This session is being continued from a previous conversation. Summary: things were told."},
            {"t":"user","content":"<local-command-caveat>Caveat: ...</local-command-caveat>","ts":"2026-07-27T02:37:08.120Z"},
            {"t":"user","content":"<command-name>/compact</command-name>","ts":"2026-07-27T02:37:08.130Z"}
        ]"#).unwrap();
    assert_eq!(got, want, "compacted session render items");
}

/// A message sent with pasted images is stored as content BLOCKS, not a
/// string — it must come back as one user item carrying its text and the
/// images' base64 (so the chips redraw), the session must be titled from
/// that text, and tool_result-only block lines must still add no bubble.
/// Shapes captured from a real CLI transcript (note the CLI re-encodes a
/// pasted PNG to image/jpeg).
#[test]
fn load_session_restores_pasted_images() {
    let _env = ENV_LOCK.lock().unwrap();
    let home = std::env::temp_dir().join("claude-eclipse-session-images-home");
    let root = r"C:\imgws";
    let dir = home.join(".claude").join("projects").join("C--imgws");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&dir).unwrap();

    fs::write(dir.join("sessi.jsonl"), concat!(
        r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"<ide_context openFile=\"C:\\a\\B.java\" />\n\nwhat is this"},{"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":"QUJD"}}]},"timestamp":"2026-07-30T01:00:00.000Z"}"#, "\n",
        r#"{"type":"assistant","message":{"model":"claude-opus-4-8","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"a.txt"}}]},"timestamp":"2026-07-30T01:00:03.000Z"}"#, "\n",
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"file contents"}]},"timestamp":"2026-07-30T01:00:04.000Z"}"#, "\n",
        r#"{"type":"assistant","message":{"model":"claude-opus-4-8","content":[{"type":"text","text":"a screenshot"}]},"timestamp":"2026-07-30T01:00:06.000Z"}"#, "\n",
    )).unwrap();

    set_home(&home);
    let loaded = super::load_session_history(root, "sessi");
    let listed = super::list_sessions(root);
    let _ = fs::remove_dir_all(&home);

    let got: serde_json::Value = serde_json::from_str(&loaded).unwrap();
    let want: serde_json::Value = serde_json::from_str(r#"[
            {"t":"user","content":"<ide_context openFile=\"C:\\a\\B.java\" />\n\nwhat is this",
             "images":[{"media_type":"image/jpeg","data":"QUJD"}],"ts":"2026-07-30T01:00:00.000Z"},
            {"t":"tool","name":"Read","input":{"file_path":"a.txt"},"status":"done","model":"claude-opus-4-8","id":"t1","resultText":"file contents"},
            {"t":"text","text":"a screenshot","model":"claude-opus-4-8"}
        ]"#).unwrap();
    assert_eq!(got, want, "pasted-image session render items");

    // The list title comes from the text block, with the IDE preamble stripped.
    let sessions: serde_json::Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(sessions[0]["display"], serde_json::json!("what is this"));
}

/// An uploaded file comes back as a document its chip can redraw and open, and
/// the browser blocks a `@browser` message carries stay out of the bubble and
/// the title.
#[test]
fn load_session_restores_documents_and_hides_browser_blocks() {
    let _env = ENV_LOCK.lock().unwrap();
    let home = std::env::temp_dir().join("claude-eclipse-session-docs-home");
    let root = r"C:\docws";
    let dir = home.join(".claude").join("projects").join("C--docws");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&dir).unwrap();

    fs::write(dir.join("sessd.jsonl"), concat!(
        r#"{"type":"user","uuid":"u-doc","message":{"role":"user","content":[{"type":"text","text":"<browser_instruction># x</browser_instruction>"},{"type":"text","text":"read this @browser:new_tab"},{"type":"document","source":{"type":"text","media_type":"text/plain","data":"hello"},"title":"notes.txt"},{"type":"text","text":"<browser tabGroupId=\"1\" tabId=\"2\"></browser>"}]},"timestamp":"2026-09-14T01:00:00.000Z"}"#, "\n",
    )).unwrap();

    set_home(&home);
    let loaded = super::load_session_history(root, "sessd");
    let listed = super::list_sessions(root);
    // The chip's click fetches the contents the render item deliberately left behind.
    let file = super::session_document_file(root, "sessd", "u-doc", 0);
    let second = super::session_document_file(root, "sessd", "u-doc", 1);
    let unknown = super::session_document_file(root, "sessd", "nope", 0);
    let _ = fs::remove_dir_all(&home);

    let got: serde_json::Value = serde_json::from_str(&loaded).unwrap();
    let want: serde_json::Value = serde_json::from_str(r#"[
            {"t":"user","content":"read this @browser:new_tab","id":"u-doc",
             "documents":[{"title":"notes.txt","media_type":"text/plain","encoding":"text","index":0}],
             "ts":"2026-09-14T01:00:00.000Z"}
        ]"#).unwrap();
    assert_eq!(got, want, "document session render items");

    assert!(file.ends_with("notes.txt"), "{file}");
    assert_eq!(fs::read_to_string(&file).unwrap(), "hello");
    assert!(second.is_empty(), "no second document");
    assert!(unknown.is_empty(), "unknown message");
    let _ = fs::remove_file(&file);

    let sessions: serde_json::Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(sessions[0]["display"], serde_json::json!("read this @browser:new_tab"));
}

/// A file uploaded as a path comes back as its chip; a message that merely quotes
/// the block keeps it as the user's own words.
#[test]
fn load_session_restores_path_attachments() {
    let _env = ENV_LOCK.lock().unwrap();
    let home = std::env::temp_dir().join("claude-eclipse-session-paths-home");
    let root = r"C:\pathws";
    let dir = home.join(".claude").join("projects").join("C--pathws");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&dir).unwrap();

    fs::write(dir.join("sessp.jsonl"), concat!(
        r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"what is in here"},{"type":"text","text":"<attached_file path=\"C:\\x\\big.zip\" />"}]},"timestamp":"2026-09-15T01:00:00.000Z"}"#, "\n",
        r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"I wrote <attached_file path=\"C:\\x\\big.zip\" /> myself"}]},"timestamp":"2026-09-15T01:01:00.000Z"}"#, "\n",
    )).unwrap();

    set_home(&home);
    let loaded = super::load_session_history(root, "sessp");
    let listed = super::list_sessions(root);
    let _ = fs::remove_dir_all(&home);

    let got: serde_json::Value = serde_json::from_str(&loaded).unwrap();
    let want: serde_json::Value = serde_json::from_str(r#"[
            {"t":"user","content":"what is in here",
             "documents":[{"title":"big.zip","encoding":"path","path":"C:\\x\\big.zip"}],
             "ts":"2026-09-15T01:00:00.000Z"},
            {"t":"user","content":"I wrote <attached_file path=\"C:\\x\\big.zip\" /> myself",
             "ts":"2026-09-15T01:01:00.000Z"}
        ]"#).unwrap();
    assert_eq!(got, want, "path attachment session render items");

    let sessions: serde_json::Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(sessions[0]["display"], serde_json::json!("what is in here"));
}

/// Tool dots are reconstructed from the transcript so a reloaded conversation
/// keeps its green/red: a non-error tool_result ⇒ "done", an is_error result ⇒
/// "interrupted", and a tool with no result at all ⇒ "interrupted".
/// A backend error the CLI stores as a SYNTHETIC assistant message flagged
/// isApiErrorMessage must come back as t:"error" (the muted "⚠ …" line the
/// live run showed via onError), never as t:"text" — otherwise reopening a
/// past session reads the outage as something the model said. Both fixture
/// lines are real shapes captured from local transcripts (a 429 session-limit
/// hit and a 529 overload); ordinary assistant text alongside them must stay
/// t:"text". If the CLI ever stops setting the flag this test breaks instead
/// of the errors silently turning back into paragraphs.
#[test]
fn load_session_surfaces_api_errors_as_muted_lines() {
    let _env = ENV_LOCK.lock().unwrap();
    let home = std::env::temp_dir().join("claude-eclipse-session-apierr-home");
    let root = r"C:\errws";
    let dir = home.join(".claude").join("projects").join("C--errws");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&dir).unwrap();

    fs::write(dir.join("sesse.jsonl"), concat!(
        r#"{"type":"user","message":{"role":"user","content":"go"},"timestamp":"2026-08-26T01:00:00.000Z"}"#, "
",
        r#"{"type":"assistant","message":{"model":"claude-opus-4-8","content":[{"type":"text","text":"working on it"}]},"timestamp":"2026-08-26T01:00:02.000Z"}"#, "
",
        r#"{"type":"assistant","isApiErrorMessage":true,"apiErrorStatus":429,"error":"rate_limit","message":{"model":"<synthetic>","role":"assistant","content":[{"type":"text","text":"You've hit your session limit · resets 2:10am (Asia/Irkutsk)"}]},"timestamp":"2026-08-26T01:00:03.000Z"}"#, "
",
        r#"{"type":"assistant","isApiErrorMessage":true,"apiErrorStatus":529,"error":"overloaded","message":{"model":"<synthetic>","role":"assistant","content":[{"type":"text","text":"API Error: 529 Overloaded. This is a server-side issue, usually temporary — try again in a moment. If it persists, check https://status.claude.com."}]},"timestamp":"2026-08-26T01:00:04.000Z"}"#, "
",
    )).unwrap();

    set_home(&home);
    let loaded = super::load_session_history(root, "sesse");
    let _ = fs::remove_dir_all(&home);

    let got: serde_json::Value = serde_json::from_str(&loaded).unwrap();
    let want: serde_json::Value = serde_json::from_str(r#"[
            {"t":"user","content":"go","ts":"2026-08-26T01:00:00.000Z"},
            {"t":"text","text":"working on it","model":"claude-opus-4-8"},
            {"t":"error","text":"You've hit your session limit · resets 2:10am (Asia/Irkutsk)"},
            {"t":"error","text":"API Error: 529 Overloaded. This is a server-side issue, usually temporary — try again in a moment. If it persists, check https://status.claude.com."}
        ]"#).unwrap();
    assert_eq!(got, want, "api error render items");
}

/// The one-line reason shown under a failed tool. Every input below is a real
/// shape from local transcripts (111 `is_error` results were surveyed).
#[test]
fn tool_error_summary_condenses_real_failures() {
    use super::tool_error_summary as sum;

    // Three quarters of genuine failures lead with a bare exit code, which on
    // its own says nothing — the next real line is what broke.
    assert_eq!(
        sum("Exit code 1\nTraceback (most recent call last):\r\n  File \"<string>\", line 4"),
        Some("Exit code 1 · Traceback (most recent call last):".into())
    );
    // The status is kept, not dropped: 143 (timeout) ≠ 1 (ordinary failure).
    assert_eq!(
        sum("Exit code 143\nCommand timed out after 2m 0s"),
        Some("Exit code 143 · Command timed out after 2m 0s".into())
    );
    // An exit code with nothing after it still beats showing nothing.
    assert_eq!(sum("Exit code 2"), Some("Exit code 2".into()));
    // "Exit code" that is NOT bare is a message in its own right — left alone.
    assert_eq!(sum("Exit code 1 was returned"), Some("Exit code 1 was returned".into()));

    // The CLI's own error envelope is unwrapped so the message reads plainly.
    assert_eq!(
        sum("<tool_use_error>File has not been read yet. Read it first before writing to it.</tool_use_error>"),
        Some("File has not been read yet. Read it first before writing to it.".into())
    );

    // A single-line failure passes through untouched.
    assert_eq!(
        sum("File does not exist. Note: your current working directory is C:\\ws"),
        Some("File does not exist. Note: your current working directory is C:\\ws".into())
    );

    // The user's own decisions are NOT failures: the GUI already shows those
    // through its decision cards, so the tool row stays quiet (red dot only).
    assert_eq!(sum("The user doesn't want to proceed with this tool use. The tool use was rejected"), None);
    assert_eq!(sum("The user declined this action in Eclipse."), None);
    assert_eq!(sum("The user dismissed the prompt."), None);
    assert_eq!(sum("[User typed]: okay do it differently"), None);

    // Nothing to say → no line at all, rather than an empty one.
    assert_eq!(sum(""), None);
    assert_eq!(sum("   \n  \n"), None);
}

/// Long results are cut to one line's worth. The cut counts CHARACTERS, not
/// bytes — these carry Windows paths and prose, and slicing mid-codepoint
/// would panic the loader on a conversation that merely contains a failure.
#[test]
fn tool_error_summary_truncates_on_char_boundaries() {
    let long = "é".repeat(400);
    let got = super::tool_error_summary(&long).unwrap();
    assert_eq!(got.chars().count(), 161, "160 chars plus the ellipsis");
    assert!(got.ends_with('…'));

    let ascii = "x".repeat(400);
    let got = super::tool_error_summary(&ascii).unwrap();
    assert!(got.starts_with("xxxx") && got.ends_with('…'));
}

/// A failed tool must carry WHY it failed onto its render item, so a reopened
/// conversation reads the same as it did live. A tool the user declined gets
/// the red dot but no text; a successful one carries its full output as
/// resultText instead (a DIFFERENT field — see result_text vs
/// result_success_text above — so makeToolLine can render it as an OUT box
/// rather than the muted one-line error note).
#[test]
fn load_session_attaches_error_text_to_failed_tools() {
    let _env = ENV_LOCK.lock().unwrap();
    let home = std::env::temp_dir().join("claude-eclipse-session-toolerr-home");
    let root = r"C:\toolerrws";
    let dir = home.join(".claude").join("projects").join("C--toolerrws");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&dir).unwrap();

    fs::write(dir.join("sesst.jsonl"), concat!(
        r#"{"type":"user","message":{"role":"user","content":"go"},"timestamp":"2026-09-04T01:00:00.000Z"}"#, "\n",
        r#"{"type":"assistant","message":{"model":"claude-opus-4-8","content":[{"type":"tool_use","id":"toolu_a","name":"Read","input":{"file_path":"C:\\nope.java"}}]},"timestamp":"2026-09-04T01:00:01.000Z"}"#, "\n",
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_a","is_error":true,"content":"File does not exist. Note: your current working directory is C:\\ws"}]},"timestamp":"2026-09-04T01:00:02.000Z"}"#, "\n",
        r#"{"type":"assistant","message":{"model":"claude-opus-4-8","content":[{"type":"tool_use","id":"toolu_b","name":"Edit","input":{"file_path":"C:\\x.java"}}]},"timestamp":"2026-09-04T01:00:03.000Z"}"#, "\n",
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_b","is_error":true,"content":"The user doesn't want to proceed with this tool use. The tool use was rejected"}]},"timestamp":"2026-09-04T01:00:04.000Z"}"#, "\n",
        r#"{"type":"assistant","message":{"model":"claude-opus-4-8","content":[{"type":"tool_use","id":"toolu_c","name":"Read","input":{"file_path":"C:\\ok.java"}}]},"timestamp":"2026-09-04T01:00:05.000Z"}"#, "\n",
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_c","content":"contents"}]},"timestamp":"2026-09-04T01:00:06.000Z"}"#, "\n",
    )).unwrap();

    set_home(&home);
    let loaded = super::load_session_history(root, "sesst");
    let _ = fs::remove_dir_all(&home);

    let got: serde_json::Value = serde_json::from_str(&loaded).unwrap();
    let want: serde_json::Value = serde_json::from_str(r#"[
            {"t":"user","content":"go","ts":"2026-09-04T01:00:00.000Z"},
            {"t":"tool","name":"Read","input":{"file_path":"C:\\nope.java"},"model":"claude-opus-4-8","id":"toolu_a","status":"interrupted","errorText":"File does not exist. Note: your current working directory is C:\\ws"},
            {"t":"tool","name":"Edit","input":{"file_path":"C:\\x.java"},"model":"claude-opus-4-8","id":"toolu_b","status":"interrupted"},
            {"t":"tool","name":"Read","input":{"file_path":"C:\\ok.java"},"model":"claude-opus-4-8","id":"toolu_c","status":"done","resultText":"contents"}
        ]"#).unwrap();
    assert_eq!(got, want, "failed tools carry their reason; declined ones stay quiet; successful ones carry their output");
}

/// A subagent's own nested transcript lives in its own dedicated file, never
/// multiplexed into its parent's — confirmed against a real on-disk conversation (no
/// parent_tool_use_id/parentToolUseId anywhere in the parent file; a background Agent
/// call's own steps only showed up under
/// `<session_id>/subagents/agent-<id>.jsonl`, matched to the top-level tool_use via
/// that file's `agent-<id>.meta.json` sidecar's `toolUseId` field). This fixture
/// reproduces that exact layout.
#[test]
fn load_session_reads_agent_log_from_its_own_subagent_file() {
    let _env = ENV_LOCK.lock().unwrap();
    let home = std::env::temp_dir().join("claude-eclipse-session-agentlog-home");
    let root = r"C:\agentws";
    let dir = home.join(".claude").join("projects").join("C--agentws");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&dir).unwrap();

    // The parent conversation: just the Agent tool_use and an early ack tool_result
    // (a background call's own real work never appears here — that is the whole
    // point of this fixture).
    fs::write(dir.join("sessa.jsonl"), concat!(
        r#"{"type":"assistant","message":{"model":"claude-sonnet-5","content":[{"type":"tool_use","id":"toolu_top","name":"Agent","input":{"description":"Count to 3","prompt":"count to 3","subagent_type":"Explore","run_in_background":true}}]},"timestamp":"2026-09-16T13:54:40.400Z"}"#, "\n",
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_top","content":"Agent running in the background"}]},"timestamp":"2026-09-16T13:54:40.410Z"}"#, "\n",
    )).unwrap();

    // The subagent's own dedicated conversation, in its own subdirectory.
    let sub_dir = dir.join("sessa").join("subagents");
    fs::create_dir_all(&sub_dir).unwrap();
    fs::write(sub_dir.join("agent-abc123.meta.json"),
        r#"{"agentType":"Explore","description":"Count to 3","toolUseId":"toolu_top","spawnDepth":1,"requestShape":"background"}"#,
    ).unwrap();
    fs::write(sub_dir.join("agent-abc123.jsonl"), concat!(
        r#"{"type":"user","isSidechain":true,"agentId":"abc123","message":{"role":"user","content":"count to 3"},"timestamp":"2026-09-16T13:54:40.450Z"}"#, "\n",
        r#"{"type":"assistant","isSidechain":true,"agentId":"abc123","message":{"model":"claude-sonnet-5","usage":{"input_tokens":100,"output_tokens":50},"content":[{"type":"tool_use","id":"toolu_sub1","name":"Bash","input":{"command":"echo 1 2 3"}}]},"timestamp":"2026-09-16T13:54:40.460Z"}"#, "\n",
        r#"{"type":"user","isSidechain":true,"agentId":"abc123","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_sub1","content":"1 2 3"}]},"timestamp":"2026-09-16T13:54:40.470Z"}"#, "\n",
        r#"{"type":"assistant","isSidechain":true,"agentId":"abc123","message":{"model":"claude-sonnet-5","usage":{"input_tokens":80,"output_tokens":40},"content":[{"type":"text","text":"Counted to 3."}]},"timestamp":"2026-09-16T13:54:40.480Z"}"#, "\n",
    )).unwrap();

    set_home(&home);
    let loaded = super::load_session_history(root, "sessa");
    let _ = fs::remove_dir_all(&home);

    let got: serde_json::Value = serde_json::from_str(&loaded).unwrap();
    let tool_item = got.as_array().unwrap().iter()
        .find(|it| it["t"] == "tool" && it["id"] == "toolu_top")
        .expect("the top-level Agent tool item");
    let log = &tool_item["agentLog"];
    assert_eq!(log["tokens"], 270, "sums usage across every completed message in the subagent's own file (100+50+80+40)");
    assert_eq!(log["model"], "claude-sonnet-5");
    assert_eq!(log["startedAt"], "2026-09-16T13:54:40.450Z", "the subagent file's own FIRST timestamp, not the parent's");
    assert_eq!(log["endedAt"], "2026-09-16T13:54:40.480Z", "the subagent file's own LAST timestamp");
    // The initial plain "user" message (the subagent's own starting prompt) produces
    // no item at all — only its own text/thinking/tool_use content does — so this is
    // exactly [tool_use Bash (stamped done+resultText), text "Counted to 3."].
    let items = log["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["kind"], "tool");
    assert_eq!(items[0]["name"], "Bash");
    assert_eq!(items[0]["status"], "done");
    assert_eq!(items[0]["resultText"], "1 2 3");
    assert_eq!(items[1]["kind"], "text");
    assert_eq!(items[1]["text"], "Counted to 3.");
}

#[test]
fn load_session_reconstructs_tool_status() {
    let _env = ENV_LOCK.lock().unwrap();
    let home = std::env::temp_dir().join("claude-eclipse-session-status-home");
    let root = r"C:\statusws";
    let dir = home.join(".claude").join("projects").join("C--statusws");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&dir).unwrap();

    fs::write(dir.join("s.jsonl"), concat!(
        // A finished Read (has a normal result), an interrupted Bash (is_error
        // result — the "user doesn't want to proceed" case), and a trailing Edit
        // with no result at all (turn cut off).
        r#"{"type":"assistant","message":{"model":"claude-opus-4-8","content":[{"type":"tool_use","id":"t1","name":"Read","input":{"file_path":"a.txt"}},{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"gh pr list"}},{"type":"tool_use","id":"t3","name":"Edit","input":{"file_path":"b.txt"}}]},"timestamp":"2026-07-15T10:00:00.000Z"}"#, "\n",
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"file contents"},{"type":"tool_result","tool_use_id":"t2","is_error":true,"content":"The user doesn't want to proceed with this tool use."}]},"timestamp":"2026-07-15T10:00:03.000Z"}"#, "\n",
    )).unwrap();

    set_home(&home);
    let loaded = super::load_session_history(root, "s");
    let _ = fs::remove_dir_all(&home);

    let got: serde_json::Value = serde_json::from_str(&loaded).unwrap();
    let tools: Vec<(&str, &str)> = got.as_array().unwrap().iter()
        .filter(|it| it["t"] == "tool")
        .map(|it| (it["name"].as_str().unwrap(), it["status"].as_str().unwrap_or("MISSING")))
        .collect();
    assert_eq!(
        tools,
        vec![("Read", "done"), ("Bash", "interrupted"), ("Edit", "interrupted")],
        "tool dot status reconstructed from tool_result presence/is_error"
    );
}

#[test]
fn delete_session_guards_and_removes() {
    let _env = ENV_LOCK.lock().unwrap();
    let home = std::env::temp_dir().join("claude-eclipse-session-del-home");
    let root = r"C:\deltest";
    let dir = home.join(".claude").join("projects").join("C--deltest");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("victim.jsonl"), "{}\n").unwrap();

    set_home(&home);

    assert!(!super::delete_session(root, ""), "empty id rejected");
    assert!(!super::delete_session(root, "../victim"), "traversal rejected");
    assert!(!super::delete_session(root, "a\\b"), "separator rejected");
    assert!(!super::delete_session(root, "missing"), "absent file is false");
    assert!(super::delete_session(root, "victim"), "existing file deleted");
    assert!(!dir.join("victim.jsonl").exists());

    let _ = fs::remove_dir_all(&home);
}

/// Fixture shaped like a real transcript (verified against a live one on
/// 2026-07-30): a parentUuid chain, an `attachment` child hanging off the
/// user line, `file-history-snapshot` lines keyed by messageId, and the two
/// UNCHAINED prompt carriers — `queue-operation.content` (wrapper still
/// attached) and `last-prompt.lastPrompt` (several copies per message).
/// Also plants the two legitimate echoes that must NOT block a delete: an
/// assistant line quoting the prompt and a tool_result line containing it.
fn msg_fixture(extra: &str) -> String {
    [
        r#"{"type":"queue-operation","operation":"enqueue","content":"<ide_context openFile=\"C:\\a.java\" />\n\nfirst question","sessionId":"sess1"}"#,
        r#"{"type":"user","uuid":"u1","parentUuid":null,"message":{"role":"user","content":"first question"},"timestamp":"2026-07-30T10:00:00.000Z"}"#,
        r#"{"type":"file-history-snapshot","messageId":"u1","snapshot":{"messageId":"u1","trackedFileBackups":{"a.java":{"backupFileName":"blob1"}}}}"#,
        r#"{"type":"assistant","uuid":"a1","parentUuid":"u1","message":{"model":"claude-opus-5","content":[{"type":"text","text":"answering the first question"}]}}"#,
        r#"{"type":"last-prompt","leafUuid":"a1","lastPrompt":"first question","sessionId":"sess1"}"#,
        r#"{"type":"queue-operation","operation":"enqueue","content":"second question","sessionId":"sess1"}"#,
        r#"{"type":"user","uuid":"u2","parentUuid":"a1","message":{"role":"user","content":"second question"},"timestamp":"2026-07-30T10:01:00.000Z"}"#,
        r#"{"type":"attachment","uuid":"at1","parentUuid":"u2","attachment":{"type":"task_reminder"}}"#,
        r#"{"type":"file-history-snapshot","messageId":"u2","snapshot":{"messageId":"u2","trackedFileBackups":{"b.java":{"backupFileName":"blob2"}}}}"#,
        r#"{"type":"assistant","uuid":"a2","parentUuid":"at1","message":{"model":"claude-opus-5","content":[{"type":"text","text":"you asked: second question"}]}}"#,
        r#"{"type":"user","uuid":"tr1","parentUuid":"a2","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"grep hit: second question"}]}}"#,
        r#"{"type":"last-prompt","leafUuid":"tr1","lastPrompt":"second question","sessionId":"sess1"}"#,
        r#"{"type":"last-prompt","leafUuid":"a2","lastPrompt":"second question","sessionId":"sess1"}"#,
    ]
    .join("\n")
        + extra
        + "\n"
        + r#"{"type":"user","uuid":"u3","parentUuid":"tr1","message":{"role":"user","content":[{"type":"text","text":"third with image"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"QUJD"}}]},"timestamp":"2026-07-30T10:02:00.000Z"}"#
        + "\n"
        + r#"{"type":"assistant","uuid":"a3","parentUuid":"u3","message":{"model":"claude-opus-5","content":[{"type":"text","text":"ok"}]}}"#
        + "\n"
}

fn msg_home(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
    let home = std::env::temp_dir().join(format!("claude-eclipse-msg-{tag}"));
    let dir = home.join(".claude").join("projects").join("C--msgtest");
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&dir).unwrap();
    (home, dir)
}

fn lines_of(p: &std::path::Path) -> Vec<serde_json::Value> {
    fs::read_to_string(p)
        .unwrap()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn message_ids_track_the_rendered_user_bubbles() {
    let _env = ENV_LOCK.lock().unwrap();
    let (home, dir) = msg_home("ids");
    fs::write(dir.join("sess1.jsonl"), msg_fixture("")).unwrap();
    set_home(&home);

    let ids = super::message_ids(r"C:\msgtest", "sess1");
    let _ = fs::remove_dir_all(&home);

    // The image-bearing message (u3) counts; tool_result turns never do. Each
    // entry carries its text so the GUI can MATCH a bubble instead of guessing
    // by position.
    let v: serde_json::Value = serde_json::from_str(&ids).unwrap();
    let pairs: Vec<(&str, &str)> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|m| (m["id"].as_str().unwrap(), m["text"].as_str().unwrap()))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("u1", "first question"),
            ("u2", "second question"),
            ("u3", "third with image"),
        ],
        "ids follow render order and carry their text: {ids}"
    );
}

#[test]
fn delete_message_relinks_the_chain_and_sweeps_unchained_copies() {
    let _env = ENV_LOCK.lock().unwrap();
    let (home, dir) = msg_home("del");
    let path = dir.join("sess1.jsonl");
    fs::write(&path, msg_fixture("")).unwrap();
    set_home(&home);

    let res = super::delete_message(r"C:\msgtest", "sess1", "u2");
    let after = lines_of(&path);
    let _ = fs::remove_dir_all(&home);

    let v: serde_json::Value = serde_json::from_str(&res).unwrap();
    assert_eq!(v["ok"], serde_json::json!(true), "delete succeeded: {res}");
    // 1 queue-operation + 2 last-prompt copies of THIS prompt.
    assert_eq!(v["stripped"], serde_json::json!(3), "unchained copies cleared: {res}");

    // The message itself is gone.
    assert!(
        !after.iter().any(|l| l["uuid"] == serde_json::json!("u2")),
        "the user line was removed"
    );
    // Its child adopted its parent, so nothing dangles.
    let uuids: std::collections::HashSet<&str> =
        after.iter().filter_map(|l| l["uuid"].as_str()).collect();
    for l in &after {
        if let Some(p) = l["parentUuid"].as_str() {
            assert!(uuids.contains(p), "dangling parentUuid {p} in {l}");
        }
    }
    let at1 = after.iter().find(|l| l["uuid"] == serde_json::json!("at1")).unwrap();
    assert_eq!(at1["parentUuid"], serde_json::json!("a1"), "child re-linked past the hole");

    // Snapshots stay — RewindService forward-merges them for EARLIER messages.
    assert_eq!(
        after
            .iter()
            .filter(|l| l["type"] == serde_json::json!("file-history-snapshot"))
            .count(),
        2,
        "both snapshots preserved"
    );

    // This prompt's unchained copies are cleared…
    for l in &after {
        if l["type"] == serde_json::json!("queue-operation") {
            assert!(
                l["content"].as_str().map_or(true, |c| !c.contains("second question")),
                "queue copy cleared: {l}"
            );
        }
        if l["type"] == serde_json::json!("last-prompt") {
            assert!(
                l["lastPrompt"].as_str().map_or(true, |c| !c.contains("second question")),
                "last-prompt copy cleared: {l}"
            );
        }
    }
    // …while the OTHER message's copy is untouched.
    assert!(
        after.iter().any(|l| l["lastPrompt"] == serde_json::json!("first question")),
        "another message's bookkeeping is left alone"
    );
    // Legitimate echoes survive: they are not the message.
    assert!(
        after.iter().any(|l| l["type"] == serde_json::json!("assistant")
            && l["message"]["content"][0]["text"]
                .as_str()
                .map_or(false, |t| t.contains("second question"))),
        "an assistant quote is not treated as a copy"
    );
    assert!(
        after.iter().any(|l| l["uuid"] == serde_json::json!("tr1")),
        "a tool_result echoing the text is not treated as a copy"
    );
}

/// The guard that matters most: a prompt carrier this code does not know
/// about must abort the delete rather than report a success that leaves the
/// text on disk. Uses a fabricated line type standing in for whatever a
/// future CLI adds.
#[test]
fn delete_message_aborts_on_an_unknown_prompt_carrier() {
    let _env = ENV_LOCK.lock().unwrap();
    let (home, dir) = msg_home("abort");
    let path = dir.join("sess1.jsonl");
    let planted = "\n".to_string()
        + r#"{"type":"future-prompt-log","promptText":"second question","sessionId":"sess1"}"#;
    let original = msg_fixture(&planted);
    fs::write(&path, &original).unwrap();
    set_home(&home);

    let res = super::delete_message(r"C:\msgtest", "sess1", "u2");
    let untouched = fs::read_to_string(&path).unwrap();
    let _ = fs::remove_dir_all(&home);

    let v: serde_json::Value = serde_json::from_str(&res).unwrap();
    assert!(
        v["error"].as_str().unwrap_or("").contains("future-prompt-log"),
        "names the offending line type: {res}"
    );
    assert_eq!(untouched, original, "nothing is written when the assertion fails");
}

#[test]
fn delete_message_guards_bad_input() {
    let _env = ENV_LOCK.lock().unwrap();
    let (home, dir) = msg_home("guard");
    fs::write(dir.join("sess1.jsonl"), msg_fixture("")).unwrap();
    set_home(&home);

    let bad_session = super::delete_message(r"C:\msgtest", "../sess1", "u2");
    let bad_msg = super::delete_message(r"C:\msgtest", "sess1", "");
    let missing = super::delete_message(r"C:\msgtest", "sess1", "nope");
    let _ = fs::remove_dir_all(&home);

    for (label, res) in [
        ("traversal", bad_session),
        ("empty message id", bad_msg),
        ("absent message", missing),
    ] {
        let v: serde_json::Value = serde_json::from_str(&res).unwrap();
        assert!(v["error"].is_string(), "{label} rejected: {res}");
    }
}
