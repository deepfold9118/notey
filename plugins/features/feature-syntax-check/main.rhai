// Notey official plugin: Syntax Check.
// Shows where the syntax might be wrong, when you pause typing: errors in
// JSON, YAML, TOML and XML files, and hints for unmatched brackets and
// quotes in code. Problems get a wavy underline, and the problem bar above
// the status bar shows the one nearest the cursor. The checks run in
// syntax.wasm, built from crates/notey-syntax in the Notey repository
// (cargo xtask syntax).
// Plugin API: docs/plugin-api.md in the Notey repository.

fn register() {
    #{
        name: "Syntax Check",
        version: "1.0.0",
        min_api: 7,
        commands: [],
        settings: [
            #{ id: "bar", title: "Problem bar",
               description: "What the bar above the status bar shows. Problems are always underlined",
               kind: "choice", initial: "all",
               choices: [["all", "Errors and hints"], ["errors", "Errors only"], ["never", "Nothing"]] },
            #{ id: "brackets", title: "Check brackets and quotes in code",
               description: "Hints where a bracket or a quote might be missing. JSON, YAML, TOML and XML are always checked",
               kind: "toggle", initial: true },
        ],
        events: ["ready", "buffer_opened", "buffer_activated", "text_idle"],
    }
}

fn check() {
    let path = notey.buffers.path(notey.buffers.active());
    let reply = notey.wasm.run("syntax.wasm", "check_text", #{
        text: notey.editor.get_text(),
        language: notey.editor.language(),
        path: if type_of(path) == "string" { path } else { "" },
        brackets: notey.settings.get("brackets") != "false",
    });
    let bar = notey.settings.get("bar");
    let problems = [];
    for p in reply.problems {
        p.bar = bar == "all" || (bar == "errors" && p.severity == "error");
        problems.push(p);
    }
    notey.editor.set_problems(problems);
}

fn on_event(ev) {
    check();
}
