//! Try the skill library from the shell (use a throwaway `MOTHR_HOME`):
//!
//! ```sh
//! MOTHR_HOME=$(mktemp -d) cargo run -p mothr-skills --example skills -- search pdf
//! MOTHR_HOME=$(mktemp -d) cargo run -p mothr-skills --example skills -- browse anthropics/skills
//! MOTHR_HOME=$(mktemp -d) cargo run -p mothr-skills --example skills -- add anthropics/skills@pdf
//! cargo run -p mothr-skills --example skills -- local
//! ```

use mothr_core::SkillSource;
use mothr_skills::Library;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let lib = Library::default();
    let arg = args.get(1).cloned().unwrap_or_default();
    match args.first().map(String::as_str) {
        Some("search") => {
            for l in lib.search(&arg).await? {
                println!("{:<40} {:<32} {:>8}  {}", l.name, l.origin, l.installs.unwrap_or(0), l.description.unwrap_or_default().chars().take(70).collect::<String>());
            }
        }
        Some("browse") => {
            for l in lib.browse(&arg).await? {
                println!("{:<40} {}", l.name, l.description.unwrap_or_default().chars().take(90).collect::<String>());
            }
        }
        Some("add") => {
            let source = SkillSource::parse(&arg).map_err(anyhow::Error::msg)?;
            let done = lib.install(&source).await?;
            println!("{}", done.summary(&source));
        }
        Some("local") => {
            for l in mothr_skills::local_agent_skills() {
                println!("{:<40} {}", l.name, l.origin);
            }
        }
        _ => {}
    }
    for s in lib.load() {
        println!("installed: /{} ({}) {}", s.name, s.source.map(|s| s.label()).unwrap_or_default(), s.dir.display());
    }
    Ok(())
}
