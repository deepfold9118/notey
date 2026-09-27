// Notey official plugin: Markdown Tools.
// Plugin API: docs/plugin-api.md in the Notey repository.

fn register() {
    let md = #{ language: "Markdown" };
    let md_typing = #{ language: "Markdown", focus: "editor" };
    #{
        name: "Markdown Tools",
        version: "2.0.0",
        min_api: 2,
        commands: [
            #{ id: "bold", title: "Markdown: Bold", shortcut: "Ctrl+B", when: md_typing },
            #{ id: "italic", title: "Markdown: Italic", shortcut: "Ctrl+I", when: md_typing },
            #{ id: "code", title: "Markdown: Inline Code", shortcut: "Ctrl+E", when: md_typing },
            #{ id: "link", title: "Markdown: Link", shortcut: "Ctrl+K", when: md_typing },
            #{ id: "rendered", title: "Markdown: Rendered View", shortcut: "Ctrl+Shift+M", when: md },
            #{ id: "split", title: "Markdown: Side-by-Side Preview", shortcut: "Ctrl+Alt+M", when: md },
        ],
        keys: [ #{ key: "Enter", when: md_typing } ],
        events: [],
    }
}

fn on_command(id) {
    switch id {
        "bold" => toggle_wrap("**"),
        "italic" => toggle_wrap("*"),
        "code" => toggle_wrap("`"),
        "link" => make_link(),
        "rendered" => toggle_preview("rendered"),
        "split" => toggle_preview("split"),
    }
}

fn toggle_preview(mode) {
    if notey.ui.preview() == mode {
        notey.ui.set_preview("off");
    } else {
        notey.ui.set_preview(mode);
    }
}

fn range(a, b) { #{ start: a, end: b } }

// Toggle a symmetric marker around the selection: remove it when it sits
// just outside or just inside the selection, otherwise add it.
fn toggle_wrap(marker) {
    let sel = notey.editor.selection();
    let a = sel.start;
    let b = sel.end;
    let ml = marker.len;
    let total = notey.editor.length();
    let selected = notey.editor.get_range(sel);

    if a >= ml && b + ml <= total {
        let pre = notey.editor.get_range(range(a - ml, a));
        let post = notey.editor.get_range(range(b, b + ml));
        // italic must not mistake the inner stars of **bold** for its own
        let bold_inside = marker == "*" && a > ml && b + ml < total
            && notey.editor.get_range(range(a - ml - 1, a - ml)) == "*"
            && notey.editor.get_range(range(b + ml, b + ml + 1)) == "*";
        if pre == marker && post == marker && !bold_inside {
            notey.editor.replace_range(range(a - ml, b + ml), selected);
            notey.editor.set_selection(range(a - ml, b - ml));
            return;
        }
    }
    if selected.len >= 2 * ml && selected.starts_with(marker) && selected.ends_with(marker) {
        let inner = selected.sub_string(ml, selected.len - 2 * ml);
        notey.editor.replace_range(sel, inner);
        notey.editor.set_selection(range(a, a + inner.len));
        return;
    }
    notey.editor.replace_range(sel, marker + selected + marker);
    notey.editor.set_selection(range(a + ml, a + ml + selected.len));
}

// [text](url) with "url" selected; an empty selection gets placeholder
// link text selected instead.
fn make_link() {
    let sel = notey.editor.selection();
    let a = sel.start;
    let label = notey.editor.get_range(sel);
    let empty = label.len == 0;
    if empty { label = "link text"; }
    notey.editor.replace_range(sel, "[" + label + "](url)");
    if empty {
        notey.editor.set_selection(range(a + 1, a + 1 + label.len));
    } else {
        let url = a + 1 + label.len + 2;
        notey.editor.set_selection(range(url, url + 3));
    }
}

fn is_blank(s) {
    let t = s;
    t.trim();
    t.len == 0
}

fn is_digit(c) { c >= '0' && c <= '9' }

// What Enter should do on a list line, from the text before the cursor:
// "" = not a list line, "END" = empty item (end the list), otherwise the
// prefix for the next item.
fn list_action(before) {
    let indent = "";
    let i = 0;
    while i < before.len && (before[i] == ' ' || before[i] == '\t') {
        indent += before[i];
        i += 1;
    }
    let rest = before.sub_string(i);
    let marker = "";
    let after = "";
    if rest.starts_with("> ") {
        marker = "> ";
        after = rest.sub_string(2);
    } else if rest.len >= 2 && (rest[0] == '-' || rest[0] == '*' || rest[0] == '+') && rest[1] == ' ' {
        marker = rest.sub_string(0, 2);
        after = rest.sub_string(2);
    } else {
        let n = 0;
        while n < rest.len && n < 9 && is_digit(rest[n]) { n += 1; }
        if n == 0 || n + 1 >= rest.len { return ""; }
        let delim = rest[n];
        if (delim != '.' && delim != ')') || rest[n + 1] != ' ' { return ""; }
        let next = parse_int(rest.sub_string(0, n)) + 1;
        marker = `${next}${delim} `;
        after = rest.sub_string(n + 2);
    }
    let task = "";
    let content = after;
    if after.starts_with("[ ] ") || after.starts_with("[x] ") || after.starts_with("[X] ") {
        task = "[ ] ";
        content = after.sub_string(4);
    }
    if is_blank(content) { return "END"; }
    indent + marker + task
}

fn on_key(key) {
    if key != "Enter" { return false; }
    let sel = notey.editor.selection();
    if sel.start != sel.end { return false; }
    let pos = sel.start;
    let line = notey.editor.line_range_at(pos);
    let before = notey.editor.get_range(range(line.start, pos));
    let after = notey.editor.get_range(range(pos, line.end));
    let action = list_action(before);
    if action == "" { return false; }
    if action == "END" {
        if !is_blank(after) { return false; }
        notey.editor.replace_range(range(line.start, pos), "");
        return true;
    }
    notey.editor.insert(pos, "\n" + action);
    true
}
