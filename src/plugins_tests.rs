//! Tests for the plugin host, run against the official feature plugins in
//! `plugins/features/` through the same load and dispatch path the app uses.

use super::*;

/// Load an official feature plugin exactly as the app does: from its file,
/// with the capabilities its package manifest grants.
fn official(id: &str) -> PluginHost {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins/features").join(id);
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("plugin.json")).unwrap()).unwrap();
    let granted: Vec<String> = manifest["capabilities"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let script = PluginHost::load_script(&dir.join("main.rhai"), Some(&granted))
        .unwrap_or_else(|e| panic!("{id}: {e}"));
    PluginHost {
        scripts: vec![script],
        errors: Vec::new(),
    }
}

fn ctx(text: &str, sel: (usize, usize), language: Option<&str>) -> ScriptCtx {
    ScriptCtx {
        text: text.to_string(),
        sel,
        text_dirty: false,
        sel_dirty: false,
        buffers: vec![BufInfo {
            id: 1,
            title: "notes.md".into(),
            path: None,
            modified: false,
            active: true,
        }],
        active_id: 1,
        language: language.map(String::from),
        config_dir: String::new(),
        actions: Vec::new(),
        status: None,
        preview: "off".into(),
        script_name: String::new(),
        capabilities: Vec::new(),
    }
}

fn event(kind: &str, text: Option<&str>) -> Map {
    let mut m = Map::new();
    m.insert("kind".into(), Dynamic::from(kind.to_string()));
    if let Some(t) = text {
        m.insert("text".into(), Dynamic::from(t.to_string()));
    }
    m
}

#[test]
fn every_official_feature_script_loads() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins/features");
    for dir in std::fs::read_dir(root).unwrap().filter_map(Result::ok) {
        if dir.path().join("main.rhai").exists() {
            official(&dir.file_name().to_string_lossy());
        }
    }
}

#[test]
fn markdown_commands_are_scoped_to_markdown() {
    let host = official("feature-markdown-tools");
    let s = &host.scripts[0];
    let bold = s.commands.iter().find(|c| c.id == "bold").unwrap();
    assert!(bold.when.matches(Some("Markdown"), true));
    assert!(!bold.when.matches(Some("Markdown"), false), "needs editor focus");
    assert!(!bold.when.matches(None, true), "not in plain text");
    assert_eq!(s.keys.len(), 1);
    assert_eq!(s.keys[0].0, Key::Enter);
}

#[test]
fn markdown_bold_toggles_on_and_off() {
    let mut host = official("feature-markdown-tools");
    let out = host.run_command(0, "bold", ctx("make this bold", (5, 9), Some("Markdown")));
    assert!(host.errors.is_empty(), "{:?}", host.errors);
    assert_eq!(out.text, "make **this** bold");
    assert_eq!(out.sel, (7, 11));
    let out = host.run_command(0, "bold", ctx(&out.text, out.sel, Some("Markdown")));
    assert_eq!(out.text, "make this bold");
    assert_eq!(out.sel, (5, 9));
}

#[test]
fn markdown_link_selects_url_placeholder() {
    let mut host = official("feature-markdown-tools");
    let out = host.run_command(0, "link", ctx("see docs", (4, 8), Some("Markdown")));
    assert_eq!(out.text, "see [docs](url)");
    assert_eq!(out.sel, (11, 14));
}

#[test]
fn markdown_enter_continues_and_ends_lists() {
    let mut host = official("feature-markdown-tools");
    let (out, handled) = host.run_key(0, "Enter", ctx("- milk", (6, 6), Some("Markdown")));
    assert!(host.errors.is_empty(), "{:?}", host.errors);
    assert!(handled);
    assert_eq!(out.text, "- milk\n- ");

    let (out, handled) = host.run_key(0, "Enter", ctx("9. ninth", (8, 8), Some("Markdown")));
    assert!(handled);
    assert_eq!(out.text, "9. ninth\n10. ");

    let (out, handled) = host.run_key(0, "Enter", ctx("- [x] done", (10, 10), Some("Markdown")));
    assert!(handled);
    assert_eq!(out.text, "- [x] done\n- [ ] ");

    // an empty item ends the list
    let (out, handled) = host.run_key(0, "Enter", ctx("- a\n- ", (6, 6), Some("Markdown")));
    assert!(handled);
    assert_eq!(out.text, "- a\n");

    // ordinary lines are left to the editor
    let (_, handled) = host.run_key(0, "Enter", ctx("plain", (5, 5), Some("Markdown")));
    assert!(!handled);
    let (_, handled) = host.run_key(0, "Enter", ctx("2024 was a year", (15, 15), Some("Markdown")));
    assert!(!handled);
}

