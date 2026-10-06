//! Dialogs that SWT cannot reach from Java: native ones raised inside this Eclipse (a
//! message box, a file chooser), and the dialogs of other Eclipse instances.
//!
//! Eclipse's own SWT dialogs are handled in Java, where they are widgets. What is left
//! is asked of the operating system, and each one answers differently, so every
//! platform has its own branch below.
//!
//! Two rules hold on all of them. Only Eclipse and the Java programs it started are
//! ever looked at, never another application's windows (see [`in_scope`]). And a button
//! is pressed only when named in full; nothing here picks a default (see [`choose`]).

use std::collections::{HashMap, HashSet};

use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

/// The dialogs found, as `{"dialogs":[…]}`. `in_process_only` asks for this Eclipse's
/// own native dialogs alone, which is cheaper: no other process is looked at.
/// `"unavailable"` says why nothing could be read, when the platform refused; `"log"`
/// carries diagnostics for the server view while debug mode is on.
pub(crate) fn list_json(in_process_only: bool) -> String {
    let mut log = Log::default();
    let mut out = match imp::find(in_process_only, &mut log) {
        Ok(found) => json!({ "dialogs": describe(&found) }),
        Err(why) => json!({ "dialogs": [], "unavailable": why }),
    };
    log.attach(&mut out);
    out.to_string()
}

/// Presses the button labelled `label` in the dialog listed as `id`:
/// `{"pressed":…,"dialog":…}` or `{"error":…}`.
pub(crate) fn press_json(id: &str, label: &str) -> String {
    let mut log = Log::default();
    let mut out = match press(id, label, &mut log) {
        Ok((pressed, dialog)) => json!({ "pressed": pressed, "dialog": dialog }),
        Err(why) => json!({ "error": why }),
    };
    log.attach(&mut out);
    out.to_string()
}

fn press(id: &str, label: &str, log: &mut Log) -> Result<(String, String), String> {
    if parse_id(id).is_none() {
        return Err(format!("'{id}' is not the id of a dialog listed by action='list'."));
    }
    // Found afresh, so the press goes to what is on screen now and not to what a
    // listing saw a moment ago.
    let found = imp::find(false, log)?;
    let (dialog, button) = choose(&found, id, label)?;
    imp::press(dialog, button, log)?;
    Ok((button.label.clone(), dialog.title.clone()))
}

/// Diagnostics for the server view. `eprintln!` never gets there, so they travel back
/// in the reply and Java logs them; nothing is collected unless debug mode is on.
#[derive(Default)]
struct Log(Vec<String>);

impl Log {
    fn add(&mut self, line: impl FnOnce() -> String) {
        if crate::is_debug() {
            self.0.push(line());
        }
    }

    fn attach(self, out: &mut Value) {
        if !self.0.is_empty() {
            out["log"] = json!(self.0);
        }
    }
}

// ---------------------------------------------------------------------------
// Which processes may be looked at
// ---------------------------------------------------------------------------

/// One row of the process table.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
}

/// A program's name as the scope rules compare it: no folder, no `.exe`, lower case.
fn image(name: &str) -> String {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name).trim().to_lowercase();
    file.strip_suffix(".exe").map(str::to_string).unwrap_or(file)
}

fn is_eclipse_image(name: &str) -> bool {
    matches!(image(name).as_str(), "eclipse" | "eclipsec")
}

fn is_java_image(name: &str) -> bool {
    matches!(image(name).as_str(), "java" | "javaw")
}

/// The processes whose dialogs may be listed and pressed, seen from process `me`:
/// this one, every Eclipse, the Eclipse or Java processes that started this one, and
/// the Java programs started under any of those.
///
/// Nothing else is ever in scope: not the shell or desktop that started Eclipse, and
/// not the other programs Eclipse starts (a browser, git, the CLI).
pub(crate) fn in_scope(procs: &[Proc], me: u32) -> HashSet<u32> {
    let by_pid: HashMap<u32, &Proc> = procs.iter().map(|p| (p.pid, p)).collect();
    let parent_of = |pid: u32| by_pid.get(&pid).and_then(|p| by_pid.get(&p.ppid)).copied();

    let mut scope: HashSet<u32> = HashSet::from([me]);
    scope.extend(procs.iter().filter(|p| is_eclipse_image(&p.name)).map(|p| p.pid));

    // Upwards only through Eclipse and Java: a runtime Eclipse is a Java process whose
    // parent is the Eclipse that launched it. Bounded, because a parent id can outlive
    // its process and be handed to a newer one, which makes the table loop.
    let mut current = me;
    for _ in 0..procs.len() {
        match parent_of(current) {
            Some(parent) if is_eclipse_image(&parent.name) || is_java_image(&parent.name) => {
                scope.insert(parent.pid);
                current = parent.pid;
            }
            _ => break,
        }
    }

    // Downwards to Java programs, through whatever started them (a shell, a launcher).
    let roots = scope.clone();
    for p in procs.iter().filter(|p| is_java_image(&p.name)) {
        let mut current = p.pid;
        for _ in 0..procs.len() {
            match parent_of(current) {
                Some(parent) if roots.contains(&parent.pid) => {
                    scope.insert(p.pid);
                    break;
                }
                Some(parent) => current = parent.pid,
                None => break,
            }
        }
    }
    scope
}

// ---------------------------------------------------------------------------
// Labels and ids
// ---------------------------------------------------------------------------

/// A button's label as it reads on screen, without the toolkit's mnemonic marker
/// (`&` on Windows, `_` in GTK). A doubled marker is a literal one.
pub(crate) fn plain_label(raw: &str, mnemonic: Option<char>) -> String {
    let Some(marker) = mnemonic else { return raw.trim().to_string() };
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c != marker {
            out.push(c);
        } else if chars.peek() == Some(&marker) {
            out.push(marker);
            chars.next();
        }
    }
    out.trim().to_string()
}

#[derive(Debug, PartialEq)]
pub(crate) enum Pick {
    One(usize),
    None,
    Many,
}

/// Which of `labels` is `wanted`, ignoring case. Never a prefix or a guess.
pub(crate) fn pick(labels: &[String], wanted: &str) -> Pick {
    let wanted = wanted.trim().to_lowercase();
    if wanted.is_empty() {
        return Pick::None;
    }
    let mut hits = labels.iter().enumerate().filter(|(_, label)| label.to_lowercase() == wanted);
    match (hits.next(), hits.next()) {
        (Some((index, _)), None) => Pick::One(index),
        (Some(_), Some(_)) => Pick::Many,
        _ => Pick::None,
    }
}

const ID_PREFIX: &str = "os-";

/// The id a dialog is listed under: its process and the platform's own handle for it.
/// The handle is hex-encoded, so the id is letters, digits and dashes whatever a
/// platform's handle looks like.
pub(crate) fn make_id(pid: u32, token: &str) -> String {
    let hex: String = token.bytes().map(|b| format!("{b:02x}")).collect();
    format!("{ID_PREFIX}{pid}-{hex}")
}

pub(crate) fn parse_id(id: &str) -> Option<(u32, String)> {
    let (pid, hex) = id.strip_prefix(ID_PREFIX)?.split_once('-')?;
    let pid = pid.parse().ok()?;
    if hex.is_empty() || hex.len() % 2 != 0 || !hex.is_ascii() {
        return None;
    }
    let bytes: Option<Vec<u8>> =
        (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok()).collect();
    Some((pid, String::from_utf8(bytes?).ok()?))
}

// ---------------------------------------------------------------------------
// What a platform reports, and what is done with it
// ---------------------------------------------------------------------------

/// A dialog as a platform branch found it. `H` is that platform's handle for a button.
#[derive(Debug)]
pub(crate) struct Found<H> {
    pub pid: u32,
    pub process: String,
    /// In another process than this one.
    pub external: bool,
    pub token: String,
    pub title: String,
    pub text: String,
    /// Another window is modal over it, so the user could not click it either.
    pub blocked: bool,
    pub buttons: Vec<FoundButton<H>>,
}

#[derive(Debug)]
pub(crate) struct FoundButton<H> {
    pub label: String,
    pub enabled: bool,
    pub default: bool,
    pub handle: H,
}

pub(crate) fn describe<H>(found: &[Found<H>]) -> Value {
    let dialogs: Vec<Value> = found
        .iter()
        .map(|d| {
            let buttons: Vec<Value> = d
                .buttons
                .iter()
                .map(|b| {
                    let mut button = json!({ "label": b.label });
                    if !b.enabled {
                        button["enabled"] = json!(false);
                    }
                    if b.default {
                        button["default"] = json!(true);
                    }
                    button
                })
                .collect();
            let mut dialog = json!({
                "id": make_id(d.pid, &d.token),
                "title": d.title,
                "buttons": buttons,
                "process": { "pid": d.pid, "name": d.process },
                "external": d.external,
            });
            if !d.text.is_empty() {
                dialog["text"] = json!(d.text);
            }
            if d.blocked {
                dialog["blocked"] = json!(true);
            }
            dialog
        })
        .collect();
    json!(dialogs)
}

/// The dialog `id` names and the one button in it labelled `wanted`, or why that
/// press cannot happen.
pub(crate) fn choose<'a, H>(
    found: &'a [Found<H>],
    id: &str,
    wanted: &str,
) -> Result<(&'a Found<H>, &'a FoundButton<H>), String> {
    let dialog = found.iter().find(|d| make_id(d.pid, &d.token) == id).ok_or_else(|| {
        format!("No open dialog has id '{id}'. Use action='list' to see the dialogs open now.")
    })?;
    let title = &dialog.title;
    if dialog.blocked {
        return Err(format!(
            "'{title}' is waiting on another window that is open over it. Answer that one first."
        ));
    }
    let labels: Vec<String> = dialog.buttons.iter().map(|b| b.label.clone()).collect();
    let wanted = wanted.trim();
    let button = match pick(&labels, wanted) {
        Pick::One(index) => &dialog.buttons[index],
        Pick::Many => return Err(format!("More than one button in '{title}' is labelled '{wanted}'.")),
        Pick::None => {
            return Err(format!(
                "'{title}' has no button labelled '{wanted}'. Its buttons: {}",
                labels.join(", ")
            ))
        }
    };
    if !button.enabled {
        return Err(format!("The '{}' button in '{title}' is disabled.", button.label));
    }
    Ok((dialog, button))
}

