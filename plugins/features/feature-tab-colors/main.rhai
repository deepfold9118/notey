// Notey official plugin: Tab Colors.
// Plugin API: docs/plugin-api.md in the Notey repository.
//
// Right-click a tab and pick a color from the Color submenu. Notey draws the
// color as a stripe on the tab, and remembers it for the file, so the tab
// has it again the next time the file opens.
//
// The colors are names, not fixed values: the theme decides them (a VS Code
// theme's terminal colors), so tabs match whichever theme is in use.

fn colors() {
    ["red", "yellow", "green", "cyan", "blue", "magenta"]
}

fn register() {
    let commands = [];
    for c in colors() {
        let title = c.sub_string(0, 1).to_upper() + c.sub_string(1);
        commands.push(#{ id: c, title: title, swatch: c, menu: "Tab", group: "Color" });
    }
    commands.push(#{ id: "none", title: "No Color", menu: "Tab", group: "Color" });
    #{
        name: "Tab Colors",
        version: "1.0.0",
        min_api: 5,
        commands: commands,
        events: [],
    }
}

fn on_command(id) {
    notey.editor.set_tab_color(if id == "none" { "" } else { id });
}
