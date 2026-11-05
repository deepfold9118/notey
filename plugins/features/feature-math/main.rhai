// Notey official plugin: Math.
// Live math in any tab, like Calca: write `=>` after a calculation and the
// result appears after it. The engine is math.wasm, built from
// crates/notey-math in the Notey repository (cargo xtask math).
// Plugin API: docs/plugin-api.md in the Notey repository.

fn register() {
    #{
        name: "Math",
        version: "1.0.0",
        min_api: 4,
        commands: [
            #{ id: "toggle", title: "Turn Math Off or On in This Tab", group: "Math" },
            #{ id: "rates", title: "Update Exchange Rates", group: "Math" },
        ],
        events: ["ready", "buffer_activated", "text_changed", "net"],
        capabilities: ["net"],
    }
}

// The European Central Bank's reference rates, through Frankfurter.
fn rates_url() {
    "https://api.frankfurter.dev/v1/latest"
}

// Exchange rates older than this (seconds) are fetched again.
fn rates_max_age() {
    12 * 60 * 60
}

// Write every result. `typing`: the change follows the user's typing, so
// it undoes with it; otherwise it is not an undo step of its own.
fn calculate(typing) {
    let text = notey.editor.get_text();
    if !text.contains("=>") {
        return;
    }
    let id = notey.buffers.active();
    let path = notey.buffers.path(id);
    let request = #{
        buffer: id,
        text: text,
        language: notey.editor.language(),
        path: if type_of(path) == "string" { path } else { "" },
        rates: notey.net.cached(rates_url()),
    };
    let reply = notey.wasm.run("math.wasm", "evaluate", request);
    if reply.currency {
        let age = notey.net.age(rates_url());
        if age < 0 || age > rates_max_age() {
            notey.net.fetch(rates_url());
        }
    }
    if reply.edits.len() > 0 {
        notey.editor.set_history(if typing { "join" } else { "skip" });
        notey.editor.replace_ranges(reply.edits);
    }
}

fn on_event(ev) {
    calculate(ev.kind == "text_changed");
}

fn on_command(id) {
    if id == "toggle" {
        let reply = notey.wasm.run("math.wasm", "toggle", #{ buffer: notey.buffers.active() });
        if reply.paused {
            notey.ui.status("Math is off in this tab");
        } else {
            notey.ui.status("Math is on in this tab");
            calculate(false);
        }
    } else if id == "rates" {
        notey.net.fetch(rates_url());
        notey.ui.status("Updating exchange rates");
    }
}
