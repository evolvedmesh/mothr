//! Design mode: agents author design systems and UI mockups as JSON that Mothr renders
//! natively. See [`model`] for the format and [`INSTRUCTIONS`] for the agent-facing guide.

pub mod export;
pub mod model;

pub use model::*;

use notify::{RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Folder (relative to the project root) that holds design files.
pub const DESIGN_DIR: &str = "mothr-design";

/// Everything loaded from a design folder.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DesignDoc {
    pub system: Option<DesignSystem>,
    pub screens: Vec<Screen>,
    /// Parse errors to surface in the canvas (file: message).
    pub errors: Vec<String>,
}

impl DesignDoc {
    pub fn is_empty(&self) -> bool {
        self.system.is_none() && self.screens.is_empty()
    }

    pub fn tokens(&self) -> Tokens {
        self.system.as_ref().map(|s| s.tokens.clone()).unwrap_or_default()
    }

    pub fn component(&self, name: &str) -> Option<&Component> {
        let sys = self.system.as_ref()?;
        let norm = |s: &str| s.to_ascii_lowercase().replace([' ', '/', '-', '_'], "");
        let want = norm(name);
        sys.components.iter().find(|c| norm(&c.name) == want)
    }

    /// Resolve a color string (`#hex`, `rgb()`, or `$token`) to 0xRRGGBBAA.
    pub fn color(&self, value: &str) -> Option<u32> {
        let tokens = self.system.as_ref().map(|s| &s.tokens);
        resolve_color(tokens, value, 0)
    }

    pub fn length(&self, len: &Length, scale: TokenScale) -> f32 {
        match len {
            Length::Px(v) => *v,
            Length::Token(name) => {
                let Some(sys) = &self.system else { return 0.0 };
                let map = match scale {
                    TokenScale::Spacing => &sys.tokens.spacing,
                    TokenScale::Radius => &sys.tokens.radii,
                };
                map.get(name)
                    .or_else(|| sys.tokens.spacing.get(name))
                    .or_else(|| sys.tokens.radii.get(name))
                    .map(|v| v.0)
                    .unwrap_or(0.0)
            }
        }
    }

    pub fn type_style(&self, name: &str) -> Option<&TypeStyle> {
        let name = name.trim_start_matches('$');
        self.system.as_ref()?.tokens.typography.get(name)
    }

    pub fn shadow(&self, name: &str) -> Option<&Shadow> {
        let name = name.trim_start_matches('$');
        self.system.as_ref()?.tokens.shadows.get(name)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum TokenScale {
    Spacing,
    Radius,
}

fn resolve_color(tokens: Option<&Tokens>, value: &str, depth: u8) -> Option<u32> {
    let v = value.trim();
    if let Some(name) = v.strip_prefix('$') {
        if depth > 4 {
            return None;
        }
        let raw = tokens?.colors.get(name)?;
        return resolve_color(tokens, raw, depth + 1);
    }
    parse_color(v)
}

/// The design folder for a project.
pub fn design_root(project: &Path) -> PathBuf {
    let root = project.join(DESIGN_DIR);
    // Projects designed before the rename keep working from their old folder.
    let legacy = project.join(LEGACY_DESIGN_DIR);
    if !root.exists() && legacy.is_dir() { legacy } else { root }
}

/// Design folder name used before Shepherd was renamed to Mothr.
pub const LEGACY_DESIGN_DIR: &str = "shepherd-design";

/// Load `system.json` and `screens/*.json` (sorted by file name).
pub fn load(project: &Path) -> DesignDoc {
    let root = design_root(project);
    let mut doc = DesignDoc::default();
    let sys_path = root.join("system.json");
    if let Ok(text) = std::fs::read_to_string(&sys_path) {
        match serde_json::from_str::<DesignSystem>(&text) {
            Ok(s) => doc.system = Some(s),
            Err(e) => doc.errors.push(format!("system.json: {e}")),
        }
    }
    let screens_dir = root.join("screens");
    if let Ok(read) = std::fs::read_dir(&screens_dir) {
        let mut files: Vec<PathBuf> = read
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        files.sort();
        for path in files {
            let fname = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
            match std::fs::read_to_string(&path).map_err(|e| e.to_string()).and_then(|t| {
                serde_json::from_str::<Screen>(&t).map_err(|e| e.to_string())
            }) {
                Ok(mut s) => {
                    if s.name.is_empty() {
                        s.name = fname.trim_end_matches(".json").replace(['-', '_'], " ");
                    }
                    s.file = fname;
                    doc.screens.push(s);
                }
                Err(e) => doc.errors.push(format!("screens/{fname}: {e}")),
            }
        }
    }
    doc
}

/// Create the design folder with a starter `system.json` if it doesn't exist yet.
pub fn ensure_scaffold(project: &Path, name: &str) -> anyhow::Result<PathBuf> {
    let root = design_root(project);
    std::fs::create_dir_all(root.join("screens"))?;
    let sys = root.join("system.json");
    if !sys.exists() {
        let starter = serde_json::json!({
            "name": name,
            "description": "Describe the product and the feel of this design system.",
            "tokens": { "colors": {}, "typography": {}, "spacing": {}, "radii": {}, "shadows": {} },
            "components": []
        });
        std::fs::write(&sys, serde_json::to_string_pretty(&starter)?)?;
    }
    let readme = root.join("README.md");
    if !readme.exists() {
        std::fs::write(&readme, INSTRUCTIONS)?;
    }
    Ok(root)
}

/// Watch the design folder; sends `()` (debounced ~150ms) whenever a file changes.
/// Keep the returned watcher alive for as long as you want notifications.
pub fn watch(project: &Path) -> anyhow::Result<(notify::RecommendedWatcher, async_channel::Receiver<()>)> {
    let root = design_root(project);
    std::fs::create_dir_all(&root)?;
    let (raw_tx, raw_rx) = std::sync::mpsc::channel::<()>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res
            && !matches!(ev.kind, notify::EventKind::Access(_)) {
                let _ = raw_tx.send(());
            }
    })?;
    watcher.watch(&root, RecursiveMode::Recursive)?;
    let (tx, rx) = async_channel::bounded(4);
    std::thread::Builder::new()
        .name("design-watch".into())
        .spawn(move || {
            while raw_rx.recv().is_ok() {
                // Debounce bursts of writes from the agent.
                while raw_rx.recv_timeout(Duration::from_millis(150)).is_ok() {}
                if tx.send_blocking(()).is_err() {
                    break;
                }
            }
        })?;
    Ok((watcher, rx))
}

