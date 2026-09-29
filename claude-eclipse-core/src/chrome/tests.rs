use super::*;

#[test]
fn the_tab_id_comes_from_the_reply_the_server_really_sends() {
    // Captured from `claude --claude-in-chrome-mcp`, tabs_context_mcp.
    let ctx: Value = serde_json::from_str(
        r#"{"availableTabs":[{"tabId":60923631,"title":"New Tab","url":"chrome://newtab/"}],
                "selectedTabId":60923631,"tabGroupId":1988695452}"#,
    )
    .unwrap();
    assert_eq!(id_string(&ctx["tabGroupId"]), "1988695452");
    assert_eq!(context_tab_id(&ctx), "60923631");

    // No selection: the first available tab stands in.
    let ctx: Value = serde_json::from_str(
        r#"{"availableTabs":[{"tabId":7,"title":"t","url":"u"}],"tabGroupId":3}"#,
    )
    .unwrap();
    assert_eq!(context_tab_id(&ctx), "7");

    // Nothing to go on: empty, and the caller drops the block rather than
    // inventing a tab.
    let ctx: Value = serde_json::from_str(r#"{"tabGroupId":3}"#).unwrap();
    assert!(context_tab_id(&ctx).is_empty());
}

#[test]
fn mentions_follow_the_extension_forms() {
    assert_eq!(browser_mentions("open @browser now"), vec![None]);
    assert_eq!(browser_mentions("@browser"), vec![None]);
    assert_eq!(browser_mentions("@browser:new_tab go"), vec![None]);
    assert_eq!(
        browser_mentions("see @browser:771:12:https://example.com/a?b=c:d then"),
        vec![Some(("771".into(), "12".into(), "https://example.com/a?b=c:d".into()))]
    );
    assert!(browser_mentions("@browserx @browser:tab_title mail@example.com").is_empty());
}

#[test]
fn no_mention_touches_nothing() {
    let mut called = false;
    let blocks = browser_blocks(
        "claude",
        "plain message",
        |_| {
            called = true;
            1
        },
        || panic!("no mention asks for the instruction"),
    );
    assert!(blocks.is_empty() && !called);
}

#[test]
fn named_tab_gets_instruction_once_enabled() {
    let instruction = || Some(format!("{INSTRUCTION_HEAD}\nbody"));
    let blocks = browser_blocks(
        "claude",
        "@browser:9:4:https://x.dev",
        |cfg| {
            assert_eq!(cfg["type"], "stdio");
            1
        },
        instruction,
    );
    assert_eq!(blocks.len(), 2);
    let first = blocks[0]["text"].as_str().unwrap();
    assert!(first.starts_with("<browser_instruction># Claude in Chrome browser automation"));
    assert!(first.ends_with("</browser_instruction>"));
    assert_eq!(blocks[1]["text"], "<browser tabGroupId=\"9\" tabId=\"4\">https://x.dev</browser>");

    let again = browser_blocks("claude", "@browser:9:4:https://x.dev", |_| 0, instruction);
    assert_eq!(again.len(), 1);
}

#[test]
fn without_the_instruction_the_mention_still_goes() {
    let blocks = browser_blocks("claude", "@browser:9:4:https://x.dev", |_| 1, || None);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0]["text"], "<browser tabGroupId=\"9\" tabId=\"4\">https://x.dev</browser>");
}

#[test]
fn template_literal_is_evaluated_like_js() {
    // The shape the CLI bundles: string-literal slots, \u escapes, escaped backticks.
    let src = r#"`# Head
${'ToolSearch with query "select:a,b"'} and ${ "xA\"" } — \`tick\` \${kept} \u{1F600}😀 end`,next=`other`"#;
    assert_eq!(
        eval_template(src).as_deref(),
        Some("# Head\nToolSearch with query \"select:a,b\" and xA\" \u{2014} `tick` ${kept} \u{1F600}\u{1F600} end")
    );
    // Anything that isn't a plain literal is refused rather than guessed at.
    assert_eq!(eval_template("`a ${name} b`"), None);
    assert_eq!(eval_template("`no closing backtick"), None);
    assert_eq!(eval_template(r"`bad \u12 escape`"), None);
    assert_eq!(eval_template("not a template"), None);
}

#[test]
fn a_match_split_across_two_reads_is_found() {
    let hay = b"0123456789`# Head and more".to_vec();
    for chunk in [1, 3, 4, 11, 64] {
        assert_eq!(find_in_reader(&mut &hay[..], b"`# Head", chunk), Some(10), "chunk {chunk}");
    }
    assert_eq!(find_in_reader(&mut &hay[..], b"`# Nope", 4), None);
}