/// How much of a dialog's text is reported.
const MAX_TEXT_CHARS: usize = 2000;

/// Adds one more piece of a dialog's text, a line each, up to [`MAX_TEXT_CHARS`].
fn append_text(text: &mut String, more: &str) {
    let more = more.trim();
    let used = text.chars().count();
    if more.is_empty() || used >= MAX_TEXT_CHARS {
        return;
    }
    if !text.is_empty() {
        text.push('\n');
    }
    let room = MAX_TEXT_CHARS - used;
    if more.chars().count() <= room {
        text.push_str(more);
    } else {
        text.extend(more.chars().take(room));
        text.push_str("…(truncated)");
    }
}

// ---------------------------------------------------------------------------
// Windows: top-level windows and their child controls, over user32
// ---------------------------------------------------------------------------

/// The window class of a Win32 dialog box: a message box, a file or folder chooser.
#[cfg(any(windows, test))]
const WIN_DIALOG_CLASS: &str = "#32770";
#[cfg(any(windows, test))]
const WS_CAPTION: u32 = 0x00C0_0000;
#[cfg(any(windows, test))]
const WS_MINIMIZEBOX: u32 = 0x0002_0000;

/// Whether a visible top-level window is a dialog this module answers for.
///
/// A Win32 dialog box always is. An SWT shell is one when it belongs to another
/// Eclipse and is not that Eclipse's main window: it has a title bar, and either an
/// owner or no minimise button. This Eclipse's own shells are left to Java, which sees
/// them as widgets.
#[cfg(any(windows, test))]
fn is_dialog_window(class: &str, style: u32, has_owner: bool, external: bool) -> bool {
    if class == WIN_DIALOG_CLASS {
        return true;
    }
    external
        && class.starts_with("SWT_Window")
        && style & WS_CAPTION == WS_CAPTION
        && (has_owner || style & WS_MINIMIZEBOX == 0)
}

/// Whether a `Button` control with this style is one that is clicked to act: a push
/// button, split button or command link, and not a checkbox, radio button or group box.
#[cfg(any(windows, test))]
fn is_push_button(style: u32) -> bool {
    matches!(style & 0xF, 0x0 | 0x1 | 0xC | 0xD | 0xE | 0xF)
}

/// Whether that push button is the dialog's default one.
#[cfg(any(windows, test))]
fn is_default_button(style: u32) -> bool {
    matches!(style & 0xF, 0x1 | 0xD | 0xF)
}