/// Guide given to the agent at the start of every Design session.
pub const INSTRUCTIONS: &str = r##"# Mothr Design mode

You are working inside Mothr's Design mode. The user sees a live, native canvas that renders
the JSON files in `mothr-design/` (relative to the working directory) every time you save.
Do NOT write HTML/CSS/React for the mockups — write these JSON files. Edit them incrementally
with your file tools; the canvas hot-reloads.

## Files
- `mothr-design/system.json` — the design system: tokens + reusable components.
- `mothr-design/screens/<nn>-<slug>.json` — one mockup screen per file (prefix with a number
  to control order, e.g. `01-onboarding.json`).

## system.json
```json
{
  "name": "Acme",
  "description": "Calm, precise fintech UI",
  "tokens": {
    "colors": { "bg": "#0B0B0F", "surface": "#15151C", "text": "#F4F4F5", "muted": "#A1A1AA",
                "primary": "#6366F1", "on-primary": "#FFFFFF", "border": "#27272A" },
    "typography": { "display": {"size": 40, "weight": 700, "line_height": 1.1},
                    "title": {"size": 22, "weight": 600}, "body": {"size": 15, "weight": 400},
                    "caption": {"size": 12, "weight": 500} },
    "spacing": { "xs": 4, "sm": 8, "md": 16, "lg": 24, "xl": 40 },
    "radii": { "sm": 6, "md": 10, "lg": 16, "full": 999 },
    "shadows": { "md": {"x": 0, "y": 8, "blur": 24, "color": "#00000055"} }
  },
  "components": [
    { "name": "Button / Primary", "description": "Main call to action",
      "variants": [
        { "name": "Default", "node": { "type": "button", "text": "Continue",
          "style": {"bg": "$primary", "fg": "$on-primary", "radius": "$md", "padding": [12, 20], "font": "$body", "weight": 600} } }
      ] }
  ]
}
```
Color values may reference other tokens with `$name`.

## Screens
```json
{ "name": "Home", "device": "mobile", "background": "$bg",
  "root": { "type": "column", "style": {"padding": "$lg", "gap": "$md", "width": "fill", "height": "fill"},
    "children": [
      { "type": "text", "text": "Good morning", "style": {"font": "$display", "fg": "$text"} },
      { "type": "ref", "ref": "Button / Primary", "overrides": {"text": "Get started"} }
    ] } }
```
`device`: desktop (1280×800), tablet, mobile (390×844), watch — or give `width`/`height`.

## Nodes
`type`: box (flex container, `style.direction` row|column), row, column, stack (overlapping
children positioned with `style.x`/`style.y`), text, icon (`icon`: a Lucide icon name such as
"search", "bell", "settings", "user", "home", "plus", "chevron-right", "heart", "star"), image
(placeholder block, `placeholder` = alt text), spacer, divider, input (`placeholder`, `text`),
button (`text`, `icon`), switch / checkbox (`checked`), progress (`value` 0..1), ref (component
instance: `ref`, optional `variant`, `overrides`).

Overrides customise copy inside an instance: give nodes inside the component a `name`
(e.g. `"name": "title"`, `"name": "price"`) and pass `"overrides": {"title": "Honey Oat Latte",
"price": "$5.20"}`. The special key `"text"` replaces the text of the first text-bearing node.

## Style keys
width/height (px | "fill" | "hug" | "50%"), min_width, max_width, min_height, padding/margin
(n | [v, h] | [t, r, b, l] | "$token"), gap, direction, align (start|center|end|stretch),
justify (start|center|end|between|around), wrap, grow, bg, gradient (["#a", "#b"]), fg,
radius, border {"width", "color"}, shadow ("$md"), font ("$body"), size, weight, italic,
text_align, opacity, overflow ("hidden").

## Workflow
1. Ask the user clarifying questions when the brief is vague (product, audience, mood, platforms).
2. Establish tokens and core components in system.json first, then build screens from components.
3. Keep JSON valid. After each change, briefly tell the user what changed and what to look at.
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instructions_examples_parse() {
        // The examples embedded in INSTRUCTIONS must parse with the real model.
        let blocks: Vec<&str> = INSTRUCTIONS.split("```json").skip(1).map(|b| b.split("```").next().unwrap()).collect();
        let sys: DesignSystem = serde_json::from_str(blocks[0]).unwrap();
        assert_eq!(sys.components.len(), 1);
        let screen: Screen = serde_json::from_str(blocks[1]).unwrap();
        assert_eq!(screen.root.children.len(), 2);
        let doc = DesignDoc { system: Some(sys), screens: vec![screen], errors: vec![] };
        assert_eq!(doc.color("$primary"), Some(0x6366F1FF));
        assert!(doc.component("button/primary").is_some());
        assert_eq!(doc.length(&Length::Token("md".into()), TokenScale::Radius), 10.0);
    }
}