#[test]
fn markdown_preview_modes_toggle() {
    let mut host = official("feature-markdown-tools");
    let out = host.run_command(0, "split", ctx("# hi", (0, 0), Some("Markdown")));
    assert!(matches!(out.actions.as_slice(), [HostAction::SetPreview(m)] if m == "split"));
    let mut c = ctx("# hi", (0, 0), Some("Markdown"));
    c.preview = "split".into();
    let out = host.run_command(0, "split", c);
    assert!(matches!(out.actions.as_slice(), [HostAction::SetPreview(m)] if m == "off"));
}

#[test]
fn word_count_ignores_markup_tokens() {
    assert_eq!(word_count(""), 0);
    assert_eq!(word_count("# Weekend trip"), 2);
    assert_eq!(word_count("- [ ] rain jacket"), 2);
    assert_eq!(word_count("1. Leave by 8 am"), 5); // "1." counts, like Word
    assert_eq!(word_count("> quoted — text"), 2);
    assert_eq!(word_count("don't stop-now 日本語"), 3);
}

#[test]
fn word_count_sets_a_status_item() {
    let mut host = official("feature-word-count");
    let out = host.run_event(0, event("text_changed", None), ctx("one two three", (0, 0), None));
    assert!(host.errors.is_empty(), "{:?}", host.errors);
    assert!(matches!(
        out.actions.as_slice(),
        [HostAction::SetStatusItem(key, text, tip)]
            if key == "Word Count::words" && text == "3 words" && tip == "About 1 minute to read"
    ));
    let out = host.run_event(0, event("selection_changed", None), ctx("one two three", (0, 7), None));
    assert!(matches!(out.actions.as_slice(), [HostAction::SetStatusItem(_, text, _)] if text == "2 of 3 words"));
}

#[test]
fn autocorrect_fixes_typos_in_prose_only() {
    let mut host = official("feature-autocorrect");
    // "teh" then a space was typed; the caret sits after the space
    let out = host.run_event(0, event("input", Some(" ")), ctx("I saw teh ", (10, 10), None));
    assert!(host.errors.is_empty(), "{:?}", host.errors);
    assert_eq!(out.text, "I saw the ");
    assert_eq!(out.sel, (10, 10));

    let out = host.run_event(0, event("input", Some(".")), ctx("Recieve.", (8, 8), Some("Markdown")));
    assert_eq!(out.text, "Receive.");

    let out = host.run_event(0, event("input", Some(" ")), ctx("so i ", (5, 5), None));
    assert_eq!(out.text, "so I ");

    // code is never touched
    let out = host.run_event(0, event("input", Some(" ")), ctx("teh ", (4, 4), Some("Rust")));
    assert!(!out.text_dirty);
    // correct words are left alone
    let out = host.run_event(0, event("input", Some(" ")), ctx("the ", (4, 4), None));
    assert!(!out.text_dirty);
}

#[test]
fn export_print_writes_html_and_opens_it() {
    let mut host = official("feature-export");
    let out = host.run_command(0, "print", ctx("# Title\n\nSome *text*", (0, 0), Some("Markdown")));
    assert!(host.errors.is_empty(), "{:?}", host.errors);
    let path = out
        .actions
        .iter()
        .find_map(|a| match a {
            HostAction::OpenPath(p) => Some(p.clone()),
            _ => None,
        })
        .expect("opens the printable page");
    let html = std::fs::read_to_string(&path).unwrap();
    assert!(html.contains("<h1>Title</h1>"));
    assert!(html.contains("<em>text</em>"));
    assert!(html.contains("window.print()"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn capabilities_must_be_granted_by_the_package() {
    // the same script, installed from a package that grants nothing
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("plugins/features/feature-export");
    let script = PluginHost::load_script(&dir.join("main.rhai"), Some(&[])).unwrap();
    assert!(script.capabilities.is_empty());
    let mut host = PluginHost {
        scripts: vec![script],
        errors: Vec::new(),
    };
    let out = host.run_command(0, "print", ctx("text", (0, 0), None));
    assert!(out.actions.iter().all(|a| !matches!(a, HostAction::OpenPath(_))));
    assert!(
        host.errors.iter().any(|e| e.contains("permission 'fs.write'")),
        "{:?}",
        host.errors
    );
}

#[test]
fn legacy_markdown_script_is_recognized() {
    assert!(LEGACY_MARKDOWN_SCRIPT.contains("Markdown Preview"));
    let crlf = LEGACY_MARKDOWN_SCRIPT.replace('\n', "\r\n");
    assert_eq!(crlf.replace("\r\n", "\n"), LEGACY_MARKDOWN_SCRIPT);
}
