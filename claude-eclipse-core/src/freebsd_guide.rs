//! The FreeBSD setup guide shown in the Claude GUI view when the `claude` CLI is
//! missing.
//!
//! The guide's content lives in `.settings/org.eclipse.core.freebsd.container`
//! (JSON) and is compiled into the FreeBSD builds of this library, so editing it
//! only takes effect once the natives are rebuilt. It is rendered here to the
//! Markdown dialect `markdown.js` understands (headings, paragraphs, lists and
//! fenced code, each fence getting its own Copy button). Other platforms carry no
//! guide and get an empty string.
//!
//! Rendering reads the JSON loosely: a missing or renamed key drops that piece of
//! the guide instead of failing, so the file can be edited freely.
//!
//! The same JSON also goes to Claude itself: when a tool fails the way it does
//! without the fdescfs fix ([`is_fdescfs_failure`]), the chat queues
//! [`diagnosis_message`] onto the live process, once per process, so Claude can
//! tell the user what to change.

// Everything below the two entry points is FreeBSD-only outside tests.
#![cfg_attr(not(target_os = "freebsd"), allow(dead_code))]

#[cfg(target_os = "freebsd")]
const GUIDE_JSON: &str = include_str!("../.settings/org.eclipse.core.freebsd.container");

/// Whether a failed tool result is the fdescfs failure the guide fixes: the
/// Linux-ABI `claude` hitting ENOTDIR on its own task folder or file writes.
/// Always false outside FreeBSD.
pub fn is_fdescfs_failure(error_text: &str) -> bool {
    cfg!(target_os = "freebsd") && matches_fdescfs_failure(error_text)
}

fn matches_fdescfs_failure(t: &str) -> bool {
    t.contains("ENOTDIR")
        && (t.contains("swapped leaf") || t.contains("/tasks") || t.contains("not a directory, open"))
}

/// The message queued to Claude after [`is_fdescfs_failure`] matched, carrying the
/// guide as JSON. `None` outside FreeBSD.
pub fn diagnosis_message(error_text: &str) -> Option<String> {
    #[cfg(target_os = "freebsd")]
    {
        Some(compose_diagnosis(error_text, GUIDE_JSON))
    }
    #[cfg(not(target_os = "freebsd"))]
    {
        let _ = error_text;
        None
    }
}

fn compose_diagnosis(error_text: &str, guide_json: &str) -> String {
    format!(
        "[Sent automatically by the Eclipse plugin, not typed by the user.]\n\n\
         A tool call just failed with:\n\n```\n{}\n```\n\n\
         This is the known FreeBSD fdescfs problem: Claude Code runs under the Linux \
         ABI, and without a patched fdescfs module and `/dev/fd` mounted \
         `linrdlnk,nodup`, the Bash and Write tools fail with ENOTDIR. Read still works.\n\n\
         Using the guide below, tell the user how to fix it on this machine. Rules:\n\
         - Do not retry Bash or Write, and do not try to apply the fix yourself: it \
           needs root, and unmounting fdescfs or unloading the module from inside a \
           running Claude session kills that session.\n\
         - Tell the user to run the commands from a plain terminal with Claude closed, \
           then restart Claude.\n\
         - Where it helps, use Read to check the current state first (for example \
           `/etc/fstab`, `/boot/loader.conf`, `/etc/rc.local`, `/boot/modules/`).\n\n\
         The guide (JSON):\n\n```json\n{}\n```",
        error_text.trim(),
        guide_json.trim()
    )
}

/// The guide as Markdown, or empty when this platform has none (or the JSON
/// does not parse).
pub fn markdown() -> String {
    #[cfg(target_os = "freebsd")]
    {
        match serde_json::from_str::<serde_json::Value>(GUIDE_JSON) {
            Ok(v) => render(&v),
            Err(_) => String::new(),
        }
    }
    #[cfg(not(target_os = "freebsd"))]
    {
        String::new()
    }
}

