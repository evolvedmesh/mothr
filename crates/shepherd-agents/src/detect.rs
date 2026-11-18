//! Find agent installations on this machine and decide how to talk to each.

use shepherd_core::{AgentInstall, AgentKind, AgentOverride, Transport};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Detect every first-party agent (in parallel), applying user overrides.
pub fn detect_all(overrides: &BTreeMap<AgentKind, AgentOverride>) -> Vec<AgentInstall> {
    let handles: Vec<_> = AgentKind::ALL
        .into_iter()
        .map(|kind| {
            let ov = overrides.get(&kind).cloned().unwrap_or_default();
            std::thread::spawn(move || detect(kind, &ov))
        })
        .collect();
    handles.into_iter().filter_map(|h| h.join().ok()).collect()
}

pub fn detect(kind: AgentKind, ov: &AgentOverride) -> AgentInstall {
    let mut install = AgentInstall {
        kind,
        binary: None,
        version: None,
        transport: None,
        note: None,
    };
    if ov.disabled {
        install.note = Some("Disabled in settings".into());
        return install;
    }
    let env: Vec<(String, String)> = ov.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

    // Fully custom command from settings.
    if let (Some(program), Some(args)) = (&ov.program, &ov.args) {
        install.binary = Some(program.clone());
        install.version = version_of(program);
        install.transport = Some(Transport::Acp { program: program.clone(), args: args.clone(), env });
        install.note = Some("Custom command from settings".into());
        return install;
    }

    let binary = ov.program.clone().or_else(|| find_cli(kind));
    install.binary = binary.clone();
    if let Some(b) = &binary {
        install.version = version_of(b);
    }

    install.transport = match kind {
        AgentKind::ClaudeCode => binary.clone().map(|program| Transport::ClaudeStreamJson { program }),
        AgentKind::Copilot => binary.clone().map(|b| {
            let native = native_copilot(&b);
            if let Some(n) = &native {
                install.version = install.version.take().or_else(|| version_of(n));
            }
            Transport::Acp {
            program: native.unwrap_or(b),
            args: vec!["--acp".into()],
            env: env.clone(),
        }}),
        AgentKind::OpenCode => binary.clone().map(|program| Transport::Acp {
            program,
            args: vec!["acp".into()],
            env: env.clone(),
        }),
        AgentKind::Hermes => binary.clone().map(|program| Transport::Acp {
            program,
            args: vec!["acp".into()],
            env: env.clone(),
        }),
        AgentKind::Codex => {
            // Codex speaks ACP through the codex-acp adapter.
            if let Some(adapter) = find_in_path(&["codex-acp"]) {
                Some(Transport::Acp { program: adapter, args: vec![], env: env.clone() })
            } else if binary.is_some() {
                match package_runner() {
                    Some((runner, mut args)) => {
                        args.push("@zed-industries/codex-acp".into());
                        install.note = Some(format!(
                            "Using the codex-acp adapter via {}",
                            runner.file_name().unwrap_or_default().to_string_lossy()
                        ));
                        Some(Transport::Acp { program: runner, args, env: env.clone() })
                    }
                    None => {
                        install.note = Some("Install `codex-acp` (or Node/Bun) so Shepherd can talk to Codex".into());
                        None
                    }
                }
            } else {
                None
            }
        }
    };

    if install.binary.is_none() && install.transport.is_none() && install.note.is_none() {
        install.note = Some(format!("{} was not found on this system", kind.display_name()));
    }
    install
}

