// Notey official plugin: Markdown Tools.
// Plugin API: docs/plugin-api.md in the Notey repository.

fn register() {
    #{
        name: "Markdown Tools",
        version: "1.0.0",
        min_api: 1,
        commands: [
            #{ id: "toggle_md", title: "Toggle Raw/Rendered Markdown", shortcut: "Ctrl+Shift+M" },
        ],
        events: [],
    }
}

fn on_command(id) {
    if id == "toggle_md" {
        if notey.editor.language() == "Markdown" {
            notey.ui.toggle_markdown_preview();
        } else {
            notey.ui.status("Not a Markdown tab - set the language to Markdown in the status bar first");
        }
    }
}
