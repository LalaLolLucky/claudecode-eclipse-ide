#[test]
fn guide_json_renders_every_section() {
    let json =
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/.settings/org.eclipse.core.freebsd.container"));
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
