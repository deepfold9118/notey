// Notey official plugin: Terminal.
// Plugin API: docs/plugin-api.md in the Notey repository.
//
// Adds "Open in Terminal" and "Run in Terminal" to the tab menu, and
// Ctrl+F5 to run the active tab. The terminal app and the shell are chosen
// in Settings > Terminal, from what is installed. Running a file saves it
// first; an untitled tab runs from a temporary copy.

fn register() {
    let settings = [
        #{ id: "terminal", title: "Terminal", description: "The window that the shell runs in",
           kind: "choice", choices: "terminal_choices", initial: "auto" },
        #{ id: "terminal_path", title: "Terminal program", description: "The terminal's .exe file",
           kind: "path", show_if: #{ setting: "terminal", value: "other" } },
        #{ id: "terminal_args", title: "Terminal arguments",
           description: "{dir} is the folder, {command} the shell and what it runs",
           kind: "text", initial: "{command}", show_if: #{ setting: "terminal", value: "other" } },
        #{ id: "shell", title: "Shell", description: "The program that runs your commands",
           kind: "choice", choices: "shell_choices", initial: "auto" },
        #{ id: "shell_path", title: "Shell program", description: "The shell's .exe file",
           kind: "path", show_if: #{ setting: "shell", value: "other" } },
        #{ id: "shell_args", title: "Shell arguments to run a command",
           description: "{command} is the command, for example: -c {command}",
           kind: "text", initial: "{command}", show_if: #{ setting: "shell", value: "other" } },
    ];
    for k in kinds() {
        settings.push(#{
            id: `cmd_${k.id}`, title: `${k.name} command`,
            description: `{file} is the file. Empty: ${k.windows}, or in WSL: ${k.unix}`,
            kind: "text", initial: "",
        });
    }
    #{
        name: "Terminal",
        version: "1.0.0",
        min_api: 6,
        capabilities: ["run", "fs.write"],
        commands: [
            #{ id: "open", title: "Open in Terminal", menu: "Tab" },
            #{ id: "run", title: "Run in Terminal", menu: "Tab", shortcut: "Ctrl+F5" },
        ],
        settings: settings,
        events: [],
    }
}

// ---------- what runs each kind of file ----------

fn kinds() {
    [
        #{ id: "python", name: "Python", exts: ["py", "pyw"], language: "Python",
           windows: "py {file}", unix: "python3 {file}" },
        #{ id: "javascript", name: "JavaScript", exts: ["js", "mjs", "cjs"], language: "JavaScript",
           windows: "node {file}", unix: "node {file}" },
        #{ id: "powershell", name: "PowerShell", exts: ["ps1"], language: "PowerShell",
           windows: "powershell -NoProfile -ExecutionPolicy Bypass -File {file}", unix: "pwsh -File {file}" },
        #{ id: "batch", name: "Batch file", exts: ["bat", "cmd"], language: "Batch File",
           windows: "cmd /c {file}", unix: "cmd.exe /c {file}", gitbash: "cmd //c {file}" },
        #{ id: "bash", name: "Shell script", exts: ["sh", "bash"], language: "Bourne Again Shell (bash)",
           windows: "bash {file}", unix: "bash {file}" },
        #{ id: "ruby", name: "Ruby", exts: ["rb"], language: "Ruby", windows: "ruby {file}", unix: "ruby {file}" },
        #{ id: "lua", name: "Lua", exts: ["lua"], language: "Lua", windows: "lua {file}", unix: "lua {file}" },
        #{ id: "perl", name: "Perl", exts: ["pl"], language: "Perl", windows: "perl {file}", unix: "perl {file}" },
        #{ id: "go", name: "Go", exts: ["go"], language: "Go", windows: "go run {file}", unix: "go run {file}" },
    ]
}

// The kind of the tab: by its file's extension, else by its language.
fn kind_of(path, language) {
    let ext = extension(path);
    for k in kinds() {
        if path != "" && ext in k.exts { return k; }
        if path == "" && language == k.language { return k; }
    }
    ()
}

fn file_name(path) {
    let parts = path.split("\\");
    parts[parts.len() - 1]
}

fn extension(path) {
    let name = file_name(path);
    let parts = name.split(".");
    if parts.len() < 2 { return ""; }
    parts[parts.len() - 1].to_lower()
}

fn folder(path) {
    let name = file_name(path);
    if name.len() >= path.len() { return path; }
    path.sub_string(0, path.len() - name.len() - 1)
}