/// CLI names and well-known install locations for each agent.
fn find_cli(kind: AgentKind) -> Option<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    let (names, extra): (&[&str], Vec<PathBuf>) = match kind {
        AgentKind::ClaudeCode => (
            &["claude"],
            vec![
                home.join(".claude/local/claude"),
                home.join(".local/bin/claude"),
                PathBuf::from("/opt/claude-code/bin/claude"),
                home.join(".local/bin/claude.exe"),
            ],
        ),
        AgentKind::Codex => (&["codex"], vec![home.join(".codex/bin/codex")]),
        AgentKind::Copilot => (&["copilot"], vec![]),
        AgentKind::OpenCode => (&["opencode"], vec![home.join(".opencode/bin/opencode")]),
        AgentKind::Hermes => (&["hermes"], vec![PathBuf::from("/opt/hermes-agent/bin/hermes"), home.join(".hermes/bin/hermes")]),
    };
    find_in_path(names).or_else(|| extra.into_iter().find(|p| is_executable(p)))
}

fn find_in_path(names: &[&str]) -> Option<PathBuf> {
    let home = dirs::home_dir().unwrap_or_default();
    // GUI apps often launch without the user's shell PATH; add common bin dirs.
    let extra_dirs = [
        home.join(".local/bin"),
        home.join(".bun/bin"),
        home.join(".npm-global/bin"),
        home.join(".cargo/bin"),
        home.join(".volta/bin"),
        home.join("go/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/usr/bin"),
        home.join("AppData/Roaming/npm"),
    ];
    let mut path = std::env::var_os("PATH").unwrap_or_default();
    for d in &extra_dirs {
        if d.is_dir() {
            let mut joined = std::env::split_paths(&path).collect::<Vec<_>>();
            joined.push(d.clone());
            path = std::env::join_paths(joined).unwrap_or(path);
        }
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    names
        .iter()
        .find_map(|n| which::which_in(n, Some(&path), &cwd).ok())
}

fn is_executable(p: &Path) -> bool {
    p.is_file()
}

/// The Copilot npm package launches a native binary through Node. Prefer the native binary
/// directly so a broken/missing Node install doesn't block Copilot.
fn native_copilot(cli: &Path) -> Option<PathBuf> {
    let real = std::fs::canonicalize(cli).ok()?;
    if real.extension().is_some_and(|e| e == "js") || is_script(&real) {
        let pkg_dir = real.parent()?;
        let scoped = pkg_dir.join("node_modules/@github");
        let entries = std::fs::read_dir(&scoped).ok()?;
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("copilot-") {
                for exe in ["copilot", "copilot.exe"] {
                    let p = e.path().join(exe);
                    if p.is_file() {
                        return Some(p);
                    }
                }
            }
        }
    }
    None
}

fn is_script(p: &Path) -> bool {
    std::fs::read(p)
        .map(|b| b.starts_with(b"#!"))
        .unwrap_or(false)
}

/// `npx -y` when Node works, else `bunx`.
fn package_runner() -> Option<(PathBuf, Vec<String>)> {
    if let Some(npx) = find_in_path(&["npx"])
        && find_in_path(&["node"]).is_some_and(|n| run_with_timeout(&n, &["--version"], Duration::from_secs(5)).is_some()) {
            return Some((npx, vec!["-y".into()]));
        }
    find_in_path(&["bunx"]).map(|b| (b, vec![]))
}

fn version_of(binary: &Path) -> Option<String> {
    let out = run_with_timeout(binary, &["--version"], Duration::from_secs(6))?;
    let line = out.lines().find(|l| !l.trim().is_empty())?.trim().to_string();
    // Pull the first token that looks like a version number.
    let v = line
        .split(|c: char| c.is_whitespace() || c == '(' || c == ')')
        .find(|t| t.trim_start_matches('v').chars().next().is_some_and(|c| c.is_ascii_digit()) && t.contains('.'))
        .map(|t| t.trim_start_matches('v').trim_end_matches('.').to_string());
    Some(v.unwrap_or(line))
}

fn run_with_timeout(program: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut s = String::new();
                use std::io::Read;
                child.stdout.take()?.read_to_string(&mut s).ok()?;
                return Some(s);
            }
            Ok(None) if start.elapsed() < timeout => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                return None;
            }
        }
    }
}