fn render(g: &serde_json::Value) -> String {
    let mut md = String::new();
    let fix = &g["fix"];
    let install = &g["install"];

    md.push_str("## Claude Code isn't installed\n\n");
    md.push_str(
        "This view runs the `claude` command, and it wasn't found on this machine. \
         On FreeBSD, Claude Code runs as a Linux program through the Linux ABI, \
         which takes a few extra steps. Commands marked **root** need `sudo`.\n\n",
    );
    para(&mut md, &g["summary"]);
    if let Some(t) = g["tested_on"].as_object() {
        let parts: Vec<String> = t
            .iter()
            .filter_map(|(k, v)| v.as_str().map(|v| format!("{}: {}", k.replace('_', " "), v)))
            .collect();
        if !parts.is_empty() {
            md.push_str(&format!("*Tested on {}.*\n\n", parts.join(" · ")));
        }
    }

    // 1. Requirements
    md.push_str("### 1. Requirements\n\n");
    let req = &g["requirements"];
    if let Some(a) = req["arch"].as_array() {
        let a: Vec<&str> = a.iter().filter_map(|x| x.as_str()).collect();
        md.push_str(&format!("- **Architecture:** {}\n", a.join(", ")));
    }
    if req["root"].as_bool() == Some(true) {
        md.push_str("- **Root access** (`sudo`)\n");
    }
    if let Some(s) = req["kernel_source"].as_str() {
        md.push_str(&format!("- **Kernel source:** {s}\n"));
    }
    for p in arr(&req["packages"]) {
        if let (Some(n), Some(why)) = (p["name"].as_str(), p["purpose"].as_str()) {
            md.push_str(&format!("- `{n}`: {why}\n"));
        }
    }
    md.push('\n');

    // 2. Linux ABI
    let abi = &install["linux_abi"];
    md.push_str("### 2. Turn on the Linux ABI (root)\n\n");
    code(&mut md, &abi["commands"]);
    para(&mut md, &abi["notes"]);

    // 3. Claude Code
    let claude = &install["claude"];
    md.push_str("### 3. Install Claude Code\n\n");
    if let Some(src) = claude["source"].as_str() {
        md.push_str(&format!(
            "Use `claude-freebsd` ({src}), which installs the official Linux build behind a wrapper.\n\n"
        ));
    }
    code(&mut md, &claude["commands"]);
    if let Some(w) = claude["what_it_installs"].as_object() {
        md.push_str("It installs:\n\n");
        for (k, v) in w {
            if let Some(v) = v.as_str() {
                md.push_str(&format!("- **{}:** {}\n", k.replace('_', " "), v));
            }
        }
        md.push('\n');
    }
    for (label, key) in [("Update", "update"), ("Uninstall", "uninstall")] {
        if let Some(c) = claude[key].as_str() {
            md.push_str(&format!("- **{label}:** `{c}`\n"));
        }
    }
    md.push('\n');
    bullets(&mut md, "What the wrapper does:", &claude["wrapper_behaviour"]);

    // npm
    let npm = &install["npm"];
    if npm.is_object() {
        md.push_str("**npm.** ");
        if let Some(env) = npm["wrapper_env"].as_object() {
            for (k, v) in env {
                if let Some(v) = v.as_str() {
                    md.push_str(&format!("The wrapper sets `{k}={v}`. "));
                }
            }
        }
        if let Some(r) = npm["reason"].as_str() {
            md.push_str(r);
            md.push(' ');
        }
        if let Some(n) = npm["note"].as_str() {
            md.push_str(n);
        }
        md.push_str("\n\n");
    }

    // 4. The fdescfs fix
    md.push_str("### 4. Fix the Bash and Write tools (root)\n\n");
    para(&mut md, &fix["problem"]);
    bullets(&mut md, "Without it, you'll see errors like:", &fix["symptoms"]);

    let patch = &fix["kernel_patch"];
    if let Some(file) = patch["file"].as_str() {
        md.push_str(&format!("**Patch `{file}`.** "));
        para(&mut md, &patch["purpose"]);
        for e in arr(&patch["edits"]) {
            if let Some(loc) = e["location"].as_str() {
                md.push_str(&format!("In {loc}, change:\n\n"));
            }
            fence(&mut md, "c", &[e["before"].as_str().unwrap_or("")]);
            md.push_str("to:\n\n");
            fence(&mut md, "c", &[e["after"].as_str().unwrap_or("")]);
        }
    }

    let build = &fix["build"];
    if let Some(cmd) = build["command"].as_str() {
        md.push_str("**Build the module** with base-system tools only:\n\n");
        fence(&mut md, "sh", &[cmd]);
        bullets(&mut md, "", &build["rules"]);
        checks(&mut md, "Check the build before loading it:", &build["verify"]);
    }

    let steps = arr(&fix["persistent_config"]);
    if !steps.is_empty() {
        md.push_str("**Make it permanent:**\n\n");
        for s in steps {
            let file = s["file"].as_str().unwrap_or("");
            match s["action"].as_str() {
                Some("install_module") => {
                    md.push_str("Install the module:\n\n");
                    fence(&mut md, "sh", &[s["command"].as_str().unwrap_or("")]);
                }
                Some("append") => {
                    md.push_str(&format!("Add to `{file}`:\n\n"));
                    let lines: Vec<&str> = arr(&s["lines"]).iter().filter_map(|l| l.as_str()).collect();
                    fence(&mut md, "sh", &lines);
                }
                Some("edit_line") => {
                    md.push_str(&format!("In `{file}`, change the line to:\n\n"));
                    fence(&mut md, "sh", &[s["after"].as_str().unwrap_or("")]);
                }
                _ => {}
            }
            para(&mut md, &s["note"]);
        }
    }
    if let Some(a) = fix["activate"].as_str() {
        md.push_str(&format!("**Then:** {a}\n\n"));
    }
    if !arr(&fix["undo"]).is_empty() {
        md.push_str("To undo it:\n\n");
        code(&mut md, &fix["undo"]);
    }

    // 5. Verify
    checks(&mut md, "### 5. Check that it works", &g["verification"]);

    // Warnings
    bullets(&mut md, "### Before you start", &g["warnings"]);

    md.push_str("Once `claude` is installed, reopen this view.\n");
    md
}

