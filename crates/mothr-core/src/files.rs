//! Browsing a project's files: directory listings and file contents, shared by the desktop's
//! Projects page, the remote server and the mobile client.
//!
//! Paths inside a project are relative and `/`-separated (`""` is the project root), so they
//! read the same on every platform and can travel over the remote API unchanged.

use serde::{Deserialize, Serialize};

/// Files larger than this are shown truncated.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Directory listings stop after this many entries.
pub const MAX_DIR_ENTRIES: usize = 2000;

/// One entry of a directory listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    /// Relative to the project root, `/`-separated.
    pub path: String,
    pub dir: bool,
    /// Bytes; 0 for directories.
    #[serde(default)]
    pub size: u64,
}

/// The contents of one directory, folders first, then files, each alphabetically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DirListing {
    /// The directory, relative to the project root (`""` for the root).
    pub path: String,
    pub entries: Vec<FileEntry>,
    /// More than [`MAX_DIR_ENTRIES`] entries; the rest were left out.
    #[serde(default)]
    pub truncated: bool,
    /// The directory's README, if it has one (relative path).
    #[serde(default)]
    pub readme: Option<String>,
}

/// One file, ready to show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileContent {
    /// Relative to the project root, `/`-separated.
    pub path: String,
    pub size: u64,
    /// Syntax name for highlighting ("rust", "markdown", "text" …).
    pub language: String,
    /// The text, or `None` for binary files.
    pub text: Option<String>,
    /// Only the first [`MAX_FILE_BYTES`] are included.
    #[serde(default)]
    pub truncated: bool,
}

impl FileContent {
    pub fn name(&self) -> &str {
        file_name(&self.path)
    }

    pub fn is_markdown(&self) -> bool {
        self.language == "markdown"
    }
}

/// A fuzzy file-name match inside a project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileHit {
    /// Relative to the project root, `/`-separated.
    pub path: String,
    /// Char indices matched in `path`.
    #[serde(default)]
    pub indices: Vec<u32>,
}

/// The last component of a `/`-separated path.
pub fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// The parent of a `/`-separated path (`""` for top-level entries).
pub fn parent_path(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(p, _)| p)
}

/// Whether a file name is a Markdown document.
pub fn is_markdown_name(name: &str) -> bool {
    language_for(name) == "markdown"
}

/// Whether a file name looks like a README.
pub fn is_readme(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "readme" || lower.starts_with("readme.")
}

/// Syntax name for a file, from its name or extension. Unknown files are `"text"`.
pub fn language_for(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "dockerfile" | "containerfile" => return "dockerfile",
        "makefile" | "gnumakefile" => return "make",
        "cmakelists.txt" => return "cmake",
        "cargo.lock" | "uv.lock" | "poetry.lock" => return "toml",
        ".bashrc" | ".zshrc" | ".profile" | ".bash_profile" => return "bash",
        ".gitignore" | ".dockerignore" | ".ignore" => return "text",
        "readme" => return "markdown",
        _ => {}
    }
    let Some((_, ext)) = lower.rsplit_once('.') else {
        return "text";
    };
    match ext {
        "rs" => "rust",
        "md" | "markdown" | "mdx" | "mdown" => "markdown",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "json" | "jsonc" | "json5" | "webmanifest" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "py" | "pyi" | "pyw" => "python",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" => "php",
        "sh" | "bash" | "zsh" | "fish" => "bash",
        "html" | "htm" | "xhtml" => "html",
        "css" | "scss" | "sass" | "less" => "css",
        "xml" | "svg" | "plist" | "xaml" | "csproj" => "xml",
        "sql" => "sql",
        "lua" => "lua",
        "zig" => "zig",
        "ex" | "exs" => "elixir",
        "erl" | "hrl" => "erlang",
        "hs" => "haskell",
        "scala" | "sc" => "scala",
        "dart" => "dart",
        "vue" => "vue",
        "svelte" => "svelte",
        "graphql" | "gql" => "graphql",
        "proto" => "proto",
        "ini" | "cfg" | "conf" | "env" | "properties" => "ini",
        "diff" | "patch" => "diff",
        "nix" => "nix",
        "tf" | "hcl" => "hcl",
        "r" => "r",
        "jl" => "julia",
        "ml" | "mli" => "ocaml",
        "clj" | "cljs" | "edn" => "clojure",
        "gradle" => "groovy",
        "groovy" => "groovy",
        "cmake" => "cmake",
        "txt" | "log" | "csv" | "tsv" | "lock" => "text",
        _ => "text",
    }
}

/// A short, human file size: `812 B`, `14.2 KB`, `3.1 MB`.
pub fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < KB * KB {
        format!("{:.1} KB", b / KB)
    } else if b < KB * KB * KB {
        format!("{:.1} MB", b / (KB * KB))
    } else {
        format!("{:.1} GB", b / (KB * KB * KB))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_follow_names_and_extensions() {
        assert_eq!(language_for("main.rs"), "rust");
        assert_eq!(language_for("README.md"), "markdown");
        assert_eq!(language_for("README"), "markdown");
        assert_eq!(language_for("Dockerfile"), "dockerfile");
        assert_eq!(language_for("App.KT"), "kotlin");
        assert_eq!(language_for("noext"), "text");
    }

    #[test]
    fn paths_split_on_slashes() {
        assert_eq!(file_name("a/b/c.rs"), "c.rs");
        assert_eq!(file_name("c.rs"), "c.rs");
        assert_eq!(parent_path("a/b/c.rs"), "a/b");
        assert_eq!(parent_path("c.rs"), "");
    }

    #[test]
    fn readmes_are_recognized() {
        assert!(is_readme("README.md"));
        assert!(is_readme("readme"));
        assert!(!is_readme("readmeish.rs"));
    }

    #[test]
    fn sizes_are_short() {
        assert_eq!(format_size(812), "812 B");
        assert_eq!(format_size(14_540), "14.2 KB");
    }
}
