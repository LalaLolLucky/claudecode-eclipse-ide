use super::*;

fn paths(entries: &[Entry]) -> Vec<&str> {
    entries.iter().map(|e| e.path.as_str()).collect()
}

#[test]
fn browser_rows_follow_files_unless_the_word_could_become_browser() {
    let files = r#"[{"path":"a.txt","name":"a.txt","type":"file"}]"#;
    let tabs = r#"[{"path":"browser:new_tab","name":"browser:new_tab","type":"browser"}]"#;
    let order = |q: &str| -> Vec<String> {
        serde_json::from_str::<Vec<serde_json::Value>>(&with_browser_rows(files, tabs, q))
            .unwrap()
            .iter()
            .map(|r| r["type"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(order(""), ["file", "browser"]);
    assert_eq!(order("a"), ["file", "browser"]);
    assert_eq!(order("b"), ["browser", "file"]);
    assert_eq!(order("BRO"), ["browser", "file"]);
    assert_eq!(order("browser:"), ["browser", "file"]);
    assert_eq!(order("browsers"), ["file", "browser"]);
    assert_eq!(with_browser_rows(files, "not json", ""), with_browser_rows(files, "[]", ""));
}

#[test]
fn folders_lead_their_contents_and_subfolders_lead_files() {
    let files = ["b.txt", "a/z.txt", "a/b/c.txt", "A2/x.txt"].map(String::from).to_vec();
    let entries = entries_from_files(files);
    assert_eq!(
        paths(&entries),
        ["a/", "a/b/", "a/b/c.txt", "a/z.txt", "A2/", "A2/x.txt", "b.txt"]
    );
    assert!(entries[0].dir && !entries[2].dir);
    assert_eq!(entries[1].name, "b");
}

#[test]
fn folder_part_of_a_query_scopes_the_search() {
    let files = ["src/main.rs", "src/lib.rs", "docs/main.md"].map(String::from).to_vec();
    let entries = entries_from_files(files);
    let got: Vec<&str> = rank(&entries, "src/").iter().map(|e| e.path.as_str()).collect();
    assert_eq!(got, ["src/", "src/lib.rs", "src/main.rs"]);
}

#[test]
fn name_match_outranks_path_match_and_scattered_letters_still_match() {
    let files = ["main/other.rs", "x/main.rs", "m_a_i_n.txt"].map(String::from).to_vec();
    let entries = entries_from_files(files);
    let got: Vec<&str> = rank(&entries, "main").iter().map(|e| e.path.as_str()).collect();
    assert_eq!(got[0], "main/");
    assert!(got.contains(&"x/main.rs"));
    assert!(got.contains(&"m_a_i_n.txt"));
    assert!(rank(&entries, "zzz").is_empty());
}

#[test]
fn walk_keeps_dotfiles_and_skips_vcs_folders() {
    let root = std::env::temp_dir().join("claude-eclipse-mentions-walk");
    let _ = std::fs::remove_dir_all(&root);
    for p in [".git/config", ".hidden/h.txt", "sub/file.txt"] {
        let f = root.join(p);
        std::fs::create_dir_all(f.parent().unwrap()).unwrap();
        std::fs::write(f, "x").unwrap();
    }
    let mut files = walk(&root.to_string_lossy());
    let _ = std::fs::remove_dir_all(&root);
    files.sort();
    assert_eq!(files, [".hidden/h.txt", "sub/file.txt"]);
}