fn arr(v: &serde_json::Value) -> &[serde_json::Value] {
    v.as_array().map(|a| a.as_slice()).unwrap_or(&[])
}

/// A paragraph from a string value; nothing for anything else.
fn para(md: &mut String, v: &serde_json::Value) {
    if let Some(s) = v.as_str() {
        md.push_str(s);
        md.push_str("\n\n");
    }
}

/// A string array as one `sh` fence.
fn code(md: &mut String, v: &serde_json::Value) {
    let lines: Vec<&str> = arr(v).iter().filter_map(|l| l.as_str()).collect();
    if !lines.is_empty() {
        fence(md, "sh", &lines);
    }
}

fn fence(md: &mut String, lang: &str, lines: &[&str]) {
    md.push_str("```");
    md.push_str(lang);
    md.push('\n');
    for l in lines {
        md.push_str(l);
        md.push('\n');
    }
    md.push_str("```\n\n");
}

/// A string array as a bulleted list under an optional heading or lead-in line.
fn bullets(md: &mut String, title: &str, v: &serde_json::Value) {
    let items: Vec<&str> = arr(v).iter().filter_map(|l| l.as_str()).collect();
    if items.is_empty() {
        return;
    }
    if !title.is_empty() {
        md.push_str(title);
        md.push_str("\n\n");
    }
    for i in items {
        md.push_str("- ");
        md.push_str(i);
        md.push('\n');
    }
    md.push('\n');
}

/// `[{command, expect}]` as a fenced command followed by what it should print.
fn checks(md: &mut String, title: &str, v: &serde_json::Value) {
    let items = arr(v);
    if items.is_empty() {
        return;
    }
    md.push_str(title);
    md.push_str("\n\n");
    for c in items {
        if let Some(cmd) = c["command"].as_str() {
            fence(md, "sh", &[cmd]);
            if let Some(e) = c["expect"].as_str() {
                md.push_str(&format!("Expect: {e}\n\n"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn guide_json_renders_every_section() {
        let json = include_str!("../.settings/org.eclipse.core.freebsd.container");
        let v: serde_json::Value = serde_json::from_str(json).expect("guide JSON parses");
        let md = super::render(&v);
        for heading in ["### 1.", "### 2.", "### 3.", "### 4.", "### 5.", "### Before you start"] {
            assert!(md.contains(heading), "missing {heading}");
        }
        assert!(md.contains("claude-freebsd"));
        assert!(md.contains("linrdlnk,nodup"));
        // Every fence is closed.
        assert_eq!(md.matches("```").count() % 2, 0);
    }

    #[test]
    fn fdescfs_failure_signature() {
        let m = super::matches_fdescfs_failure;
        assert!(m("task output swap refused (open refused a swapped leaf (ENOTDIR)): /tmp/claude-1001/x/tasks/b.output"));
        assert!(m("ENOTDIR: not a directory, mkdir '/tmp/claude-1001/-home-u-p/c248/tasks'"));
        assert!(m("ENOTDIR: not a directory, open '/home/u/file.sh'"));
        assert!(!m("ENOENT: no such file or directory, open '/home/u/file.sh'"));
        assert!(!m("cd: foo: Not a directory"));
    }

    #[test]
    fn diagnosis_carries_error_and_guide() {
        let msg = super::compose_diagnosis("ENOTDIR boom", "{\"a\":1}");
        assert!(msg.contains("ENOTDIR boom"));
        assert!(msg.contains("```json\n{\"a\":1}\n```"));
        assert!(msg.contains("Do not retry Bash or Write"));
    }
}
