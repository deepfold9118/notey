// Notey official plugin: Text Color.
// Plugin API: docs/plugin-api.md in the Notey repository.
//
// Gives the selected text a text color or a background color from the 16
// terminal (ANSI) colors. Notey keeps the colors while the tab is open. It
// does not write them to the file.

// The VS Code terminal palette: the 8 normal colors, then the 8 bright ones.
fn palette() {
    [
        ["Black", "#000000"], ["Red", "#cd3131"], ["Green", "#0dbc79"],
        ["Yellow", "#e5e510"], ["Blue", "#2472c8"], ["Magenta", "#bc3fbc"],
        ["Cyan", "#11a8cd"], ["White", "#e5e5e5"],
        ["Bright Black", "#666666"], ["Bright Red", "#f14c4c"],
        ["Bright Green", "#23d18b"], ["Bright Yellow", "#f5f543"],
        ["Bright Blue", "#3b8eea"], ["Bright Magenta", "#d670d6"],
        ["Bright Cyan", "#29b8db"], ["Bright White", "#ffffff"],
    ]
}

fn register() {
    let commands = [];
    for layer in [["fg", "Text Color"], ["bg", "Background Color"]] {
        let i = 0;
        for c in palette() {
            commands.push(#{
                id: `${layer[0]}:${i}`, title: c[0], swatch: c[1],
                menu: "Format", group: layer[1], context: true,
            });
            i += 1;
        }
        commands.push(#{
            id: `${layer[0]}:default`, title: "Default",
            menu: "Format", group: layer[1], context: true,
        });
    }
    commands.push(#{ id: "clear", title: "Clear Colors", menu: "Format", context: true });
    #{
        name: "Text Color",
        version: "1.0.0",
        min_api: 3,
        commands: commands,
        events: [],
    }
}

fn on_command(id) {
    let sel = notey.editor.selection();
    if sel.start == sel.end {
        notey.ui.status("Select some text first");
        return;
    }
    if id == "clear" {
        notey.editor.clear_styles(sel);
        return;
    }
    let parts = id.split(":");
    let layer = parts[0];
    let color = if parts[1] == "default" { "" } else { palette()[parse_int(parts[1])][1] };
    let style = #{};
    style[layer] = color;
    notey.editor.style_range(sel, style);
}