// ---------- what is installed ----------

fn find(name, guesses) {
    let found = notey.app.which(name);
    if found != "" { return found; }
    for g in guesses {
        if notey.fs.exists(g) { return g; }
    }
    ""
}

fn terminals() {
    let local = notey.app.env("LOCALAPPDATA");
    let programs = notey.app.env("ProgramFiles");
    let list = [];
    let wt = find("wt", [`${local}\Microsoft\WindowsApps\wt.exe`]);
    if wt != "" { list.push(#{ id: "wt", label: "Windows Terminal", path: wt }); }
    let alacritty = find("alacritty", [`${programs}\Alacritty\alacritty.exe`]);
    if alacritty != "" { list.push(#{ id: "alacritty", label: "Alacritty", path: alacritty }); }
    let wezterm = find("wezterm", [`${programs}\WezTerm\wezterm.exe`]);
    if wezterm != "" { list.push(#{ id: "wezterm", label: "WezTerm", path: wezterm }); }
    let warp = find("warp", [`${local}\Programs\Warp\warp.exe`, `${programs}\Warp\warp.exe`]);
    if warp != "" { list.push(#{ id: "warp", label: "Warp", path: warp }); }
    list.push(#{ id: "console", label: "Console window", path: "" });
    list
}

fn shells() {
    let programs = notey.app.env("ProgramFiles");
    let system = notey.app.env("SystemRoot") + "\\System32";
    let list = [];
    let pwsh = find("pwsh", [`${programs}\PowerShell\7\pwsh.exe`]);
    if pwsh != "" { list.push(#{ id: "pwsh", label: "PowerShell 7", path: pwsh }); }
    let ps = `${system}\WindowsPowerShell\v1.0\powershell.exe`;
    if notey.fs.exists(ps) { list.push(#{ id: "powershell", label: "Windows PowerShell", path: ps }); }
    let cmd = notey.app.env("ComSpec");
    if cmd == "" { cmd = `${system}\cmd.exe`; }
    list.push(#{ id: "cmd", label: "Command Prompt", path: cmd });
    for g in [`${programs}\Git\bin\bash.exe`, notey.app.env("LOCALAPPDATA") + "\\Programs\\Git\\bin\\bash.exe"] {
        if notey.fs.exists(g) {
            list.push(#{ id: "gitbash", label: "Git Bash", path: g });
            break;
        }
    }
    let wsl = `${system}\wsl.exe`;
    if notey.fs.exists(wsl) { list.push(#{ id: "wsl", label: "WSL", path: wsl }); }
    list
}

fn choices_of(list) {
    let out = [["auto", `Automatic (${list[0].label})`]];
    for x in list { out.push([x.id, x.label]); }
    out.push(["other", "Other…"]);
    out
}

fn terminal_choices() { choices_of(terminals()) }
fn shell_choices() { choices_of(shells()) }

// The chosen entry of `list`, or the first; "other" uses the path setting.
fn pick(list, id, path) {
    if id == "other" { return #{ id: "other", label: "Other", path: path }; }
    for x in list {
        if x.id == id { return x; }
    }
    list[0]
}

// ---------- building the command line ----------

// The default command for `kind` in `shell`. Git Bash runs Windows
// programs (and turns /c/... paths into C:/... for them); WSL runs Linux.
fn default_command(kind, shell) {
    if shell.id == "wsl" { return kind.unix; }
    if shell.id == "gitbash" && "gitbash" in kind { return kind.gitbash; }
    kind.windows
}

// A path as the shell writes it, quoted.
fn quote(shell, path) {
    if shell.id == "gitbash" || shell.id == "wsl" {
        let p = path;
        p.replace("\\", "/");
        if p.len() > 1 && p.sub_string(1, 1) == ":" {
            let drive = p.sub_string(0, 1).to_lower();
            let root = if shell.id == "wsl" { "/mnt/" } else { "/" };
            p = root + drive + p.sub_string(2);
        }
        p.replace("'", "'\\''");
        return "'" + p + "'";
    }
    if shell.id == "pwsh" || shell.id == "powershell" {
        let p = path;
        p.replace("'", "''");
        return "'" + p + "'";
    }
    "\"" + path + "\""
}

// Split a template on spaces and fill in its {placeholders}; {command}
// becomes the `command` arguments.
fn fill(template, dir, command) {
    let out = [];
    let used = false;
    for word in template.split(" ") {
        if word == "" { continue; }
        if word == "{command}" {
            out += command;
            used = true;
        } else {
            let w = word;
            w.replace("{dir}", dir);
            out.push(w);
        }
    }
    if !used { out += command; }
    out
}

// The shell's arguments: none to just open it, else run `line` and stay.
// The Command Prompt gets `words`, the same command as separate arguments:
// cmd.exe doesn't read the \" that quoting a whole line would need.
fn shell_args(shell, dir, line, words) {
    let id = shell.id;
    if id == "wsl" {
        let args = ["--cd", dir];
        if line != "" { args += ["--", "bash", "-lc", line + "; exec bash"]; }
        return args;
    }
    if line == "" {
        if id == "gitbash" { return ["--login", "-i"]; }
        if id == "other" { return []; }
        return [];
    }
    switch id {
        "pwsh" => ["-NoExit", "-Command", "& " + line],
        "powershell" => ["-NoExit", "-Command", "& " + line],
        "cmd" => ["/k"] + words,
        "gitbash" => ["--login", "-i", "-c", line + "; exec bash"],
        _ => fill(notey.settings.get("shell_args"), dir, [line]),
    }
}

// Start `shell` (with `args`) in the chosen terminal, in `dir`. Returns a
// note for the status bar, or "".
fn launch(shell, args, dir, running) {
    let term = pick(terminals(), notey.settings.get("terminal"), notey.settings.get("terminal_path"));
    let sh = [shell.path] + args;
    let note = "";
    if term.id == "warp" && running {
        // Warp can open a folder, but not be given a command.
        term = #{ id: "console", label: "Console window", path: "" };
        note = "Warp can't run a command, so it runs in a console window";
    }
    switch term.id {
        "wt" => {
            // Windows Terminal reads ";" as "new tab"
            let escaped = [];
            for a in sh {
                let x = a;
                x.replace(";", "\\;");
                escaped.push(x);
            }
            notey.process.start(term.path, ["-d", dir] + escaped, dir);
        }
        "alacritty" => notey.process.start(term.path, ["--working-directory", dir, "-e"] + sh, dir),
        "wezterm" => notey.process.start(term.path, ["start", "--cwd", dir, "--"] + sh, dir),
        "warp" => {
            let d = dir;
            d.replace(" ", "%20");
            notey.process.start("explorer.exe", ["warp://action/new_window?path=" + d], dir);
        }
        "other" => {
            if term.path == "" { return "Choose the terminal program in Settings > Terminal"; }
            notey.process.start(term.path, fill(notey.settings.get("terminal_args"), dir, sh), dir);
        }
        _ => notey.process.start(shell.path, args, dir),
    }
    note
}

// ---------- commands ----------

fn on_command(id) {
    let buffer = notey.buffers.active();
    let path = notey.buffers.path(buffer);
    if path == () { path = ""; }
    let shell = pick(shells(), notey.settings.get("shell"), notey.settings.get("shell_path"));
    if shell.path == "" {
        notey.ui.status("Choose the shell program in Settings > Terminal");
        return;
    }
    let dir = if path == "" { notey.app.env("USERPROFILE") + "\\Documents" } else { folder(path) };

    if id == "open" {
        let note = launch(shell, shell_args(shell, dir, "", []), dir, false);
        if note != "" { notey.ui.status(note); }
        return;
    }

    let kind = kind_of(path, notey.editor.language());
    if kind == () {
        notey.ui.status("Notey doesn't know how to run this file type");
        return;
    }
    let file = path;
    if path == "" {
        // an untitled tab runs from a temporary copy
        let title = "untitled";
        for b in notey.buffers.list() {
            if b.id == buffer { title = b.title; }
        }
        file = notey.app.temp_path(title + "." + kind.exts[0]);
        notey.fs.write_text(file, notey.editor.get_text());
    } else {
        for b in notey.buffers.list() {
            if b.id == buffer && b.modified { notey.buffers.save(); }
        }
    }
    let template = notey.settings.get(`cmd_${kind.id}`);
    if template == "" { template = default_command(kind, shell); }
    let line = template;
    line.replace("{file}", quote(shell, file));
    let words = [];
    for w in template.split(" ") {
        if w == "" { continue; }
        let x = w;
        x.replace("{file}", file);
        words.push(x);
    }
    let note = launch(shell, shell_args(shell, dir, line, words), dir, true);
    notey.ui.status(if note != "" { note } else { `Running ${file_name(file)}` });
}