#[test]
fn instruction_is_read_out_of_a_program_file() {
    let body = "call tabs_context_mcp first. ".repeat(60);
    let mut file = vec![0xFFu8, 0x00, 0xC3, 0x28]; // not UTF-8, like a real binary
    file.extend_from_slice(b"var a=1;var O9e=`");
    file.extend_from_slice(format!("{INSTRUCTION_HEAD}\n{body}${{'x'}} \\u2014 {INSTRUCTION_TAIL}`,P=`next`").as_bytes());
    file.extend_from_slice(&[0xFE, 0xFF]);
    let path = std::env::temp_dir().join("claude-eclipse-instruction-fixture.bin");
    std::fs::write(&path, &file).unwrap();

    let text = read_instruction(&path);
    let short = std::env::temp_dir().join("claude-eclipse-instruction-short.bin");
    std::fs::write(&short, format!("`{INSTRUCTION_HEAD}\n{INSTRUCTION_TAIL}`")).unwrap();
    let too_short = read_instruction(&short);
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&short);

    assert_eq!(text, Some(format!("{INSTRUCTION_HEAD}\n{body}x \u{2014} {INSTRUCTION_TAIL}")));
    assert_eq!(too_short, None, "a fragment is not the instruction");
}

#[test]
fn a_slot_may_name_a_constant() {
    let mut consts = |name: &str| (name == "YL").then(|| "## Loaded".to_string());
    assert_eq!(eval_template_with("`a ${YL} b ${ YL } c`", &mut consts).as_deref(), Some("a ## Loaded b ## Loaded c"));
    // An unknown name, or an expression, refuses the whole literal.
    assert_eq!(eval_template_with("`a ${ZZ} b`", &mut consts), None);
    assert_eq!(eval_template_with("`a ${YL+1} b`", &mut consts), None);
    assert_eq!(eval_template_with("`a ${YL.x} b`", &mut consts), None);
}

/// The 2.1.280 layout: a constant `YL`, the instruction with `${YL}` in it, and the
/// same instruction without it, chosen between by `j7e`.
fn layout_2_1_280(yl_definition: &str) -> Vec<u8> {
    let body = "call tabs_context_mcp first. ".repeat(60);
    let mut file = vec![0xFFu8, 0x00, 0xC3, 0x28];
    file.extend_from_slice(
        format!(
            "{yl_definition};var qL=`{INSTRUCTION_HEAD}\n{body}\n${{YL}}\n{INSTRUCTION_TAIL}`;\
                 function j7e(e){{return e?qL:`{INSTRUCTION_HEAD}\n{body}\n{INSTRUCTION_TAIL}`}}"
        )
        .as_bytes(),
    );
    file.extend_from_slice(&[0xFE, 0xFF]);
    file
}

