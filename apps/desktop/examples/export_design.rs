//! Export a project's design without opening the app.
//!
//! `cargo run -p mothr-app --example export_design -- <project> <out.html> [storybook-dir] [--build]`

use gpui_kit::AssetSource as _;
use mothr_design::export::{ExportAssets, FontFile, build_storybook, export_html, export_storybook, find_js_runtime};
use std::path::PathBuf;

fn assets() -> ExportAssets {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/fonts");
    let fonts = [("Geist", 400), ("Geist", 500), ("Geist", 600), ("Geist", 700), ("Geist Mono", 400), ("Geist Mono", 500), ("Geist Mono", 600)]
        .into_iter()
        .filter_map(|(family, weight)| {
            let file_name = format!("{}-{weight}.ttf", family.replace(' ', ""));
            let data = std::fs::read(dir.join(&file_name)).ok()?;
            Some(FontFile { family: family.into(), weight, italic: false, file_name, data })
        })
        .collect();
    ExportAssets {
        fonts,
        icons: Some(Box::new(|name| {
            let bytes = gpui_kit::assets::AllAssets.load(&format!("icons/{name}.svg")).ok().flatten()?;
            Some(String::from_utf8_lossy(&bytes).into_owned())
        })),
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let project = PathBuf::from(args.first().map(String::as_str).unwrap_or("."));
    let out = PathBuf::from(args.get(1).map(String::as_str).unwrap_or("design.html"));
    let doc = mothr_design::load(&project);
    anyhow::ensure!(!doc.is_empty(), "no design in {}", project.display());
    let name = project.canonicalize()?.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let assets = assets();
    std::fs::write(&out, export_html(&doc, &name, &assets))?;
    println!("html: {} ({} bytes)", out.display(), std::fs::metadata(&out)?.len());
    if let Some(sb) = args.get(2).filter(|a| !a.starts_with("--")) {
        let sb = PathBuf::from(sb);
        let files = export_storybook(&doc, &name, &assets, &sb)?;
        println!("storybook: {} ({} files)", sb.display(), files.len());
        if args.iter().any(|a| a == "--build") {
            let rt = find_js_runtime().ok_or_else(|| anyhow::anyhow!("no working bun or npm"))?;
            println!("building with {}", rt.program.display());
            println!("static: {}", build_storybook(&sb, &rt)?.display());
        }
    }
    Ok(())
}
