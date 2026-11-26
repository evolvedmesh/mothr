//! Plain helpers for the file browser: a Markdown document's headings, and where a link in it
//! leads.

use std::ops::Range;

/// One ATX heading (`## Title`) of a Markdown document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    /// Byte range of the heading's text in the source.
    pub range: Range<usize>,
    /// GitHub-style anchor (`#getting-started`).
    pub slug: String,
}

/// The headings of a Markdown document, skipping fenced code blocks.
pub fn headings(source: &str) -> Vec<Heading> {
    let mut out = Vec::new();
    let mut fence: Option<&str> = None;
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let start = offset;
        offset += line.len();
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if indent > 3 {
            continue;
        }
        for marker in ["```", "~~~"] {
            if trimmed.starts_with(marker) {
                fence = match fence {
                    Some(m) if m == marker => None,
                    None => Some(marker),
                    other => other,
                };
            }
        }
        if fence.is_some() || !trimmed.starts_with('#') {
            continue;
        }
        let level = trimmed.bytes().take_while(|&b| b == b'#').count();
        let rest = &trimmed[level..];
        if level > 6 || !(rest.starts_with(' ') || rest.starts_with('\t') || rest.trim().is_empty()) {
            continue;
        }
        // Text without the closing hashes (`## Title ##`).
        let body = rest.trim_end_matches(['\n', '\r']).trim();
        let unclosed = body.trim_end_matches('#');
        let text = if unclosed.len() < body.len() && (unclosed.is_empty() || unclosed.ends_with(' ')) { unclosed.trim_end() } else { body };
        if text.is_empty() {
            continue;
        }
        let text_start = start + indent + level + (rest.len() - rest.trim_start().len());
        out.push(Heading {
            level: level as u8,
            text: plain(text),
            range: text_start..text_start + text.len(),
            slug: slug(text),
        });
    }
    // Repeated anchors get -1, -2 … as on GitHub.
    let mut seen: std::collections::HashMap<String, usize> = Default::default();
    for h in &mut out {
        let n = seen.entry(h.slug.clone()).or_insert(0);
        if *n > 0 {
            h.slug = format!("{}-{}", h.slug, n);
        }
        *n += 1;
    }
    out
}

/// Heading text without inline Markdown markers.
fn plain(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' | '_' | '`' => {}
            // [label](url) → label
            '[' => {
                let mut label = String::new();
                for c in chars.by_ref() {
                    if c == ']' {
                        break;
                    }
                    label.push(c);
                }
                if chars.peek() == Some(&'(') {
                    for c in chars.by_ref() {
                        if c == ')' {
                            break;
                        }
                    }
                }
                s.push_str(&label);
            }
            c => s.push(c),
        }
    }
    s
}

/// GitHub's heading anchor: lowercase, punctuation dropped, spaces as dashes.
pub fn slug(text: &str) -> String {
    plain(text)
        .trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

/// Where a link in a document leads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Somewhere on the web (or another app): open it outside Mothr.
    External(String),
    /// A heading in the same document.
    Anchor(String),
    /// A file or folder of the project (relative path), with an optional heading.
    Project { path: String, anchor: Option<String> },
    /// A path that would leave the project.
    Outside,
}

/// Resolve `url` from a document at `from` (relative path of the file that links).
pub fn resolve_link(url: &str, from: &str) -> Target {
    let url = url.trim();
    if let Some(anchor) = url.strip_prefix('#') {
        return Target::Anchor(anchor.to_string());
    }
    let scheme = url.split_once(':').map(|(s, _)| s);
    if scheme.is_some_and(|s| s.len() > 1 && s.chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))) {
        return Target::External(url.to_string());
    }
    let (path, anchor) = match url.split_once('#') {
        Some((p, a)) => (p, Some(a.to_string()).filter(|a| !a.is_empty())),
        None => (url, None),
    };
    let path = path.split('?').next().unwrap_or(path);
    let path = decode(path);
    let base: Vec<&str> = if path.starts_with('/') {
        vec![]
    } else {
        let dir = from.rsplit_once('/').map_or("", |(d, _)| d);
        dir.split('/').filter(|s| !s.is_empty()).collect()
    };
    let mut parts: Vec<String> = base.into_iter().map(str::to_string).collect();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Target::Outside;
                }
            }
            s => parts.push(s.to_string()),
        }
    }
    Target::Project { path: parts.join("/"), anchor }
}

/// `%20`-style escapes, as links to files with spaces use them.
fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_headings_outside_code() {
        let src = "# Mothr\n\nIntro\n\n## Getting **started**\n\n```sh\n# not a heading\n```\n\n### Build ##\n#nope\n";
        let h = headings(src);
        let texts: Vec<&str> = h.iter().map(|h| h.text.as_str()).collect();
        assert_eq!(texts, ["Mothr", "Getting started", "Build"]);
        assert_eq!(h[1].level, 2);
        assert_eq!(&src[h[1].range.clone()], "Getting **started**");
        assert_eq!(h[1].slug, "getting-started");
        assert_eq!(&src[h[2].range.clone()], "Build");
    }

    #[test]
    fn repeated_headings_get_numbered_anchors() {
        let h = headings("## Usage\n## Usage\n");
        assert_eq!(h[1].slug, "usage-1");
    }

    #[test]
    fn links_resolve_inside_the_project() {
        assert_eq!(resolve_link("https://mothr.dev", "README.md"), Target::External("https://mothr.dev".into()));
        assert_eq!(resolve_link("#build", "README.md"), Target::Anchor("build".into()));
        assert_eq!(
            resolve_link("../DESIGN.md#colors", "docs/guide.md"),
            Target::Project { path: "DESIGN.md".into(), anchor: Some("colors".into()) }
        );
        assert_eq!(resolve_link("./a%20b.md", "docs/x.md"), Target::Project { path: "docs/a b.md".into(), anchor: None });
        assert_eq!(resolve_link("/crates", "docs/x.md"), Target::Project { path: "crates".into(), anchor: None });
        assert_eq!(resolve_link("../../etc/passwd", "docs/x.md"), Target::Outside);
    }
}
