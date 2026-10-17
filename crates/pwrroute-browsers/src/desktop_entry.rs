//! Linux desktop entries (`.desktop` files): just enough of the Desktop
//! Entry Specification to find a browser and run its `Exec` line.
//! <https://specifications.freedesktop.org/desktop-entry-spec/latest/>

use std::path::{Path, PathBuf};

use crate::env::Env;

/// The `[Desktop Entry]` group of a desktop file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DesktopEntry {
    pub name: String,
    pub exec: String,
    pub mime_types: Vec<String>,
    /// `Hidden=true`: the entry is deleted, as far as the user is concerned.
    pub hidden: bool,
}

impl DesktopEntry {
    pub fn parse(text: &str) -> Option<Self> {
        let mut in_main = false;
        let (mut name, mut exec, mut mime, mut hidden, mut kind) = (None, None, "", false, "");
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_main = line == "[Desktop Entry]";
                continue;
            }
            if !in_main || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "Name" => name = Some(value.trim()),
                "Exec" => exec = Some(value.trim()),
                "MimeType" => mime = value.trim(),
                "Hidden" => hidden = value.trim() == "true",
                "Type" => kind = value.trim(),
                _ => {}
            }
        }
        if kind != "Application" {
            return None;
        }
        Some(Self {
            name: unescape_string(name?),
            exec: unescape_string(exec?),
            mime_types: mime
                .split(';')
                .filter(|m| !m.is_empty())
                .map(str::to_owned)
                .collect(),
            hidden,
        })
    }

    /// Registers for `https:` links.
    pub fn handles_https(&self) -> bool {
        self.mime_types
            .iter()
            .any(|m| m == "x-scheme-handler/https")
    }
}

/// The `applications` directories, most important first: `$XDG_DATA_HOME`,
/// then `$XDG_DATA_DIRS`, then Flatpak's and Snap's export directories in
/// case the session did not add them to `$XDG_DATA_DIRS`.
pub(crate) fn application_dirs(env: &Env) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(data_home) = env.expand("{xdg_data}") {
        dirs.push(data_home.join("applications"));
    }
    let data_dirs = env
        .var("XDG_DATA_DIRS")
        .unwrap_or("/usr/local/share:/usr/share");
    for d in data_dirs.split(':').filter(|d| !d.is_empty()) {
        dirs.push(env.system_path(d).join("applications"));
    }
    dirs.push(
        env.home()
            .join(".local/share/flatpak/exports/share/applications"),
    );
    dirs.push(env.system_path("/var/lib/flatpak/exports/share/applications"));
    dirs.push(env.system_path("/var/lib/snapd/desktop/applications"));
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs
}

/// The first desktop file named `id` in [`application_dirs`].
pub(crate) fn find(env: &Env, id: &str) -> Option<(PathBuf, DesktopEntry)> {
    let id = if id.ends_with(".desktop") {
        id.to_owned()
    } else {
        format!("{id}.desktop")
    };
    application_dirs(env).into_iter().find_map(|dir| {
        let path = dir.join(&id);
        let entry = DesktopEntry::parse(&std::fs::read_to_string(&path).ok()?)?;
        (!entry.hidden).then_some((path, entry))
    })
}

/// Splits an `Exec` value into the program and its arguments, dropping
/// field codes and Flatpak's `@@`/`@@u` markers. `extra` (profile flags,
/// args, the URL) goes where the first file or URL field code was, or at
/// the end. `None` for an empty or unbalanced line.
pub(crate) fn exec_command(exec: &str, extra: &[String]) -> Option<(PathBuf, Vec<String>)> {
    let mut words = Vec::new();
    let mut insert_at = None;
    for word in split_exec(exec)? {
        match word.as_str() {
            "%f" | "%F" | "%u" | "%U" => {
                insert_at.get_or_insert(words.len());
            }
            "@@" | "@@u" | "@@f" => {}
            w if w.len() == 2 && w.starts_with('%') && w != "%%" => {} // %i %c %k, deprecated codes
            w => words.push(w.replace("%%", "%")),
        }
    }
    let at = insert_at.unwrap_or(words.len());
    words.splice(at..at, extra.iter().cloned());
    let mut it = words.into_iter();
    let program = it.next()?;
    Some((PathBuf::from(program), it.collect()))
}

/// The spec's quoting: words split on spaces; inside double quotes a
/// backslash escapes `"`, `` ` ``, `$` and `\`.
fn split_exec(exec: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => word.push(chars.next()?),
                        c => word.push(c),
                    }
                }
            }
            ' ' | '\t' => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    Some(words)
}

/// The spec's string escapes: `\s`, `\n`, `\t`, `\r`, `\\`.
fn unescape_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// The file name of a desktop entry path, i.e. its desktop id.
pub(crate) fn id_of(path: &Path) -> Option<&str> {
    path.file_name()?.to_str()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn native_exec() {
        let (p, a) = exec_command("/usr/bin/google-chrome-stable %U", &s(&["https://x"])).unwrap();
        assert_eq!(p, PathBuf::from("/usr/bin/google-chrome-stable"));
        assert_eq!(a, s(&["https://x"]));
    }

    #[test]
    fn flatpak_exec() {
        let exec = "/usr/bin/flatpak run --branch=stable --arch=x86_64 --command=firefox \
                    --file-forwarding org.mozilla.firefox @@u %u @@";
        let (p, a) = exec_command(exec, &s(&["-P", "Work", "https://x"])).unwrap();
        assert_eq!(p, PathBuf::from("/usr/bin/flatpak"));
        assert_eq!(
            a,
            s(&[
                "run",
                "--branch=stable",
                "--arch=x86_64",
                "--command=firefox",
                "--file-forwarding",
                "org.mozilla.firefox",
                "-P",
                "Work",
                "https://x"
            ])
        );
    }

    #[test]
    fn quoting_and_codes() {
        let exec = r#""/opt/My Browser/browser" --name "a \"b\"" %i %c 100%% %U --after"#;
        let (p, a) = exec_command(exec, &s(&["U"])).unwrap();
        assert_eq!(p, PathBuf::from("/opt/My Browser/browser"));
        assert_eq!(a, s(&["--name", "a \"b\"", "100%", "U", "--after"]));
        assert_eq!(exec_command(r#""unterminated"#, &[]), None);
        assert_eq!(exec_command("  ", &[]), None);
    }

    #[test]
    fn parses_main_group_only() {
        let text = "[Desktop Entry]\nType=Application\nName=Firefox\nExec=firefox %u\n\
                    MimeType=text/html;x-scheme-handler/http;x-scheme-handler/https;\n\
                    [Desktop Action new-window]\nName=New Window\nExec=firefox --new-window %u\n";
        let e = DesktopEntry::parse(text).unwrap();
        assert_eq!(
            (e.name.as_str(), e.exec.as_str()),
            ("Firefox", "firefox %u")
        );
        assert!(e.handles_https());
        assert!(DesktopEntry::parse("[Desktop Entry]\nType=Link\nName=x\nExec=y\n").is_none());
    }
}
