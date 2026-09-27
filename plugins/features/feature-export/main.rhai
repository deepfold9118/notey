// Notey official plugin: Export & Print.
// Plugin API: docs/plugin-api.md in the Notey repository.

fn register() {
    #{
        name: "Export & Print",
        version: "1.0.0",
        min_api: 2,
        capabilities: ["fs.write", "open"],
        commands: [
            #{ id: "export_html", title: "Export as HTML…" },
            #{ id: "print", title: "Print Formatted…", shortcut: "Ctrl+Shift+P" },
        ],
        events: [],
    }
}

fn escape_html(s) {
    let out = s;
    out.replace("&", "&amp;");
    out.replace("<", "&lt;");
    out.replace(">", "&gt;");
    out
}

fn doc_title() {
    let id = notey.buffers.active();
    for b in notey.buffers.list() {
        if b.id == id { return b.title; }
    }
    "Untitled"
}

fn to_html(auto_print) {
    let text = notey.editor.get_text();
    let body = if notey.editor.language() == "Markdown" {
        notey.text.markdown_to_html(text)
    } else {
        "<pre>" + escape_html(text) + "</pre>"
    };
    let script = if auto_print { "<script>window.onload = () => window.print();</script>" } else { "" };
    `<!doctype html>
<html><head><meta charset="utf-8"><title>${escape_html(doc_title())}</title>
<style>
body { font-family: "Segoe UI", system-ui, sans-serif; max-width: 46em; margin: 2em auto; padding: 0 1em; line-height: 1.55; color: #1b1b1b; }
pre, code { font-family: Consolas, "Cascadia Mono", monospace; font-size: 0.92em; }
pre { white-space: pre-wrap; word-wrap: break-word; }
pre, code { background: #f4f4f4; border-radius: 4px; }
pre { padding: 0.8em 1em; }
table { border-collapse: collapse; } td, th { border: 1px solid #ccc; padding: 0.3em 0.6em; }
blockquote { border-left: 3px solid #ccc; margin-left: 0; padding-left: 1em; color: #555; }
@media print { body { margin: 0; max-width: none; } }
</style>${script}</head>
<body>
${body}
</body></html>
`
}

fn on_command(id) {
    if id == "export_html" {
        let name = doc_title();
        let dot = name.index_of('.');
        if dot > 0 { name = name.sub_string(0, dot); }
        let path = notey.ui.save_file_dialog(name + ".html");
        if path == "" { return; }
        notey.fs.write_text(path, to_html(false));
        notey.ui.status("Exported " + path);
    } else if id == "print" {
        let path = notey.app.temp_path("notey-print.html");
        notey.fs.write_text(path, to_html(true));
        notey.app.open(path);
        notey.ui.status("Opened in your browser for printing");
    }
}
