//! Pasting images and files into a composer.
//!
//! GPUI reads the clipboard as text first and only falls back to an image when there is no
//! text at all, and it can't see file lists a file manager copies (`text/uri-list`). So when a
//! paste carries nothing GPUI understood, on Linux we ask the clipboard tool of the session
//! (`wl-paste` on Wayland, `xclip` on X11) what's there.

use gpui_kit::*;
use mothr_core::save_image;
use std::path::PathBuf;

/// Images and files on the clipboard, ready to attach. Empty when the clipboard holds plain
/// text (the text box pastes that itself) or nothing usable.
pub fn attachments_from_clipboard(cx: &App) -> Vec<PathBuf> {
    let mut out = vec![];
    if let Some(item) = cx.read_from_clipboard() {
        for entry in item.entries() {
            match entry {
                ClipboardEntry::Image(img) => {
                    let mime = match img.format {
                        ImageFormat::Png => "image/png",
                        ImageFormat::Jpeg => "image/jpeg",
                        ImageFormat::Webp => "image/webp",
                        ImageFormat::Gif => "image/gif",
                        _ => continue,
                    };
                    match save_image(&img.bytes, mime) {
                        Ok(path) => out.push(path),
                        Err(e) => tracing::warn!("couldn't store pasted image: {e}"),
                    }
                }
                ClipboardEntry::ExternalPaths(paths) => out.extend(paths.paths().iter().cloned()),
                ClipboardEntry::String(_) => {}
            }
        }
        if !out.is_empty() {
            return out;
        }
        if let Some(text) = item.text().filter(|t| !t.trim().is_empty()) {
            // Real text pastes as text. Some file managers copy files as `file://` lines.
            return paths_from_uri_list(&text);
        }
    }
    external_clipboard()
}

/// Existing files named by `file://` lines (`text/uri-list`, GNOME's copied-files format).
/// Empty unless every line is such a path.
fn paths_from_uri_list(text: &str) -> Vec<PathBuf> {
    let mut out = vec![];
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        if matches!(line, "copy" | "cut") {
            continue;
        }
        let Some(rest) = line.strip_prefix("file://") else { return vec![] };
        // `file://localhost/path` and `file:///path` both end at the path.
        let rest = rest.strip_prefix("localhost").unwrap_or(rest);
        let path = PathBuf::from(percent_decode(rest));
        if !path.exists() {
            return vec![];
        }
        out.push(path);
    }
    out
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(target_os = "linux")]
fn external_clipboard() -> Vec<PathBuf> {
    use std::process::Command;
    use std::sync::mpsc;
    use std::time::Duration;

    /// Run a command, giving up after a second and a half (a hung clipboard owner).
    fn run(program: &'static str, args: Vec<String>) -> Option<Vec<u8>> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(Command::new(program).args(args).output().ok().filter(|o| o.status.success()).map(|o| o.stdout));
        });
        rx.recv_timeout(Duration::from_millis(1500)).ok().flatten()
    }

    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
    type ReadArgs = fn(&str) -> Vec<String>;
    let (program, list_args, read_args): (&'static str, Vec<String>, ReadArgs) = if wayland {
        ("wl-paste", vec!["--list-types".into()], |t| vec!["--no-newline".into(), "--type".into(), t.into()])
    } else if std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty()) {
        (
            "xclip",
            vec!["-selection".into(), "clipboard".into(), "-t".into(), "TARGETS".into(), "-o".into()],
            |t| vec!["-selection".into(), "clipboard".into(), "-t".into(), t.into(), "-o".into()],
        )
    } else {
        return vec![];
    };
    let Some(types) = run(program, list_args) else { return vec![] };
    let types = String::from_utf8_lossy(&types);
    let has = |t: &str| types.lines().any(|l| l.trim() == t);

    for mime in ["image/png", "image/jpeg", "image/webp", "image/gif"] {
        if has(mime)
            && let Some(bytes) = run(program, read_args(mime)).filter(|b| !b.is_empty())
        {
            match save_image(&bytes, mime) {
                Ok(path) => return vec![path],
                Err(e) => tracing::warn!("couldn't store pasted image: {e}"),
            }
        }
    }
    for list in ["text/uri-list", "x-special/gnome-copied-files"] {
        if has(list)
            && let Some(bytes) = run(program, read_args(list))
        {
            let paths = paths_from_uri_list(&String::from_utf8_lossy(&bytes));
            if !paths.is_empty() {
                return paths;
            }
        }
    }
    vec![]
}

#[cfg(not(target_os = "linux"))]
fn external_clipboard() -> Vec<PathBuf> {
    vec![]
}

#[cfg(test)]
mod tests {
    use super::paths_from_uri_list;

    #[test]
    fn uri_lists_become_existing_paths() {
        let dir = std::env::temp_dir();
        let file = dir.join("mothr clip test.txt");
        std::fs::write(&file, "x").unwrap();
        let uri = format!("copy\nfile://{}\n", file.display().to_string().replace(' ', "%20"));
        assert_eq!(paths_from_uri_list(&uri), vec![file.clone()]);
        // Anything else is text, and so is a list naming a file that isn't there.
        assert!(paths_from_uri_list("hello world").is_empty());
        assert!(paths_from_uri_list("file:///definitely/not/here.png").is_empty());
        let _ = std::fs::remove_file(file);
    }
}