#[test]
fn the_tool_search_version_is_read_with_its_constant() {
    let path = std::env::temp_dir().join("claude-eclipse-instruction-2_1_280.bin");
    let body = "call tabs_context_mcp first. ".repeat(60);

    std::fs::write(&path, layout_2_1_280(r#"var aYL=`wrong`,YL=`## Loading deferred tools ${'select:a,b'} — done`"#)).unwrap();
    let full = read_instruction(&path);
    // No definition to be found: the version without the paragraph.
    std::fs::write(&path, layout_2_1_280("var ZZ=1")).unwrap();
    let without_constant = read_instruction(&path);
    // A constant with a name of its own inside: refused, not guessed at.
    std::fs::write(&path, layout_2_1_280("var YL=`deeper ${XX}`")).unwrap();
    let nested = read_instruction(&path);
    let _ = std::fs::remove_file(&path);

    let plain = format!("{INSTRUCTION_HEAD}\n{body}\n{INSTRUCTION_TAIL}");
    assert_eq!(full, Some(format!("{INSTRUCTION_HEAD}\n{body}\n## Loading deferred tools select:a,b \u{2014} done\n{INSTRUCTION_TAIL}")));
    assert_eq!(without_constant.as_deref(), Some(plain.as_str()));
    assert_eq!(nested.as_deref(), Some(plain.as_str()));
}

/// Opt-in: reads the Claude Code CLI installed on this machine. Run with
/// `cargo test --release -- --ignored installed_cli`.
#[test]
#[ignore = "reads the installed Claude Code CLI"]
fn installed_cli_carries_the_instruction() {
    let started = std::time::Instant::now();
    let text = cli_instruction("claude").expect("instruction not found in the installed CLI");
    eprintln!("instruction: {} bytes, read in {:?}", text.len(), started.elapsed());
    // Written out so it can be compared byte for byte with the extension's text.
    let dump = std::env::temp_dir().join("claude-eclipse-cli-instruction.txt");
    std::fs::write(&dump, &text).unwrap();
    eprintln!("written to {}", dump.display());
    let cached = std::time::Instant::now();
    assert_eq!(cli_instruction("claude").as_deref(), Some(text.as_str()));
    eprintln!("cached: {:?}", cached.elapsed());
    // A failure here means the CLI's wording has moved on from the sealed copy:
    // re-seal from the dump above (seal_the_instruction).
    assert!(instruction().as_deref() == Some(text.as_str()), "the installed CLI's wording differs from the sealed copy");
}

#[test]
fn the_sealed_instruction_opens() {
    let text = instruction().expect("the container did not open");
    assert!(text.starts_with(INSTRUCTION_HEAD));
    assert!(text.ends_with(INSTRUCTION_TAIL));
    assert!(text.chars().count() > 1000 && !text.contains('\u{FFFD}'));
    // A checkout's line endings, or a wrapped value, change nothing.
    let value = INSTRUCTION_CONTAINER.lines().find_map(|l| l.trim().strip_prefix("inst_hash=")).unwrap();
    let (a, b) = value.split_at(value.len() / 2);
    let wrapped = format!("# comment\r\ninst_hash={a}\r\n  {b}\r\n");
    assert_eq!(open_instruction(&wrapped).as_deref(), Some(text.as_str()));
}

#[test]
fn a_damaged_container_does_not_open() {
    use base64::Engine as _;
    let value = INSTRUCTION_CONTAINER.lines().find_map(|l| l.trim().strip_prefix("inst_hash=")).unwrap();
    let mut bytes = base64::engine::general_purpose::STANDARD.decode(value.trim()).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    let tampered = format!("inst_hash={}", base64::engine::general_purpose::STANDARD.encode(&bytes));
    assert_eq!(open_instruction(&tampered), None);
    assert_eq!(open_instruction("inst_hash="), None);
    assert_eq!(open_instruction("inst_hash=not base64!"), None);
    assert_eq!(open_instruction(""), None);
}

/// Opt-in maintenance: seals the text in the file named by
/// `CLAUDE_ECLIPSE_INSTRUCTION` (a dump from installed_cli_carries_the_instruction)
/// into `.settings/com.eclipse.chrome.container`, then reads it back.
#[test]
#[ignore = "rewrites .settings/com.eclipse.chrome.container"]
fn seal_the_instruction() {
    use base64::Engine as _;
    use ring::aead::{Aad, Nonce, NONCE_LEN};
    use ring::rand::{SecureRandom, SystemRandom};
    let Ok(source) = std::env::var("CLAUDE_ECLIPSE_INSTRUCTION") else { return };
    let text = std::fs::read_to_string(source.trim()).unwrap();
    assert!(text.starts_with(INSTRUCTION_HEAD) && text.ends_with(INSTRUCTION_TAIL), "not the instruction");

    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new().fill(&mut nonce).unwrap();
    let mut sealed = text.as_bytes().to_vec();
    instruction_key()
        .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), Aad::from(INSTRUCTION_AAD), &mut sealed)
        .unwrap();
    let mut value = nonce.to_vec();
    value.extend(sealed);
    let container = format!("inst_hash={}\n", base64::engine::general_purpose::STANDARD.encode(value));

    assert_eq!(open_instruction(&container).as_deref(), Some(text.as_str()));
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".settings/com.eclipse.chrome.container");
    std::fs::write(&path, container).unwrap();
    eprintln!("sealed {} chars into {}", text.chars().count(), path.display());
}

/// Opt-in: reads each CLI program listed in `CLAUDE_ECLIPSE_CLIS` (separated by
/// `;`) — other installs, say VS Code's bundled ones — and writes what was read to
/// `claude-eclipse-cli-instruction-<n>.txt` in the temp folder for comparison.
#[test]
#[ignore = "reads the Claude Code CLIs listed in CLAUDE_ECLIPSE_CLIS"]
fn listed_clis_carry_the_instruction() {
    let Ok(list) = std::env::var("CLAUDE_ECLIPSE_CLIS") else { return };
    for (n, path) in list.split(';').filter(|p| !p.trim().is_empty()).enumerate() {
        let text = read_instruction(std::path::Path::new(path.trim()))
            .unwrap_or_else(|| panic!("instruction not found in {path}"));
        let dump = std::env::temp_dir().join(format!("claude-eclipse-cli-instruction-{n}.txt"));
        std::fs::write(&dump, &text).unwrap();
        eprintln!("{path}: {} bytes -> {}", text.len(), dump.display());
    }
}
