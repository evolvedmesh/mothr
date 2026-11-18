//! App assets: gpui-kit's catalog (Lucide icons and component art) plus Mothr's own files.

use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

/// Agent logos (see assets/logos/NOTICE.md), served at `logos/<agent id>.svg`.
const LOGOS: &[(&str, &[u8])] = &[
    ("logos/claude_code.svg", include_bytes!("../assets/logos/claude_code.svg")),
    ("logos/codex.svg", include_bytes!("../assets/logos/codex.svg")),
    ("logos/copilot.svg", include_bytes!("../assets/logos/copilot.svg")),
    ("logos/open_code.svg", include_bytes!("../assets/logos/open_code.svg")),
    ("logos/hermes.svg", include_bytes!("../assets/logos/hermes.svg")),
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if let Some((_, bytes)) = LOGOS.iter().find(|(p, _)| *p == path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_kit::assets::AllAssets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut out = gpui_kit::assets::AllAssets.list(path)?;
        out.extend(LOGOS.iter().filter(|(p, _)| p.starts_with(path)).map(|(p, _)| SharedString::from(*p)));
        Ok(out)
    }
}