/// A link control's text without its `<a>` markup.
#[cfg(any(windows, test))]
fn strip_links(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        let tag = &rest[open..];
        let is_anchor = tag.len() > 1
            && (tag[1..].starts_with(['a', 'A']) || tag[1..].starts_with("/a") || tag[1..].starts_with("/A"));
        match (is_anchor, tag.find('>')) {
            (true, Some(close)) => {
                out.push_str(&rest[..open]);
                rest = &tag[close + 1..];
            }
            _ => {
                out.push_str(&rest[..=open]);
                rest = &tag[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(windows)]
mod imp {
    use super::*;

    /// A button's window handle.
    pub(super) type Handle = isize;

    const GWL_STYLE: i32 = -16;
    const GW_OWNER: u32 = 4;
    const ES_READONLY: u32 = 0x0800;
    const WM_GETTEXT: u32 = 0x000D;
    const WM_GETTEXTLENGTH: u32 = 0x000E;
    const WM_COMMAND: u32 = 0x0111;
    const SMTO_ABORTIFHUNG: u32 = 0x0002;
    /// A window that does not answer for its text within this is skipped, not waited on.
    const TEXT_TIMEOUT_MS: u32 = 200;
    const MAX_CONTROL_TEXT: usize = 4096;
    const TH32CS_SNAPPROCESS: u32 = 0x0000_0002;
    const INVALID_HANDLE_VALUE: isize = -1;

    #[repr(C)]
    struct ProcessEntry32W {
        size: u32,
        usage: u32,
        process_id: u32,
        default_heap_id: usize,
        module_id: u32,
        threads: u32,
        parent_process_id: u32,
        priority_class_base: i32,
        flags: u32,
        exe_file: [u16; 260],
    }

    type EnumProc = unsafe extern "system" fn(isize, isize) -> i32;

    #[link(name = "user32")]
    extern "system" {
        fn EnumWindows(callback: EnumProc, lparam: isize) -> i32;
        fn EnumChildWindows(parent: isize, callback: EnumProc, lparam: isize) -> i32;
        fn GetClassNameW(hwnd: isize, buffer: *mut u16, max: i32) -> i32;
        fn IsWindow(hwnd: isize) -> i32;
        fn IsWindowVisible(hwnd: isize) -> i32;
        fn IsWindowEnabled(hwnd: isize) -> i32;
        fn GetWindowThreadProcessId(hwnd: isize, pid: *mut u32) -> u32;
        fn GetWindowLongW(hwnd: isize, index: i32) -> i32;
        fn GetWindow(hwnd: isize, command: u32) -> isize;
        fn GetParent(hwnd: isize) -> isize;
        fn GetDlgCtrlID(hwnd: isize) -> i32;
        fn SendMessageTimeoutW(
            hwnd: isize,
            message: u32,
            wparam: usize,
            lparam: isize,
            flags: u32,
            timeout_ms: u32,
            result: *mut usize,
        ) -> isize;
        fn PostMessageW(hwnd: isize, message: u32, wparam: usize, lparam: isize) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentProcessId() -> u32;
        fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> isize;
        fn Process32FirstW(snapshot: isize, entry: *mut ProcessEntry32W) -> i32;
        fn Process32NextW(snapshot: isize, entry: *mut ProcessEntry32W) -> i32;
        fn CloseHandle(handle: isize) -> i32;
    }

    unsafe extern "system" fn collect(hwnd: isize, into: isize) -> i32 {
        // SAFETY: `into` is the Vec the two callers below pass, alive for the whole call.
        (*(into as *mut Vec<isize>)).push(hwnd);
        1
    }

    fn top_level_windows() -> Vec<isize> {
        let mut windows: Vec<isize> = Vec::new();
        // SAFETY: the callback only pushes onto `windows`, which outlives the call.
        unsafe { EnumWindows(collect, &mut windows as *mut Vec<isize> as isize) };
        windows
    }

    /// Every control inside `parent`, however deeply nested.
    fn controls(parent: isize) -> Vec<isize> {
        let mut windows: Vec<isize> = Vec::new();
        // SAFETY: as in `top_level_windows`.
        unsafe { EnumChildWindows(parent, collect, &mut windows as *mut Vec<isize> as isize) };
        windows
    }

    fn class_name(hwnd: isize) -> String {
        let mut buffer = [0u16; 256];
        // SAFETY: the buffer is as long as the length passed.
        let len = unsafe { GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
        String::from_utf16_lossy(&buffer[..len.max(0) as usize])
    }

    /// A window's or control's text. Asked by message with a timeout: unlike
    /// `GetWindowText` that reads a control in another process too, and a hung window
    /// cannot hold the call.
    fn text_of(hwnd: isize) -> String {
        let mut len: usize = 0;
        // SAFETY: both messages take a caller-owned buffer of the length given, and
        // Windows marshals them across processes.
        unsafe {
            if SendMessageTimeoutW(hwnd, WM_GETTEXTLENGTH, 0, 0, SMTO_ABORTIFHUNG, TEXT_TIMEOUT_MS, &mut len) == 0
                || len == 0
            {
                return String::new();
            }
            let mut buffer = vec![0u16; len.min(MAX_CONTROL_TEXT) + 1];
            let mut copied: usize = 0;
            if SendMessageTimeoutW(
                hwnd,
                WM_GETTEXT,
                buffer.len(),
                buffer.as_mut_ptr() as isize,
                SMTO_ABORTIFHUNG,
                TEXT_TIMEOUT_MS,
                &mut copied,
            ) == 0
            {
                return String::new();
            }
            String::from_utf16_lossy(&buffer[..copied.min(buffer.len() - 1)])
        }
    }

    fn style_of(hwnd: isize) -> u32 {
        // SAFETY: reads one window long; an invalid handle yields 0.
        unsafe { GetWindowLongW(hwnd, GWL_STYLE) as u32 }
    }

    fn pid_of(hwnd: isize) -> u32 {
        let mut pid = 0;
        // SAFETY: writes one u32.
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        pid
    }

    fn process_table() -> Vec<Proc> {
        let mut procs = Vec::new();
        // SAFETY: the entry is sized as Windows requires and the snapshot is closed.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return procs;
            }
            let mut entry: ProcessEntry32W = std::mem::zeroed();
            entry.size = std::mem::size_of::<ProcessEntry32W>() as u32;
            let mut more = Process32FirstW(snapshot, &mut entry);
            while more != 0 {
                let len = entry.exe_file.iter().position(|&c| c == 0).unwrap_or(entry.exe_file.len());
                procs.push(Proc {
                    pid: entry.process_id,
                    ppid: entry.parent_process_id,
                    name: String::from_utf16_lossy(&entry.exe_file[..len]),
                });
                more = Process32NextW(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
        }
        procs
    }

    pub(super) fn find(in_process_only: bool, log: &mut Log) -> Result<Vec<Found<Handle>>, String> {
        // SAFETY: no arguments, no failure mode.
        let me = unsafe { GetCurrentProcessId() };
        let procs = if in_process_only { Vec::new() } else { process_table() };
        let scope = if in_process_only { HashSet::from([me]) } else { in_scope(&procs, me) };
        log.add(|| format!("windows: {} processes, {} in scope", procs.len(), scope.len()));
        Ok(find_in(&scope, me, &procs, log))
    }

    /// The dialogs of the processes in `scope`.
    pub(super) fn find_in(scope: &HashSet<u32>, me: u32, procs: &[Proc], log: &mut Log) -> Vec<Found<Handle>> {
        let mut found = Vec::new();
        for hwnd in top_level_windows() {
            // SAFETY: plain queries on a handle; a window that has gone answers 0.
            let (visible, owner) = unsafe { (IsWindowVisible(hwnd) != 0, GetWindow(hwnd, GW_OWNER)) };
            let pid = pid_of(hwnd);
            if !visible || !scope.contains(&pid) {
                continue;
            }
            let external = pid != me;
            let class = class_name(hwnd);
            if !is_dialog_window(&class, style_of(hwnd), owner != 0, external) {
                continue;
            }

            let mut text = String::new();
            let mut buttons = Vec::new();
            for control in controls(hwnd) {
                // SAFETY: as above. IsWindowVisible is false for a control on a page
                // that is not showing, which is what keeps a wizard's other pages out.
                if unsafe { IsWindowVisible(control) } == 0 {
                    continue;
                }
                let style = style_of(control);
                match class_name(control).to_lowercase().as_str() {
                    "button" if is_push_button(style) => {
                        let label = plain_label(&text_of(control), Some('&'));
                        if !label.is_empty() {
                            buttons.push(FoundButton {
                                label,
                                // SAFETY: as above.
                                enabled: unsafe { IsWindowEnabled(control) } != 0,
                                default: is_default_button(style),
                                handle: control,
                            });
                        }
                    }
                    "static" => append_text(&mut text, &text_of(control)),
                    "syslink" => append_text(&mut text, &strip_links(&text_of(control))),
                    "edit" if style & ES_READONLY != 0 => append_text(&mut text, &text_of(control)),
                    _ => {}
                }
            }

            let process = procs
                .iter()
                .find(|p| p.pid == pid)
                .map(|p| p.name.clone())
                .or_else(|| {
                    std::env::current_exe().ok()?.file_name().map(|n| n.to_string_lossy().into_owned())
                })
                .unwrap_or_default();
            let title = text_of(hwnd);
            log.add(|| format!("windows: {class} {hwnd:#x} pid {pid} '{title}', {} buttons", buttons.len()));
            found.push(Found {
                pid,
                process,
                external,
                token: format!("{hwnd:x}"),
                title,
                text,
                // SAFETY: as above. A window with a modal one over it is disabled.
                blocked: unsafe { IsWindowEnabled(hwnd) } == 0,
                buttons,
            });
        }
        found
    }

    /// Clicks a button the way its dialog hears a click: the `WM_COMMAND` a button
    /// sends its parent. Posted, so a dialog that opens another one in response cannot
    /// hold this call, and it needs neither focus nor the mouse.
    pub(super) fn press(dialog: &Found<Handle>, button: &FoundButton<Handle>, log: &mut Log) -> Result<(), String> {
        let hwnd = button.handle;
        // SAFETY: plain queries and one posted message on handles found a moment ago.
        unsafe {
            let parent = GetParent(hwnd);
            if IsWindow(hwnd) == 0 || parent == 0 {
                return Err(format!("'{}' closed before the button could be pressed.", dialog.title));
            }
            // BN_CLICKED is 0, so the high word of wParam stays clear.
            let control_id = (GetDlgCtrlID(hwnd) as u32 & 0xFFFF) as usize;
            log.add(|| format!("windows: WM_COMMAND id {control_id} to {parent:#x} for button {hwnd:#x}"));
            if PostMessageW(parent, WM_COMMAND, control_id, hwnd) == 0 {
                return Err(format!(
                    "Windows refused the click on '{}' in '{}'.",
                    button.label, dialog.title
                ));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// macOS: windows and their buttons, over the Accessibility API
// ---------------------------------------------------------------------------

/// One line of `ps -axo pid=,ppid=,comm=`: two numbers, then the program, which may
/// itself contain spaces.
#[cfg(any(target_os = "macos", target_os = "freebsd", test))]
fn parse_ps_line(line: &str) -> Option<Proc> {
    let line = line.trim_start();
    let (pid, rest) = line.split_once(char::is_whitespace)?;
    let (ppid, name) = rest.trim_start().split_once(char::is_whitespace)?;
    Some(Proc { pid: pid.parse().ok()?, ppid: ppid.parse().ok()?, name: name.trim().to_string() })
}

/// Whether an accessibility window is a dialog: the system says so, or it is modal.
/// An Eclipse's main window is neither.
#[cfg(any(target_os = "macos", test))]
fn is_ax_dialog(subrole: &str, modal: bool) -> bool {
    modal || matches!(subrole, "AXDialog" | "AXSystemDialog")
}

/// Whether a dialog among `siblings` of one application cannot be clicked: another
/// one is modal and this is not the one in front.
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "freebsd", test))]
fn is_behind_a_modal(index: usize, siblings: &[(bool, bool)]) -> bool {
    let (_, in_front) = siblings[index];
    !in_front && siblings.iter().enumerate().any(|(other, (modal, _))| other != index && *modal)
}

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use std::ffi::{c_char, c_void, CStr};
    use std::process::Command;

    type CfRef = *const c_void;

    const UTF8: u32 = 0x0800_0100;
    const AX_SUCCESS: i32 = 0;
    /// The target did not answer in time: what a button reports when its own handler
    /// opens another modal window before returning.
    const AX_CANNOT_COMPLETE: i32 = -25204;
    /// How long one question to another process may take.
    const MESSAGING_TIMEOUT_S: f32 = 0.5;
    const MAX_DEPTH: usize = 12;
    const MAX_ELEMENTS: usize = 600;

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: CfRef);
        fn CFRetain(cf: CfRef) -> CfRef;
        fn CFEqual(a: CfRef, b: CfRef) -> u8;
        fn CFGetTypeID(cf: CfRef) -> usize;
        fn CFStringGetTypeID() -> usize;
        fn CFArrayGetTypeID() -> usize;
        fn CFBooleanGetTypeID() -> usize;
        fn CFBooleanGetValue(boolean: CfRef) -> u8;
        fn CFArrayGetCount(array: CfRef) -> isize;
        fn CFArrayGetValueAtIndex(array: CfRef, index: isize) -> CfRef;
        fn CFStringCreateWithBytes(allocator: CfRef, bytes: *const u8, len: isize, encoding: u32, external: u8) -> CfRef;
        fn CFStringGetLength(string: CfRef) -> isize;
        fn CFStringGetMaximumSizeForEncoding(len: isize, encoding: u32) -> isize;
        fn CFStringGetCString(string: CfRef, buffer: *mut c_char, size: isize, encoding: u32) -> u8;
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> u8;
        fn AXUIElementCreateApplication(pid: i32) -> CfRef;
        fn AXUIElementCopyAttributeValue(element: CfRef, attribute: CfRef, value: *mut CfRef) -> i32;
        fn AXUIElementPerformAction(element: CfRef, action: CfRef) -> i32;
        fn AXUIElementSetMessagingTimeout(element: CfRef, seconds: f32) -> i32;
    }

    /// An owned Core Foundation object, released when dropped. Never null.
    #[derive(Debug)]
    pub(super) struct Cf(CfRef);

    impl Drop for Cf {
        fn drop(&mut self) {
            // SAFETY: the one reference this value owns.
            unsafe { CFRelease(self.0) };
        }
    }

    impl Cf {
        /// Takes over a reference the caller already owns.
        fn owned(cf: CfRef) -> Option<Cf> {
            (!cf.is_null()).then_some(Cf(cf))
        }

        fn string(text: &str) -> Option<Cf> {
            // SAFETY: the bytes are valid UTF-8 of the length given, and are copied.
            Cf::owned(unsafe {
                CFStringCreateWithBytes(std::ptr::null(), text.as_ptr(), text.len() as isize, UTF8, 0)
            })
        }

        fn as_string(&self) -> Option<String> {
            // SAFETY: the type is checked first, and the buffer is as large as Core
            // Foundation says the conversion can need, plus the terminator.
            unsafe {
                if CFGetTypeID(self.0) != CFStringGetTypeID() {
                    return None;
                }
                let size = CFStringGetMaximumSizeForEncoding(CFStringGetLength(self.0), UTF8) + 1;
                let mut buffer = vec![0 as c_char; size.max(1) as usize];
                if CFStringGetCString(self.0, buffer.as_mut_ptr(), size, UTF8) == 0 {
                    return None;
                }
                Some(CStr::from_ptr(buffer.as_ptr()).to_string_lossy().into_owned())
            }
        }

        fn as_bool(&self) -> Option<bool> {
            // SAFETY: the type is checked first.
            unsafe { (CFGetTypeID(self.0) == CFBooleanGetTypeID()).then(|| CFBooleanGetValue(self.0) != 0) }
        }

        /// The members of an array, each retained so it outlives the array.
        fn members(&self) -> Vec<Cf> {
            // SAFETY: the type is checked first and every index is within the count.
            unsafe {
                if CFGetTypeID(self.0) != CFArrayGetTypeID() {
                    return Vec::new();
                }
                (0..CFArrayGetCount(self.0))
                    .filter_map(|i| {
                        let member = CFArrayGetValueAtIndex(self.0, i);
                        (!member.is_null()).then(|| Cf(CFRetain(member)))
                    })
                    .collect()
            }
        }

        /// An accessibility attribute of this element, or None when it has none.
        fn attribute(&self, name: &str) -> Option<Cf> {
            let name = Cf::string(name)?;
            let mut value: CfRef = std::ptr::null();
            // SAFETY: `value` receives an owned reference only on success.
            let status = unsafe { AXUIElementCopyAttributeValue(self.0, name.0, &mut value) };
            if status != AX_SUCCESS {
                return None;
            }
            Cf::owned(value)
        }

        fn text(&self, attribute: &str) -> String {
            self.attribute(attribute).and_then(|v| v.as_string()).unwrap_or_default()
        }

        fn flag(&self, attribute: &str) -> Option<bool> {
            self.attribute(attribute).and_then(|v| v.as_bool())
        }

        fn list(&self, attribute: &str) -> Vec<Cf> {
            self.attribute(attribute).map(|v| v.members()).unwrap_or_default()
        }
    }

    /// A button's accessibility element.
    pub(super) type Handle = Cf;

    fn process_table() -> Vec<Proc> {
        let Ok(output) = Command::new("/bin/ps").args(["-axo", "pid=,ppid=,comm="]).output() else {
            return Vec::new();
        };
        String::from_utf8_lossy(&output.stdout).lines().filter_map(parse_ps_line).collect()
    }

    pub(super) fn find(in_process_only: bool, log: &mut Log) -> Result<Vec<Found<Handle>>, String> {
        // SAFETY: no arguments. Asks only; it never brings up the system's prompt.
        if unsafe { AXIsProcessTrusted() } == 0 {
            return Err("macOS has not given Eclipse the Accessibility permission, which reading and \
                        pressing these dialogs needs. It is granted in System Settings > Privacy & \
                        Security > Accessibility."
                .to_string());
        }
        let me = std::process::id();
        let procs = if in_process_only { Vec::new() } else { process_table() };
        let scope = if in_process_only { HashSet::from([me]) } else { in_scope(&procs, me) };
        log.add(|| format!("macos: {} processes, {} in scope", procs.len(), scope.len()));

        let mut found = Vec::new();
        let mut pids: Vec<u32> = scope.into_iter().collect();
        pids.sort_unstable();
        for pid in pids {
            let process = procs.iter().find(|p| p.pid == pid).map(|p| image(&p.name)).unwrap_or_default();
            application(pid, pid != me, &process, &mut found, log);
        }
        Ok(found)
    }

    fn application(pid: u32, external: bool, process: &str, found: &mut Vec<Found<Handle>>, log: &mut Log) {
        // SAFETY: creates an element for a process id; null when it cannot.
        let Some(app) = Cf::owned(unsafe { AXUIElementCreateApplication(pid as i32) }) else { return };
        // SAFETY: a valid element and a plain number.
        unsafe { AXUIElementSetMessagingTimeout(app.0, MESSAGING_TIMEOUT_S) };

        let first = found.len();
        let mut standing: Vec<(bool, bool)> = Vec::new();
        for (index, window) in app.list("AXWindows").iter().enumerate() {
            let title = window.text("AXTitle");
            let modal = window.flag("AXModal").unwrap_or(false);
            let in_front = window.flag("AXMain").unwrap_or(false) || window.flag("AXFocused").unwrap_or(false);

            // A sheet is a dialog attached to its window; the window waits on it.
            let mut has_sheet = false;
            for (sheet_index, sheet) in
                window.list("AXChildren").iter().filter(|c| c.text("AXRole") == "AXSheet").enumerate()
            {
                has_sheet = true;
                let token = format!("{index}.{sheet_index}:{title}");
                found.push(dialog(pid, external, process, token, &title, sheet, false));
                standing.push((true, true));
            }
            if is_ax_dialog(&window.text("AXSubrole"), modal) {
                let token = format!("{index}:{title}");
                found.push(dialog(pid, external, process, token, &title, window, has_sheet));
                standing.push((modal, in_front));
            }
        }
        for (index, dialog) in found[first..].iter_mut().enumerate() {
            dialog.blocked = dialog.blocked || is_behind_a_modal(index, &standing);
        }
        log.add(|| format!("macos: pid {pid} ({process}): {} dialogs", found.len() - first));
    }

    fn dialog(
        pid: u32,
        external: bool,
        process: &str,
        token: String,
        title: &str,
        root: &Cf,
        blocked: bool,
    ) -> Found<Handle> {
        let mut text = String::new();
        let mut buttons = Vec::new();
        let default = root.attribute("AXDefaultButton");
        let mut budget = MAX_ELEMENTS;
        walk(root, 0, &mut budget, &mut |element| match element.text("AXRole").as_str() {
            // A title bar's close, minimise and zoom buttons carry a subrole; a
            // dialog's own buttons do not.
            "AXButton" if element.text("AXSubrole").is_empty() => {
                let label = plain_label(&element.text("AXTitle"), None);
                if !label.is_empty() {
                    buttons.push(FoundButton {
                        label,
                        enabled: element.flag("AXEnabled").unwrap_or(true),
                        // SAFETY: two valid references.
                        default: default.as_ref().is_some_and(|d| unsafe { CFEqual(d.0, element.0) } != 0),
                        // SAFETY: a valid reference, retained for the handle to own.
                        handle: Cf(unsafe { CFRetain(element.0) }),
                    });
                }
            }
            "AXStaticText" => append_text(&mut text, &element.text("AXValue")),
            _ => {}
        });
        Found { pid, process: process.to_string(), external, token, title: title.to_string(), text, blocked, buttons }
    }

    /// Visits `element`'s descendants, to a fixed depth and count so that a huge or
    /// cyclic tree cannot hold the call.
    fn walk(element: &Cf, depth: usize, budget: &mut usize, visit: &mut dyn FnMut(&Cf)) {
        if depth >= MAX_DEPTH {
            return;
        }
        for child in element.list("AXChildren") {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            visit(&child);
            walk(&child, depth + 1, budget, visit);
        }
    }

    pub(super) fn press(dialog: &Found<Handle>, button: &FoundButton<Handle>, log: &mut Log) -> Result<(), String> {
        let action = Cf::string("AXPress").ok_or("macOS could not name the press action.")?;
        // SAFETY: two valid references; the timeout bounds the wait for the answer.
        let status = unsafe {
            AXUIElementSetMessagingTimeout(button.handle.0, MESSAGING_TIMEOUT_S);
            AXUIElementPerformAction(button.handle.0, action.0)
        };
        log.add(|| format!("macos: AXPress '{}' in '{}' → {status}", button.label, dialog.title));
        match status {
            AX_SUCCESS | AX_CANNOT_COMPLETE => Ok(()),
            other => Err(format!(
                "macOS refused the press of '{}' in '{}' (accessibility error {other}).",
                button.label, dialog.title
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Linux and FreeBSD: windows and their buttons, over the accessibility bus (AT-SPI)
// ---------------------------------------------------------------------------

// Role and state numbers from at-spi2-core's atspi-constants.h.
#[cfg(any(target_os = "linux", target_os = "freebsd", test))]
const ATSPI_ROLE_ALERT: u32 = 2;
#[cfg(any(target_os = "linux", target_os = "freebsd", test))]
const ATSPI_ROLE_COLOR_CHOOSER: u32 = 9;
#[cfg(any(target_os = "linux", target_os = "freebsd", test))]
const ATSPI_ROLE_DIALOG: u32 = 16;
#[cfg(any(target_os = "linux", target_os = "freebsd", test))]
const ATSPI_ROLE_FILE_CHOOSER: u32 = 19;
#[cfg(any(target_os = "linux", target_os = "freebsd", test))]
const ATSPI_ROLE_FONT_CHOOSER: u32 = 22;
#[cfg(any(target_os = "linux", target_os = "freebsd", test))]
const ATSPI_ROLE_FRAME: u32 = 23;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const ATSPI_ROLE_LABEL: u32 = 29;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const ATSPI_ROLE_PUSH_BUTTON: u32 = 43;

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const ATSPI_STATE_ACTIVE: u32 = 1;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const ATSPI_STATE_ENABLED: u32 = 8;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const ATSPI_STATE_MODAL: u32 = 16;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const ATSPI_STATE_SENSITIVE: u32 = 24;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const ATSPI_STATE_SHOWING: u32 = 25;
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const ATSPI_STATE_IS_DEFAULT: u32 = 39;

/// Whether a top-level accessible window is a dialog. GTK's own dialogs carry a role
/// that says so. An SWT shell is a plain frame whatever it is used for, so one of
/// those counts only when it is modal, which an Eclipse's main window never is.
#[cfg(any(target_os = "linux", target_os = "freebsd", test))]
fn is_atspi_dialog(role: u32, modal: bool) -> bool {
    match role {
        ATSPI_ROLE_ALERT
        | ATSPI_ROLE_DIALOG
        | ATSPI_ROLE_FILE_CHOOSER
        | ATSPI_ROLE_COLOR_CHOOSER
        | ATSPI_ROLE_FONT_CHOOSER => true,
        ATSPI_ROLE_FRAME => modal,
        _ => false,
    }
}

/// Whether state number `state` is set in a state set as the bus sends it: 32 states
/// to a word, lowest first.
#[cfg(any(target_os = "linux", target_os = "freebsd", test))]
fn has_state(states: &[u32], state: u32) -> bool {
    states.get((state / 32) as usize).is_some_and(|word| word & (1 << (state % 32)) != 0)
}

/// A process from its `/proc/<pid>/stat`: `pid (name) state ppid …`. The name may
/// itself contain spaces and parentheses, so it ends at the last `)`.
#[cfg(any(target_os = "linux", test))]
fn parse_proc_stat(stat: &str) -> Option<Proc> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let pid = stat[..open].trim().parse().ok()?;
    let ppid = stat.get(close + 1..)?.split_whitespace().nth(1)?.parse().ok()?;
    Some(Proc { pid, ppid, name: stat.get(open + 1..close)?.to_string() })
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod atspi {
    use super::*;
    use std::time::Duration;

    use zbus::blocking::{connection, Connection};
    use zbus::zvariant::{OwnedObjectPath, OwnedValue};

    const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
    const ACTION: &str = "org.a11y.atspi.Action";
    const REGISTRY: &str = "org.a11y.atspi.Registry";
    const ROOT: &str = "/org/a11y/atspi/accessible/root";
    /// How long one question to another process may take.
    const CALL_TIMEOUT: Duration = Duration::from_millis(1000);
    const MAX_DEPTH: usize = 14;
    const MAX_NODES: usize = 800;

    /// An accessible object: the bus name of its application, and its path there.
    #[derive(Debug, Clone)]
    pub(super) struct Node {
        bus: String,
        path: String,
    }

    /// A button's accessible object.
    pub(super) type Handle = Node;

    struct Bus(Connection);

    impl Bus {
        /// Connects to the accessibility bus, whose address the session bus hands out.
        fn open() -> Result<Bus, String> {
            let session = Connection::session()
                .map_err(|e| format!("There is no session D-Bus to reach the accessibility service through ({e})."))?;
            let address: String = session
                .call_method(Some("org.a11y.Bus"), "/org/a11y/bus", Some("org.a11y.Bus"), "GetAddress", &())
                .and_then(|reply| reply.body().deserialize())
                .map_err(|e| format!("The accessibility service (at-spi2-core) is not available ({e})."))?;
            let bus = connection::Builder::address(address.as_str())
                .and_then(|builder| builder.method_timeout(CALL_TIMEOUT).build())
                .map_err(|e| format!("The accessibility bus could not be reached ({e})."))?;
            Ok(Bus(bus))
        }

        fn children(&self, node: &Node) -> Vec<Node> {
            let reply: zbus::Result<Vec<(String, OwnedObjectPath)>> = self
                .0
                .call_method(Some(node.bus.as_str()), node.path.as_str(), Some(ACCESSIBLE), "GetChildren", &())
                .and_then(|reply| reply.body().deserialize());
            reply
                .unwrap_or_default()
                .into_iter()
                .map(|(bus, path)| Node { bus, path: path.as_str().to_string() })
                .collect()
        }

        fn role(&self, node: &Node) -> u32 {
            self.0
                .call_method(Some(node.bus.as_str()), node.path.as_str(), Some(ACCESSIBLE), "GetRole", &())
                .and_then(|reply| reply.body().deserialize())
                .unwrap_or(0)
        }

        fn states(&self, node: &Node) -> Vec<u32> {
            self.0
                .call_method(Some(node.bus.as_str()), node.path.as_str(), Some(ACCESSIBLE), "GetState", &())
                .and_then(|reply| reply.body().deserialize())
                .unwrap_or_default()
        }

        fn name(&self, node: &Node) -> String {
            let value: zbus::Result<OwnedValue> = self
                .0
                .call_method(
                    Some(node.bus.as_str()),
                    node.path.as_str(),
                    Some("org.freedesktop.DBus.Properties"),
                    "Get",
                    &(ACCESSIBLE, "Name"),
                )
                .and_then(|reply| reply.body().deserialize());
            value.ok().and_then(|v| String::try_from(v).ok()).unwrap_or_default()
        }

        /// The process behind a bus name, as the bus itself knows it.
        fn pid(&self, bus_name: &str) -> Option<u32> {
            self.0
                .call_method(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    Some("org.freedesktop.DBus"),
                    "GetConnectionUnixProcessID",
                    &(bus_name,),
                )
                .and_then(|reply| reply.body().deserialize())
                .ok()
        }

        /// Asks a button to do its first action, which for a button is the click. Sent
        /// without waiting for the answer: a button whose handler opens another modal
        /// dialog only answers once that one has closed.
        fn click(&self, node: &Node) -> zbus::Result<()> {
            let message = zbus::Message::method_call(node.path.as_str(), "DoAction")?
                .destination(node.bus.as_str())?
                .interface(ACTION)?
                .with_flags(zbus::message::Flags::NoReplyExpected)?
                .build(&(0i32,))?;
            self.0.send(&message)
        }
    }

    /// The dialogs of the processes in `scope`.
    pub(super) fn find(
        scope: &HashSet<u32>,
        me: u32,
        procs: &[Proc],
        log: &mut Log,
    ) -> Result<Vec<Found<Handle>>, String> {
        let bus = Bus::open()?;
        let mut found = Vec::new();
        let desktop = Node { bus: REGISTRY.to_string(), path: ROOT.to_string() };
        for app in bus.children(&desktop) {
            // Whose application this is comes first: nothing outside the scope is read.
            let Some(pid) = bus.pid(&app.bus) else { continue };
            if !scope.contains(&pid) {
                continue;
            }
            let process = procs.iter().find(|p| p.pid == pid).map(|p| image(&p.name)).unwrap_or_default();
            let first = found.len();
            let mut standing: Vec<(bool, bool)> = Vec::new();
            for window in bus.children(&app) {
                let states = bus.states(&window);
                let modal = has_state(&states, ATSPI_STATE_MODAL);
                if !has_state(&states, ATSPI_STATE_SHOWING) || !is_atspi_dialog(bus.role(&window), modal) {
                    continue;
                }
                found.push(dialog(&bus, pid, pid != me, &process, &window));
                standing.push((modal, has_state(&states, ATSPI_STATE_ACTIVE)));
            }
            for (index, dialog) in found[first..].iter_mut().enumerate() {
                dialog.blocked = is_behind_a_modal(index, &standing);
            }
            log.add(|| format!("atspi: {} pid {pid} ({process}): {} dialogs", app.bus, found.len() - first));
        }
        Ok(found)
    }

    fn dialog(bus: &Bus, pid: u32, external: bool, process: &str, window: &Node) -> Found<Handle> {
        let mut text = String::new();
        let mut buttons = Vec::new();
        let mut budget = MAX_NODES;
        walk(bus, window, 0, &mut budget, &mut |node| {
            let role = bus.role(node);
            if role != ATSPI_ROLE_PUSH_BUTTON && role != ATSPI_ROLE_LABEL {
                return;
            }
            let states = bus.states(node);
            if !has_state(&states, ATSPI_STATE_SHOWING) {
                return;
            }
            // An accessible name is the label as read out: it carries no mnemonic marker.
            let name = plain_label(&bus.name(node), None);
            if role == ATSPI_ROLE_LABEL {
                append_text(&mut text, &name);
            } else if !name.is_empty() {
                buttons.push(FoundButton {
                    label: name,
                    enabled: has_state(&states, ATSPI_STATE_ENABLED) && has_state(&states, ATSPI_STATE_SENSITIVE),
                    default: has_state(&states, ATSPI_STATE_IS_DEFAULT),
                    handle: node.clone(),
                });
            }
        });
        Found {
            pid,
            process: process.to_string(),
            external,
            token: format!("{}|{}", window.bus, window.path),
            title: bus.name(window),
            text,
            blocked: false,
            buttons,
        }
    }

    /// Visits `node`'s descendants, to a fixed depth and count so that a huge tree (a
    /// file chooser's listing) cannot hold the call.
    fn walk(bus: &Bus, node: &Node, depth: usize, budget: &mut usize, visit: &mut dyn FnMut(&Node)) {
        if depth >= MAX_DEPTH {
            return;
        }
        for child in bus.children(node) {
            if *budget == 0 {
                return;
            }
            *budget -= 1;
            visit(&child);
            walk(bus, &child, depth + 1, budget, visit);
        }
    }

    pub(super) fn press(dialog: &Found<Handle>, button: &FoundButton<Handle>, log: &mut Log) -> Result<(), String> {
        let bus = Bus::open()?;
        log.add(|| format!("atspi: DoAction(0) on {} {}", button.handle.bus, button.handle.path));
        bus.click(&button.handle).map_err(|e| {
            format!("The press of '{}' in '{}' could not be sent ({e}).", button.label, dialog.title)
        })
    }
}

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod imp {
    use super::*;

    pub(super) type Handle = atspi::Handle;

    fn process_table() -> Vec<Proc> {
        let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
        entries
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
            .filter_map(|entry| std::fs::read_to_string(entry.path().join("stat")).ok())
            .filter_map(|stat| parse_proc_stat(&stat))
            .collect()
    }

    pub(super) fn find(in_process_only: bool, log: &mut Log) -> Result<Vec<Found<Handle>>, String> {
        let me = std::process::id();
        let procs = if in_process_only { Vec::new() } else { process_table() };
        let scope = if in_process_only { HashSet::from([me]) } else { in_scope(&procs, me) };
        log.add(|| format!("linux: {} processes, {} in scope", procs.len(), scope.len()));
        atspi::find(&scope, me, &procs, log)
    }

    pub(super) fn press(dialog: &Found<Handle>, button: &FoundButton<Handle>, log: &mut Log) -> Result<(), String> {
        atspi::press(dialog, button, log)
    }
}

// ---------------------------------------------------------------------------
// FreeBSD
// ---------------------------------------------------------------------------

#[cfg(target_os = "freebsd")]
mod imp {
    use super::*;
    use std::process::Command;

    pub(super) type Handle = atspi::Handle;

    /// From `ps`: FreeBSD has no `/proc` unless one is mounted by hand.
    fn process_table() -> Vec<Proc> {
        let Ok(output) = Command::new("/bin/ps").args(["-axo", "pid=,ppid=,comm="]).output() else {
            return Vec::new();
        };
        String::from_utf8_lossy(&output.stdout).lines().filter_map(parse_ps_line).collect()
    }

    pub(super) fn find(in_process_only: bool, log: &mut Log) -> Result<Vec<Found<Handle>>, String> {
        let me = std::process::id();
        let procs = if in_process_only { Vec::new() } else { process_table() };
        let scope = if in_process_only { HashSet::from([me]) } else { in_scope(&procs, me) };
        log.add(|| format!("freebsd: {} processes, {} in scope", procs.len(), scope.len()));
        atspi::find(&scope, me, &procs, log)
    }

    pub(super) fn press(dialog: &Found<Handle>, button: &FoundButton<Handle>, log: &mut Log) -> Result<(), String> {
        atspi::press(dialog, button, log)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn proc(pid: u32, ppid: u32, name: &str) -> Proc {
        Proc { pid, ppid, name: name.to_string() }
    }

    fn scope(procs: &[Proc], me: u32) -> Vec<u32> {
        let mut pids: Vec<u32> = in_scope(procs, me).into_iter().collect();
        pids.sort_unstable();
        pids
    }

    fn button(label: &str, enabled: bool) -> FoundButton<u8> {
        FoundButton { label: label.to_string(), enabled, default: false, handle: 0 }
    }

    fn dialog(pid: u32, token: &str, title: &str, buttons: Vec<FoundButton<u8>>) -> Found<u8> {
        Found {
            pid,
            process: "eclipse".to_string(),
            external: true,
            token: token.to_string(),
            title: title.to_string(),
            text: String::new(),
            blocked: false,
            buttons,
        }
    }

    // --- program names -----------------------------------------------------

    #[test]
    fn a_program_name_is_compared_without_folder_extension_or_case() {
        assert_eq!(image(r"C:\Program Files\Eclipse\Eclipse.EXE"), "eclipse");
        assert_eq!(image("/Applications/Eclipse.app/Contents/MacOS/eclipse"), "eclipse");
        assert_eq!(image("javaw.exe"), "javaw");
        assert_eq!(image("java"), "java");
    }

    #[test]
    fn eclipse_and_java_programs_are_told_apart_from_everything_else() {
        assert!(is_eclipse_image("eclipse.exe"));
        assert!(is_eclipse_image("eclipsec.exe"));
        assert!(is_eclipse_image("/usr/lib/eclipse/eclipse"));
        assert!(!is_eclipse_image("eclipse-installer.exe"));
        assert!(is_java_image("javaw.exe"));
        assert!(is_java_image("/usr/lib/jvm/bin/java"));
        assert!(!is_java_image("javac.exe"));
        assert!(!is_eclipse_image("explorer.exe"));
    }

    // --- scope -------------------------------------------------------------

    #[test]
    fn the_eclipse_this_one_started_is_in_scope() {
        let procs = [proc(1, 0, "explorer.exe"), proc(10, 1, "eclipse.exe"), proc(20, 10, "javaw.exe")];
        assert_eq!(scope(&procs, 10), vec![10, 20]);
    }

    #[test]
    fn the_eclipse_that_started_this_one_is_in_scope() {
        let procs = [proc(1, 0, "explorer.exe"), proc(10, 1, "eclipse.exe"), proc(20, 10, "javaw.exe")];
        assert_eq!(scope(&procs, 20), vec![10, 20]);
    }

    #[test]
    fn whatever_started_eclipse_is_not_in_scope() {
        let procs = [
            proc(1, 0, "systemd"),
            proc(5, 1, "bash"),
            proc(10, 5, "java"),
            proc(20, 10, "java"),
        ];
        // 20 is this process, 10 the Eclipse that launched it; the shell and init are not.
        assert_eq!(scope(&procs, 20), vec![10, 20]);
    }

    #[test]
    fn an_eclipse_started_separately_is_in_scope() {
        let procs = [proc(10, 1, "eclipse.exe"), proc(30, 1, "eclipse.exe"), proc(40, 1, "notepad.exe")];
        assert_eq!(scope(&procs, 10), vec![10, 30]);
    }

    #[test]
    fn a_java_program_under_another_eclipse_is_in_scope() {
        let procs = [proc(10, 1, "eclipse.exe"), proc(30, 1, "eclipse.exe"), proc(31, 30, "javaw.exe")];
        assert_eq!(scope(&procs, 10), vec![10, 30, 31]);
    }

    #[test]
    fn a_java_program_started_through_a_shell_is_in_scope_but_the_shell_is_not() {
        let procs = [proc(10, 1, "eclipse.exe"), proc(11, 10, "cmd.exe"), proc(12, 11, "java.exe")];
        assert_eq!(scope(&procs, 10), vec![10, 12]);
    }

    #[test]
    fn other_programs_eclipse_started_are_not_in_scope() {
        let procs = [
            proc(10, 1, "eclipse.exe"),
            proc(11, 10, "node.exe"),
            proc(12, 10, "msedgewebview2.exe"),
            proc(13, 10, "git.exe"),
        ];
        assert_eq!(scope(&procs, 10), vec![10]);
    }

    #[test]
    fn a_java_program_unrelated_to_any_eclipse_is_not_in_scope() {
        let procs = [proc(10, 1, "eclipse.exe"), proc(50, 1, "javaw.exe")];
        assert_eq!(scope(&procs, 10), vec![10]);
    }

    #[test]
    fn a_process_table_that_loops_still_ends() {
        // A parent id can outlive its process and be handed to a newer one.
        let procs = [proc(10, 20, "java"), proc(20, 10, "java")];
        assert_eq!(scope(&procs, 10), vec![10, 20]);
    }

    #[test]
    fn this_process_is_in_scope_even_when_the_table_does_not_list_it() {
        assert_eq!(scope(&[], 77), vec![77]);
    }

    // --- labels ------------------------------------------------------------

    #[test]
    fn a_mnemonic_marker_is_not_part_of_the_label() {
        assert_eq!(plain_label("&Yes", Some('&')), "Yes");
        assert_eq!(plain_label("Do&n't Save", Some('&')), "Don't Save");
        assert_eq!(plain_label("_Cancel", Some('_')), "Cancel");
    }

    #[test]
    fn a_doubled_marker_is_a_literal_one() {
        assert_eq!(plain_label("Save && Close", Some('&')), "Save & Close");
        assert_eq!(plain_label("snake__case", Some('_')), "snake_case");
    }

    #[test]
    fn a_label_without_a_marker_convention_is_only_trimmed() {
        assert_eq!(plain_label("  Save & Close ", None), "Save & Close");
    }

    #[test]
    fn a_press_means_the_one_button_with_that_label_whatever_its_case() {
        let labels = vec!["Proceed".to_string(), "Cancel".to_string()];
        assert_eq!(pick(&labels, "cancel"), Pick::One(1));
        assert_eq!(pick(&labels, " Proceed "), Pick::One(0));
    }

    #[test]
    fn part_of_a_label_matches_nothing() {
        let labels = vec!["Proceed".to_string(), "Cancel".to_string()];
        assert_eq!(pick(&labels, "Proc"), Pick::None);
        assert_eq!(pick(&labels, ""), Pick::None);
    }

    #[test]
    fn two_buttons_with_the_same_label_are_ambiguous() {
        let labels = vec!["Browse...".to_string(), "Browse...".to_string()];
        assert_eq!(pick(&labels, "Browse..."), Pick::Many);
    }

    // --- ids ---------------------------------------------------------------

    #[test]
    fn an_id_carries_the_process_and_the_platform_handle_back() {
        let id = make_id(4242, ":1.42/org/a11y/atspi/accessible/7");
        assert!(id.starts_with("os-4242-"), "{id}");
        assert_eq!(parse_id(&id), Some((4242, ":1.42/org/a11y/atspi/accessible/7".to_string())));
    }

    #[test]
    fn an_id_is_plain_enough_to_pass_through_any_client() {
        let id = make_id(7, "a b\"c\\d-é");
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'), "{id}");
        assert_eq!(parse_id(&id), Some((7, "a b\"c\\d-é".to_string())));
    }

    #[test]
    fn something_that_is_not_one_of_these_ids_is_refused() {
        assert_eq!(parse_id("swt-1a2b"), None);
        assert_eq!(parse_id("os-"), None);
        assert_eq!(parse_id("os-12"), None);
        assert_eq!(parse_id("os-x-6162"), None);
        assert_eq!(parse_id("os-12-zz"), None);
        assert_eq!(parse_id("os-12-616"), None);
    }

    // --- listing -----------------------------------------------------------

    #[test]
    fn a_dialog_is_listed_with_its_id_text_buttons_and_process() {
        let mut d = dialog(31, "h1", "Save Resource", vec![button("Save", true), button("Cancel", true)]);
        d.text = "'A.java' has been modified.".to_string();
        d.buttons[0].default = true;
        let listed = describe(&[d]);
        assert_eq!(
            listed,
            json!([{
                "id": make_id(31, "h1"),
                "title": "Save Resource",
                "text": "'A.java' has been modified.",
                "buttons": [{ "label": "Save", "default": true }, { "label": "Cancel" }],
                "process": { "pid": 31, "name": "eclipse" },
                "external": true
            }])
        );
    }

    #[test]
    fn what_cannot_be_pressed_is_listed_as_such() {
        let mut d = dialog(31, "h1", "Wizard", vec![button("Finish", false)]);
        d.blocked = true;
        d.external = false;
        let listed = describe(&[d]);
        assert_eq!(listed[0]["blocked"], json!(true));
        assert_eq!(listed[0]["buttons"][0], json!({ "label": "Finish", "enabled": false }));
        assert_eq!(listed[0]["external"], json!(false));
        assert!(listed[0].get("text").is_none(), "no text, no key");
    }

    // --- choosing a button -------------------------------------------------

    #[test]
    fn the_named_button_of_the_named_dialog_is_chosen() {
        let found = [
            dialog(31, "h1", "First", vec![button("OK", true)]),
            dialog(31, "h2", "Second", vec![button("OK", true), button("Cancel", true)]),
        ];
        let (d, b) = choose(&found, &make_id(31, "h2"), "cancel").unwrap();
        assert_eq!((d.title.as_str(), b.label.as_str()), ("Second", "Cancel"));
    }

    #[test]
    fn a_dialog_that_is_gone_is_reported() {
        let found = [dialog(31, "h1", "First", vec![button("OK", true)])];
        let err = choose(&found, &make_id(31, "h9"), "OK").unwrap_err();
        assert!(err.contains("No open dialog has id"), "{err}");
    }

    #[test]
    fn an_id_from_the_swt_side_is_reported_as_not_one_of_these() {
        let found = [dialog(31, "h1", "First", vec![button("OK", true)])];
        let err = choose(&found, "swt-1a2b", "OK").unwrap_err();
        assert!(err.contains("No open dialog has id"), "{err}");
    }

    #[test]
    fn a_missing_button_is_reported_with_the_ones_there_are() {
        let found = [dialog(31, "h1", "Confirm", vec![button("Yes", true), button("No", true)])];
        let err = choose(&found, &make_id(31, "h1"), "OK").unwrap_err();
        assert_eq!(err, "'Confirm' has no button labelled 'OK'. Its buttons: Yes, No");
    }

    #[test]
    fn an_ambiguous_label_presses_nothing() {
        let found = [dialog(31, "h1", "Paths", vec![button("Browse...", true), button("Browse...", true)])];
        let err = choose(&found, &make_id(31, "h1"), "Browse...").unwrap_err();
        assert_eq!(err, "More than one button in 'Paths' is labelled 'Browse...'.");
    }

    #[test]
    fn a_disabled_button_is_not_pressed() {
        let found = [dialog(31, "h1", "Wizard", vec![button("Finish", false)])];
        let err = choose(&found, &make_id(31, "h1"), "Finish").unwrap_err();
        assert_eq!(err, "The 'Finish' button in 'Wizard' is disabled.");
    }

    #[test]
    fn a_dialog_behind_another_modal_window_is_not_pressed() {
        let mut d = dialog(31, "h1", "Preferences", vec![button("Apply", true)]);
        d.blocked = true;
        let err = choose(&[d], &make_id(31, "h1"), "Apply").unwrap_err();
        assert_eq!(err, "'Preferences' is waiting on another window that is open over it. Answer that one first.");
    }
}

/// What decides which Windows windows and controls count. Plain logic, so it runs on
/// every platform.
#[cfg(test)]
mod windows_rule_tests {
    use super::*;

    const DIALOG_FRAME: u32 = WS_CAPTION | 0x0008_0000;
    const MAIN_WINDOW: u32 = WS_CAPTION | WS_MINIMIZEBOX | 0x0001_0000;

    #[test]
    fn a_win32_dialog_box_is_a_dialog_in_any_process() {
        assert!(is_dialog_window("#32770", 0, false, false));
        assert!(is_dialog_window("#32770", 0, false, true));
    }

    #[test]
    fn another_eclipses_swt_dialog_is_a_dialog() {
        assert!(is_dialog_window("SWT_Window0", DIALOG_FRAME, true, true));
        // A dialog shown before any main window exists has no owner.
        assert!(is_dialog_window("SWT_Window0", DIALOG_FRAME, false, true));
    }

    #[test]
    fn another_eclipses_main_window_is_not_a_dialog() {
        assert!(!is_dialog_window("SWT_Window0", MAIN_WINDOW, false, true));
    }

    #[test]
    fn this_eclipses_own_swt_shells_are_left_to_java() {
        assert!(!is_dialog_window("SWT_Window0", DIALOG_FRAME, true, false));
    }

    #[test]
    fn a_window_without_a_title_bar_or_of_another_kind_is_not_a_dialog() {
        assert!(!is_dialog_window("SWT_Window0", 0, true, true), "a hover or a popup");
        assert!(!is_dialog_window("Chrome_WidgetWin_1", DIALOG_FRAME, true, true));
    }

    #[test]
    fn only_buttons_that_act_when_clicked_are_push_buttons() {
        for push in [0x0, 0x1, 0xC, 0xD, 0xE, 0xF] {
            assert!(is_push_button(0x5001_0000 | push), "{push:#x}");
        }
        for other in [0x2, 0x3, 0x4, 0x7, 0x9, 0xB] {
            assert!(!is_push_button(0x5001_0000 | other), "{other:#x}");
        }
        assert!(is_default_button(0x5001_0001));
        assert!(!is_default_button(0x5001_0000));
    }

    #[test]
    fn a_links_markup_is_not_part_of_its_text() {
        assert_eq!(strip_links("See <a>the log</a> for details."), "See the log for details.");
        assert_eq!(strip_links(r#"<A HREF="x">Open</A>"#), "Open");
        assert_eq!(strip_links("1 < 2 and a<b"), "1 < 2 and a<b");
    }
}

/// Against real windows. They open a message box on the desktop, so they are run by
/// hand: `cargo test dialogs::windows_tests -- --ignored`.
#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use std::process::{Child, Command};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    const MB_OKCANCEL: u32 = 0x1;
    const IDCANCEL: i32 = 2;

    #[link(name = "user32")]
    extern "system" {
        fn MessageBoxW(owner: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Polls `look` until it finds something, for as long as a slow machine may need.
    fn eventually<T>(mut look: impl FnMut() -> Option<T>) -> T {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(found) = look() {
                return found;
            }
            assert!(Instant::now() < deadline, "the message box never appeared");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    struct KillOnDrop(Child);

    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
        }
    }

    #[test]
    #[ignore = "opens a real message box"]
    fn a_message_box_in_this_process_is_listed_and_its_named_button_pressed() {
        let title = format!("claude-eclipse-core in-process {}", std::process::id());
        let (answer, answered) = mpsc::channel();
        let caption = title.clone();
        std::thread::spawn(move || {
            // SAFETY: both strings are NUL-terminated and outlive the call.
            let pressed = unsafe {
                MessageBoxW(0, wide("Proceed with the launch?").as_ptr(), wide(&caption).as_ptr(), MB_OKCANCEL)
            };
            let _ = answer.send(pressed);
        });

        let dialog = eventually(|| {
            let listed: Value = serde_json::from_str(&list_json(true)).unwrap();
            listed["dialogs"].as_array().unwrap().iter().find(|d| d["title"] == json!(title)).cloned()
        });
        assert_eq!(dialog["text"], json!("Proceed with the launch?"));
        assert_eq!(dialog["external"], json!(false));
        assert_eq!(dialog["process"]["pid"], json!(std::process::id()));
        let buttons = dialog["buttons"].as_array().unwrap();
        assert_eq!(buttons.len(), 2, "{dialog}");
        assert_eq!(buttons[0]["default"], json!(true), "{dialog}");

        // The second button is Cancel in whatever language Windows runs in.
        let cancel = buttons[1]["label"].as_str().unwrap();
        let reply: Value = serde_json::from_str(&press_json(dialog["id"].as_str().unwrap(), cancel)).unwrap();
        assert_eq!(reply, json!({ "pressed": cancel, "dialog": title }));
        assert_eq!(answered.recv_timeout(Duration::from_secs(10)), Ok(IDCANCEL));
    }

    #[test]
    #[ignore = "opens a real message box"]
    fn a_button_that_is_not_named_in_full_presses_nothing() {
        let title = format!("claude-eclipse-core refusal {}", std::process::id());
        let (answer, answered) = mpsc::channel();
        let caption = title.clone();
        std::thread::spawn(move || {
            // SAFETY: as above.
            let pressed =
                unsafe { MessageBoxW(0, wide("Still here?").as_ptr(), wide(&caption).as_ptr(), MB_OKCANCEL) };
            let _ = answer.send(pressed);
        });

        let dialog = eventually(|| {
            let listed: Value = serde_json::from_str(&list_json(true)).unwrap();
            listed["dialogs"].as_array().unwrap().iter().find(|d| d["title"] == json!(title)).cloned()
        });
        let id = dialog["id"].as_str().unwrap();
        let reply: Value = serde_json::from_str(&press_json(id, "no such button")).unwrap();
        assert!(reply["error"].as_str().unwrap().contains("has no button labelled 'no such button'"), "{reply}");
        assert!(answered.recv_timeout(Duration::from_millis(500)).is_err(), "the box is still open");

        let first = dialog["buttons"][0]["label"].as_str().unwrap();
        press_json(id, first);
        assert!(answered.recv_timeout(Duration::from_secs(10)).is_ok());
    }

    #[test]
    #[ignore = "opens a real message box"]
    fn a_message_box_in_another_process_is_listed_and_its_named_button_pressed() {
        let title = format!("claude-eclipse-core cross-process {}", std::process::id());
        let script = format!(
            "Add-Type -AssemblyName PresentationFramework; \
             $r = [System.Windows.MessageBox]::Show('Workspace in use.', '{title}', 'OKCancel'); \
             exit [int]$r"
        );
        let mut child = KillOnDrop(
            Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                .spawn()
                .expect("powershell starts"),
        );
        let scope = HashSet::from([child.0.id()]);
        let me = std::process::id();

        let found = eventually(|| {
            let found = imp::find_in(&scope, me, &[], &mut Log::default());
            found.iter().any(|d| d.title == title).then_some(found)
        });
        let dialog = found.iter().find(|d| d.title == title).unwrap();
        assert!(dialog.external);
        assert_eq!(dialog.text, "Workspace in use.");
        assert_eq!(dialog.buttons.len(), 2);

        let cancel = dialog.buttons[1].label.clone();
        let (dialog, button) = choose(&found, &make_id(dialog.pid, &dialog.token), &cancel).unwrap();
        imp::press(dialog, button, &mut Log::default()).unwrap();

        let deadline = Instant::now() + Duration::from_secs(15);
        let status = loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "the other process never saw the click");
            std::thread::sleep(Duration::from_millis(100));
        };
        // System.Windows.MessageBoxResult: OK = 1, Cancel = 2.
        assert_eq!(status.code(), Some(2));
    }

    #[test]
    fn a_process_outside_the_scope_is_never_looked_at() {
        let nobody = HashSet::new();
        assert!(imp::find_in(&nobody, std::process::id(), &[], &mut Log::default()).is_empty());
    }
}

/// What decides which macOS, Linux and FreeBSD windows count, and how their process
/// tables read. Plain logic, so it runs on every platform.
#[cfg(test)]
mod platform_rule_tests {
    use super::*;

    #[test]
    fn a_ps_line_reads_as_a_process() {
        assert_eq!(
            parse_ps_line("  812     1 /usr/local/bin/eclipse"),
            Some(Proc { pid: 812, ppid: 1, name: "/usr/local/bin/eclipse".to_string() })
        );
    }

    #[test]
    fn a_program_path_with_spaces_stays_whole() {
        let line = "70123 70001 /Applications/Eclipse IDE.app/Contents/MacOS/eclipse";
        let parsed = parse_ps_line(line).unwrap();
        assert_eq!((parsed.pid, parsed.ppid), (70123, 70001));
        assert_eq!(parsed.name, "/Applications/Eclipse IDE.app/Contents/MacOS/eclipse");
        assert!(is_eclipse_image(&parsed.name));
    }

    #[test]
    fn a_ps_line_that_is_not_a_process_is_skipped() {
        assert_eq!(parse_ps_line(""), None);
        assert_eq!(parse_ps_line("  PID  PPID COMM"), None);
        assert_eq!(parse_ps_line("12 34"), None);
    }

    #[test]
    fn a_proc_stat_line_reads_as_a_process() {
        let stat = "4242 (java) S 4100 4242 4100 0 -1 4194304 1 0";
        assert_eq!(parse_proc_stat(stat), Some(Proc { pid: 4242, ppid: 4100, name: "java".to_string() }));
    }

    #[test]
    fn a_program_name_with_spaces_and_brackets_does_not_shift_the_parent_id() {
        let stat = "77 (Web Content (x) S 9) R 55 77 55 0";
        assert_eq!(parse_proc_stat(stat), Some(Proc { pid: 77, ppid: 55, name: "Web Content (x) S 9".to_string() }));
    }

    #[test]
    fn a_proc_stat_line_that_is_cut_short_is_skipped() {
        assert_eq!(parse_proc_stat(""), None);
        assert_eq!(parse_proc_stat("12 (java"), None);
        assert_eq!(parse_proc_stat("12 (java) S"), None);
    }

    #[test]
    fn a_macos_window_is_a_dialog_when_the_system_says_so_or_it_is_modal() {
        assert!(is_ax_dialog("AXDialog", false));
        assert!(is_ax_dialog("AXSystemDialog", false));
        assert!(is_ax_dialog("AXStandardWindow", true));
        assert!(!is_ax_dialog("AXStandardWindow", false), "an Eclipse's main window");
        assert!(!is_ax_dialog("", false));
    }

    #[test]
    fn an_accessible_window_is_a_dialog_by_its_role_and_a_plain_frame_only_when_modal() {
        for role in [2, 9, 16, 19, 22] {
            assert!(is_atspi_dialog(role, false), "role {role}");
        }
        assert!(is_atspi_dialog(23, true), "a modal SWT shell");
        assert!(!is_atspi_dialog(23, false), "an Eclipse's main window");
        assert!(!is_atspi_dialog(43, true), "a button is not a window");
    }

    #[test]
    fn a_state_is_read_from_the_word_that_holds_it() {
        // Enabled (8), modal (16) and sensitive (24) in the first word; default (39) in the second.
        let states = [1 << 8 | 1 << 16 | 1 << 24, 1 << 7];
        for set in [8, 16, 24, 39] {
            assert!(has_state(&states, set), "state {set}");
        }
        for clear in [0, 7, 25, 32, 38, 40] {
            assert!(!has_state(&states, clear), "state {clear}");
        }
        assert!(!has_state(&states, 64), "beyond what was sent");
        assert!(!has_state(&[], 8));
    }

    #[test]
    fn a_dialog_alone_is_never_behind_another() {
        assert!(!is_behind_a_modal(0, &[(true, false)]));
        assert!(!is_behind_a_modal(0, &[(false, false)]));
    }

    #[test]
    fn of_two_modal_dialogs_only_the_one_in_front_can_be_pressed() {
        let stacked = [(true, false), (true, true)];
        assert!(is_behind_a_modal(0, &stacked));
        assert!(!is_behind_a_modal(1, &stacked));
    }

    #[test]
    fn a_dialog_that_is_not_modal_is_behind_one_that_is() {
        let pair = [(false, false), (true, true)];
        assert!(is_behind_a_modal(0, &pair));
        assert!(!is_behind_a_modal(1, &pair));
    }

    #[test]
    fn dialogs_that_are_not_modal_do_not_block_each_other() {
        let pair = [(false, false), (false, true)];
        assert!(!is_behind_a_modal(0, &pair));
        assert!(!is_behind_a_modal(1, &pair));
    }
}

/// Against real GTK windows, over a real accessibility bus. They need a display, a
/// session bus, at-spi2-core and python3 with GTK 3, so they are run by hand:
/// `xvfb-run -a dbus-run-session -- cargo test dialogs::linux_tests -- --ignored`.
#[cfg(all(test, target_os = "linux"))]
mod linux_tests {
    use super::*;
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};

    /// A GTK message box, as SWT's own `MessageBox` raises one. Exits 1 for OK, 2 for Cancel.
    const MESSAGE_BOX: &str = r#"
import sys, gi
gi.require_version('Gtk', '3.0')
from gi.repository import Gtk
box = Gtk.MessageDialog(title=sys.argv[1], text='Workspace in use.', buttons=Gtk.ButtonsType.OK_CANCEL)
answer = box.run()
sys.exit(1 if answer == Gtk.ResponseType.OK else 2 if answer == Gtk.ResponseType.CANCEL else 3)
"#;

    /// A modal plain window with its own buttons, which is what an SWT dialog shell is
    /// to GTK. Exits 1 for Proceed, 2 for Cancel.
    const MODAL_SHELL: &str = r#"
import sys, gi
gi.require_version('Gtk', '3.0')
from gi.repository import Gtk
answer = [3]
def press(code):
    answer[0] = code
    Gtk.main_quit()
shell = Gtk.Window(title=sys.argv[1])
shell.set_modal(True)
column = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
shell.add(column)
column.add(Gtk.Label(label='Errors exist in the project.'))
row = Gtk.Box()
column.add(row)
for label, code in (('_Proceed', 1), ('Cancel', 2)):
    button = Gtk.Button(label=label, use_underline=True)
    button.connect('clicked', lambda _button, code=code: press(code))
    row.add(button)
disabled = Gtk.Button(label='Details')
disabled.set_sensitive(False)
row.add(disabled)
shell.show_all()
Gtk.main()
sys.exit(answer[0])
"#;

    struct KillOnDrop(Child);

    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = self.0.kill();
        }
    }

    fn show(script: &str, title: &str) -> KillOnDrop {
        KillOnDrop(Command::new("python3").args(["-c", script, title]).spawn().expect("python3 starts"))
    }

    /// The dialogs of `child`, once it has one.
    fn dialogs_of(child: &KillOnDrop) -> Vec<Found<imp::Handle>> {
        let scope = HashSet::from([child.0.id()]);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let found = atspi::find(&scope, std::process::id(), &[], &mut Log::default()).expect("the bus answers");
            if !found.is_empty() {
                return found;
            }
            assert!(Instant::now() < deadline, "the window never appeared on the accessibility bus");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn exit_code(child: &mut KillOnDrop) -> Option<i32> {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                return status.code();
            }
            assert!(Instant::now() < deadline, "the other process never saw the click");
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    #[test]
    #[ignore = "needs a display, a session bus and GTK 3"]
    fn a_gtk_message_box_in_another_process_is_listed_and_its_named_button_pressed() {
        let mut child = show(MESSAGE_BOX, "claude-eclipse-core message box");
        let found = dialogs_of(&child);
        // GTK names a stock message box after its kind ("Information"), not its title.
        assert_eq!(found.len(), 1, "{found:?}");
        let dialog = &found[0];
        assert!(dialog.external);
        assert_eq!(dialog.pid, child.0.id());
        assert_eq!(dialog.text, "Workspace in use.");
        let labels: Vec<&str> = dialog.buttons.iter().map(|b| b.label.as_str()).collect();
        assert_eq!(labels, ["Cancel", "OK"], "{dialog:?}");
        assert!(dialog.buttons[0].default, "{dialog:?}");

        let (dialog, button) = choose(&found, &make_id(dialog.pid, &dialog.token), "Cancel").unwrap();
        atspi::press(dialog, button, &mut Log::default()).unwrap();
        assert_eq!(exit_code(&mut child), Some(2));
    }

    #[test]
    #[ignore = "needs a display, a session bus and GTK 3"]
    fn a_modal_plain_window_is_listed_and_its_named_button_pressed() {
        let title = format!("claude-eclipse-core modal shell {}", std::process::id());
        let mut child = show(MODAL_SHELL, &title);
        let found = dialogs_of(&child);
        let dialog = found.iter().find(|d| d.title == title).unwrap();
        assert_eq!(dialog.text, "Errors exist in the project.");
        let buttons: Vec<(&str, bool)> = dialog.buttons.iter().map(|b| (b.label.as_str(), b.enabled)).collect();
        assert_eq!(buttons, [("Proceed", true), ("Cancel", true), ("Details", false)], "{dialog:?}");
        assert!(!dialog.blocked);

        let id = make_id(dialog.pid, &dialog.token);
        assert_eq!(choose(&found, &id, "Details").unwrap_err(), format!("The 'Details' button in '{title}' is disabled."));
        let (dialog, button) = choose(&found, &id, "proceed").unwrap();
        atspi::press(dialog, button, &mut Log::default()).unwrap();
        assert_eq!(exit_code(&mut child), Some(1));
    }

    #[test]
    #[ignore = "needs a display, a session bus and GTK 3"]
    fn a_process_outside_the_scope_is_never_read() {
        let child = show(MESSAGE_BOX, "claude-eclipse-core out of scope");
        dialogs_of(&child);
        let nobody = HashSet::new();
        let found = atspi::find(&nobody, std::process::id(), &[], &mut Log::default()).unwrap();
        assert!(found.is_empty(), "{found:?}");
    }

    /// The plugin calls in from a blocking thread of the MCP server's runtime, built as
    /// `Server::new` builds it, and not from a plain thread as the tests above do.
    #[test]
    #[ignore = "needs a display, a session bus and GTK 3"]
    fn it_works_from_a_blocking_thread_of_the_servers_runtime() {
        let title = format!("claude-eclipse-core runtime thread {}", std::process::id());
        let mut child = show(MODAL_SHELL, &title);
        let pid = child.0.id();
        let runtime = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
        let wanted = title.clone();
        runtime.block_on(async move {
            tokio::task::spawn_blocking(move || {
                let scope = HashSet::from([pid]);
                let deadline = Instant::now() + Duration::from_secs(30);
                let found = loop {
                    let found = atspi::find(&scope, std::process::id(), &[], &mut Log::default()).unwrap();
                    if !found.is_empty() {
                        break found;
                    }
                    assert!(Instant::now() < deadline, "the window never appeared on the accessibility bus");
                    std::thread::sleep(Duration::from_millis(100));
                };
                let id = make_id(found[0].pid, &found[0].token);
                assert_eq!(found[0].title, wanted);
                let (dialog, button) = choose(&found, &id, "Cancel").unwrap();
                atspi::press(dialog, button, &mut Log::default()).unwrap();
            })
            .await
            .unwrap();
        });
        assert_eq!(exit_code(&mut child), Some(2));
    }
}
