// Notey official plugin: Word Count.
// Plugin API: docs/plugin-api.md in the Notey repository.

fn register() {
    #{
        name: "Word Count",
        version: "1.0.0",
        min_api: 2,
        commands: [],
        events: ["ready", "buffer_activated", "text_changed", "selection_changed"],
    }
}

fn on_event(ev) {
    let total = notey.text.word_count(notey.editor.get_text());
    let sel = notey.editor.selection();
    let label = "";
    if sel.start != sel.end {
        let picked = notey.text.word_count(notey.editor.get_range(sel));
        label = `${picked} of ${total} words`;
    } else if total == 1 {
        label = "1 word";
    } else {
        label = `${total} words`;
    }
    // ~230 words per minute, rounded up
    let minutes = (total + 229) / 230;
    let tip = if total == 0 {
        "Nothing to read yet"
    } else if minutes == 1 {
        "About 1 minute to read"
    } else {
        `About ${minutes} minutes to read`
    };
    notey.ui.set_status_item("words", label, tip);
}
