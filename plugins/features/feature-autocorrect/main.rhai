// Notey official plugin: Autocorrect.
// Plugin API: docs/plugin-api.md in the Notey repository.

fn register() {
    #{
        name: "Autocorrect",
        version: "1.0.0",
        min_api: 2,
        commands: [],
        events: ["input"],
    }
}

fn corrections() {
    #{
        teh: "the", adn: "and", taht: "that", hte: "the", thier: "their",
        recieve: "receive", recieved: "received", beleive: "believe",
        becuase: "because", beacuse: "because", definately: "definitely",
        seperate: "separate", seperately: "separately", occured: "occurred",
        occurence: "occurrence", untill: "until", wich: "which", alot: "a lot",
        tommorow: "tomorrow", tomorow: "tomorrow", accomodate: "accommodate",
        acheive: "achieve", adress: "address", agressive: "aggressive",
        apparantly: "apparently", arguement: "argument", basicly: "basically",
        begining: "beginning", calender: "calendar", comming: "coming",
        commited: "committed", concious: "conscious", embarass: "embarrass",
        enviroment: "environment", existance: "existence", finaly: "finally",
        foriegn: "foreign", freind: "friend", goverment: "government",
        grammer: "grammar", happend: "happened", independant: "independent",
        knowlege: "knowledge", neccessary: "necessary", noticable: "noticeable",
        occassion: "occasion", persistant: "persistent", posession: "possession",
        prefered: "preferred", publically: "publicly", recomend: "recommend",
        refered: "referred", relevent: "relevant", succesful: "successful",
        suprise: "surprise", truely: "truly", wierd: "weird", writting: "writing",
        dont: "don't", doesnt: "doesn't", didnt: "didn't", isnt: "isn't",
        wasnt: "wasn't", arent: "aren't", werent: "weren't", cant: "can't",
        couldnt: "couldn't", shouldnt: "shouldn't", wouldnt: "wouldn't",
        havent: "haven't", hasnt: "hasn't", im: "I'm", ive: "I've",
        youre: "you're", theyre: "they're", thats: "that's", whats: "what's",
    }
}

fn is_letter(c) { c == '\'' || c.to_lower() != c.to_upper() }

fn match_case(original, fixed) {
    if original.len > 1 && original == original.to_upper() {
        return fixed.to_upper();
    }
    let first = original.sub_string(0, 1);
    if first != first.to_lower() {
        return fixed.sub_string(0, 1).to_upper() + fixed.sub_string(1);
    }
    fixed
}

fn on_event(ev) {
    // only after a single word-ending character, and never in code
    let t = ev.text;
    if t.len != 1 || !" .,!?;:)".contains(t) { return; }
    let lang = notey.editor.language();
    if lang != "Plain Text" && lang != "Markdown" { return; }

    let sel = notey.editor.selection();
    if sel.start != sel.end || sel.start < 2 { return; }
    let end = sel.start - 1; // the boundary character just typed
    // look back only as far as a word can reasonably be
    let from = if end > 64 { end - 64 } else { 0 };
    let text = notey.editor.get_range(#{ start: from, end: end });
    let i = text.len;
    while i > 0 && is_letter(text[i - 1]) { i -= 1; }
    if i == text.len { return; }
    let word = text.sub_string(i);
    let start = from + i;

    let fixed = if word == "i" {
        "I"
    } else {
        let table = corrections();
        let key = word.to_lower();
        if key in table { match_case(word, table[key]) } else { return; }
    };
    if fixed == word { return; }
    notey.editor.replace_range(#{ start: start, end: end }, fixed);
    // keep the caret after the boundary character
    let caret = start + fixed.len + 1;
    notey.editor.set_cursor(caret);
}
