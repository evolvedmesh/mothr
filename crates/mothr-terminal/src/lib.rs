//! Terminals beside chats: the user's own shell on a pty, started in the chat's primary repo.
//!
//! The hub owns every terminal, so the desktop, the web client and the phone all see the same
//! shell. `alacritty_terminal` runs the pty and keeps the screen grid. The desktop paints that
//! grid directly ([`Terminal::term`]); remote clients get [`TerminalScreen`] snapshots.

use alacritty_terminal::event::{Event as TermEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use mothr_core::{RGB_FLAG, TermColor, TermSpan, TerminalId, TerminalScreen};
use parking_lot::Mutex;
use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use tokio::sync::{broadcast, watch};

pub use alacritty_terminal;

const SCROLLBACK: usize = 10_000;
/// A size a viewer can't sensibly ask for is clamped to this range.
const MAX_COLS: u16 = 500;
const MAX_ROWS: u16 = 200;

/// The emulator state a viewer paints from.
pub type SharedTerm = Arc<FairMutex<Term<Listener>>>;

/// Forwards what the emulator reports (new output, a title, the shell exiting) to its terminal.
#[derive(Clone)]
pub struct Listener {
    terminal: Weak<Terminal>,
    /// Which run of the shell this is; a restarted terminal ignores the old one's last words.
    run: u64,
}

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        if let Some(t) = self.terminal.upgrade() {
            t.on_event(self.run, event);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Size {
    cols: usize,
    rows: usize,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

struct Running {
    term: SharedTerm,
    pty: EventLoopSender,
}

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.pty.send(Msg::Shutdown);
    }
}

#[derive(Default)]
struct State {
    running: Option<Running>,
    run: u64,
    title: Option<String>,
    exited: Option<String>,
    size: Option<(u16, u16)>,
    cell: (u16, u16),
}

/// One shell.
pub struct Terminal {
    /// The chat it belongs to; a new-chat draft's terminal is filed under the chat later.
    id: Mutex<TerminalId>,
    cwd: PathBuf,
    state: Mutex<State>,
    /// Bumped whenever there is something new to show.
    changed: watch::Sender<u64>,
    /// Text a program asked to put on the clipboard (OSC 52); the desktop copies it.
    clipboard: broadcast::Sender<String>,
    on_change: Arc<dyn Fn(TerminalId) + Send + Sync>,
}

impl Terminal {
    pub fn id(&self) -> TerminalId {
        *self.id.lock()
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn title(&self) -> Option<String> {
        self.state.lock().title.clone()
    }

    /// Why the shell isn't running; None while it runs.
    pub fn exited(&self) -> Option<String> {
        self.state.lock().exited.clone()
    }

    /// The emulator, while the shell runs.
    pub fn term(&self) -> Option<SharedTerm> {
        self.state.lock().running.as_ref().map(|r| r.term.clone())
    }

    /// Wakes whenever the screen, the title or the running state changes.
    pub fn subscribe(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    pub fn clipboard_requests(&self) -> broadcast::Receiver<String> {
        self.clipboard.subscribe()
    }

    /// The grid size the shell has now.
    pub fn size(&self) -> Option<(u16, u16)> {
        self.state.lock().size
    }

    fn touch(&self) {
        self.changed.send_modify(|n| *n = n.wrapping_add(1));
        (self.on_change)(self.id());
    }

    /// Start the shell, or start it again (ending the one running).
    pub fn restart(self: &Arc<Self>) {
        let mut st = self.state.lock();
        st.running = None;
        st.run += 1;
        st.title = None;
        st.exited = None;
        let (cols, rows) = st.size.unwrap_or((80, 24));
        let size = Size { cols: cols as usize, rows: rows as usize };
        let listener = Listener { terminal: Arc::downgrade(self), run: st.run };
        match spawn_shell(&self.cwd, size, st.cell, listener) {
            Ok(r) => {
                st.running = Some(r);
                st.size = Some((cols, rows));
            }
            Err(e) => st.exited = Some(format!("Couldn't start a shell: {e}")),
        }
        drop(st);
        self.touch();
    }

    /// Type into the shell (text, or the escape sequences keys send).
    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        if let Some(r) = &self.state.lock().running {
            let _ = r.pty.send(Msg::Input(bytes.into()));
        }
    }

    /// Fit the shell to a viewer's room. `cell` is the cell size in pixels, when known.
    pub fn resize(&self, cols: u16, rows: u16, cell: Option<(u16, u16)>) {
        let (cols, rows) = (cols.clamp(2, MAX_COLS), rows.clamp(1, MAX_ROWS));
        let mut st = self.state.lock();
        if let Some(c) = cell {
            st.cell = c;
        }
        if st.size == Some((cols, rows)) {
            return;
        }
        st.size = Some((cols, rows));
        if let Some(r) = &st.running {
            r.term.lock().resize(Size { cols: cols as usize, rows: rows as usize });
            let _ = r.pty.send(Msg::Resize(window_size(cols, rows, st.cell)));
        }
        drop(st);
        self.touch();
    }

    /// End the shell.
    pub fn kill(&self) {
        let mut st = self.state.lock();
        st.running = None;
        st.run += 1;
        st.exited.get_or_insert_with(|| "The shell was closed.".into());
    }

    fn on_event(&self, run: u64, event: TermEvent) {
        // Runs on the pty thread, sometimes with the emulator locked: nothing here may lock it.
        let mut st = self.state.lock();
        if st.run != run {
            return;
        }
        match event {
            TermEvent::Wakeup => {}
            TermEvent::Title(t) => st.title = Some(t).filter(|t| !t.trim().is_empty()),
            TermEvent::ResetTitle => st.title = None,
            TermEvent::PtyWrite(text) => {
                if let Some(r) = &st.running {
                    let _ = r.pty.send(Msg::Input(text.into_bytes().into()));
                }
                return;
            }
            TermEvent::ClipboardStore(_, text) => {
                let _ = self.clipboard.send(text);
                return;
            }
            // Programs don't get to read the clipboard.
            TermEvent::ClipboardLoad(..) => return,
            TermEvent::ColorRequest(index, format) => {
                if let Some(r) = &st.running {
                    let reply = format(default_color(index));
                    let _ = r.pty.send(Msg::Input(reply.into_bytes().into()));
                }
                return;
            }
            TermEvent::TextAreaSizeRequest(format) => {
                if let (Some(r), Some((c, rw))) = (&st.running, st.size) {
                    let _ = r.pty.send(Msg::Input(format(window_size(c, rw, st.cell)).into_bytes().into()));
                }
                return;
            }
            TermEvent::ChildExit(status) => {
                st.exited = Some(match status.code() {
                    Some(0) | None => "The shell exited.".into(),
                    Some(c) => format!("The shell exited with code {c}."),
                });
            }
            TermEvent::Exit => {
                st.exited.get_or_insert_with(|| "The shell exited.".into());
            }
            TermEvent::MouseCursorDirty | TermEvent::CursorBlinkingChange | TermEvent::Bell => return,
        }
        drop(st);
        self.touch();
    }

    /// What the terminal shows now, for remote viewers.
    pub fn screen(&self) -> TerminalScreen {
        let (term, title, exited) = {
            let st = self.state.lock();
            (st.running.as_ref().map(|r| r.term.clone()), st.title.clone(), st.exited.clone())
        };
        let mut screen = TerminalScreen { id: self.id(), cwd: self.cwd.clone(), title, exited, ..Default::default() };
        if let Some(term) = term {
            snapshot(&term.lock(), &mut screen);
        }
        screen
    }
}

/// Every terminal, by the chat it belongs to.
pub struct Terminals {
    map: Mutex<HashMap<TerminalId, Arc<Terminal>>>,
    on_change: Arc<dyn Fn(TerminalId) + Send + Sync>,
}

impl Terminals {
    /// `on_change` is called (on any thread) whenever a terminal has something new to show.
    pub fn new(on_change: impl Fn(TerminalId) + Send + Sync + 'static) -> Self {
        Self { map: Mutex::new(HashMap::new()), on_change: Arc::new(on_change) }
    }

    pub fn get(&self, id: TerminalId) -> Option<Arc<Terminal>> {
        self.map.lock().get(&id).cloned()
    }

    /// The terminal `id`, started in `cwd` if there isn't one yet.
    pub fn open(&self, id: TerminalId, cwd: PathBuf) -> Arc<Terminal> {
        if let Some(t) = self.get(id) {
            return t;
        }
        let t = Arc::new(Terminal {
            id: Mutex::new(id),
            cwd,
            state: Mutex::new(State::default()),
            changed: watch::channel(0).0,
            clipboard: broadcast::channel(8).0,
            on_change: self.on_change.clone(),
        });
        self.map.lock().insert(id, t.clone());
        t.restart();
        t
    }

    /// End a terminal's shell and forget it. Returns whether there was one.
    pub fn close(&self, id: TerminalId) -> bool {
        match self.map.lock().remove(&id) {
            Some(t) => {
                t.kill();
                true
            }
            None => false,
        }
    }

    /// File a terminal under another key (a new-chat draft's terminal, once the chat exists).
    pub fn rekey(&self, from: TerminalId, to: TerminalId) -> Option<Arc<Terminal>> {
        let mut map = self.map.lock();
        let t = map.remove(&from)?;
        *t.id.lock() = to;
        if let Some(old) = map.insert(to, t.clone()) {
            old.kill();
        }
        drop(map);
        t.touch();
        Some(t)
    }

    pub fn screens(&self) -> Vec<TerminalScreen> {
        let all: Vec<Arc<Terminal>> = self.map.lock().values().cloned().collect();
        all.iter().map(|t| t.screen()).collect()
    }

    pub fn close_all(&self) {
        let all: Vec<Arc<Terminal>> = self.map.lock().drain().map(|(_, t)| t).collect();
        for t in all {
            t.kill();
        }
    }
}

fn window_size(cols: u16, rows: u16, cell: (u16, u16)) -> WindowSize {
    let (w, h) = if cell.0 == 0 { (8, 16) } else { cell };
    WindowSize { num_lines: rows, num_cols: cols, cell_width: w, cell_height: h }
}

fn spawn_shell(cwd: &Path, size: Size, cell: (u16, u16), listener: Listener) -> anyhow::Result<Running> {
    let mut env = HashMap::new();
    env.insert("TERM".to_string(), "xterm-256color".to_string());
    env.insert("COLORTERM".to_string(), "truecolor".to_string());
    env.insert("TERM_PROGRAM".to_string(), "Mothr".to_string());
    let options = tty::Options {
        shell: user_shell().map(|(program, args)| tty::Shell::new(program, args)),
        working_directory: Some(cwd.to_path_buf()),
        env,
        ..Default::default()
    };
    let config = Config { scrolling_history: SCROLLBACK, ..Default::default() };
    let term = Arc::new(FairMutex::new(Term::new(config, &size, listener.clone())));
    let pty = tty::new(&options, window_size(size.cols as u16, size.rows as u16, cell), 0)?;
    let event_loop = EventLoop::new(term.clone(), listener, pty, false, false)?;
    let sender = event_loop.channel();
    event_loop.spawn();
    Ok(Running { term, pty: sender })
}

/// The user's own shell, started as a login shell so their profile and rc files load. The
/// account's shell comes first: `$SHELL` can be another one when Mothr was launched from a
/// shell other than the login one. None: the platform default (Windows).
pub fn user_shell() -> Option<(String, Vec<String>)> {
    if cfg!(windows) {
        return None;
    }
    let program = account_shell().or_else(|| std::env::var("SHELL").ok().filter(|s| !s.is_empty())).unwrap_or_else(|| "/bin/sh".into());
    let name = Path::new(&program).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let login = matches!(name.as_str(), "bash" | "zsh" | "fish" | "sh" | "dash" | "ksh" | "mksh" | "yash" | "nu" | "tcsh" | "csh");
    Some((program, if login { vec!["-l".into()] } else { vec![] }))
}

/// A shell the account can log in with.
#[cfg(unix)]
fn usable_shell(s: &str) -> bool {
    !s.is_empty() && Path::new(s).is_file() && !s.ends_with("/nologin") && !s.ends_with("/false")
}

/// The login shell of the account Mothr runs as.
#[cfg(target_os = "macos")]
fn account_shell() -> Option<String> {
    let user = std::env::var("USER").ok()?;
    let out = std::process::Command::new("dscl").args([".", "-read", &format!("/Users/{user}"), "UserShell"]).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace().nth(1).map(str::to_string).filter(|s| usable_shell(s))
}

/// The login shell of the account Mothr runs as: /etc/passwd, else the directory service
/// (LDAP, systemd-homed …).
#[cfg(all(unix, not(target_os = "macos")))]
fn account_shell() -> Option<String> {
    use std::os::unix::fs::MetadataExt as _;
    let uid = std::fs::metadata("/proc/self").ok()?.uid().to_string();
    let from = |text: &str| text.lines().find(|l| l.split(':').nth(2) == Some(uid.as_str())).and_then(|l| l.split(':').nth(6)).map(|s| s.trim().to_string());
    let shell = std::fs::read_to_string("/etc/passwd").ok().and_then(|t| from(&t)).or_else(|| {
        let out = std::process::Command::new("getent").args(["passwd", &uid]).output().ok()?;
        from(&String::from_utf8_lossy(&out.stdout))
    });
    shell.filter(|s| usable_shell(s))
}

#[cfg(not(unix))]
fn account_shell() -> Option<String> {
    None
}

/// A neutral answer for programs asking what a color is (each viewer has its own theme).
fn default_color(index: usize) -> Rgb {
    match index {
        i if i == NamedColor::Background as usize => Rgb { r: 0x10, g: 0x11, b: 0x16 },
        i if i == NamedColor::Foreground as usize || i == NamedColor::Cursor as usize => Rgb { r: 0xEC, g: 0xEC, b: 0xF1 },
        _ => Rgb { r: 0x80, g: 0x80, b: 0x80 },
    }
}

fn term_color(c: Color, colors: &alacritty_terminal::term::color::Colors) -> Option<TermColor> {
    let rgb = |c: Rgb| RGB_FLAG | ((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32;
    match c {
        Color::Spec(c) => Some(rgb(c)),
        Color::Indexed(i) => Some(colors[i as usize].map(rgb).unwrap_or(i as u32)),
        Color::Named(n) => {
            if let Some(c) = colors[n as usize] {
                return Some(rgb(c));
            }
            let i = n as usize;
            match n {
                NamedColor::Foreground | NamedColor::Background | NamedColor::Cursor | NamedColor::BrightForeground | NamedColor::DimForeground => None,
                _ if i < 16 => Some(i as u32),
                // Dim black … dim white: their base color (the span is marked dim).
                _ => Some((i - NamedColor::DimBlack as usize).min(7) as u32),
            }
        }
    }
}

/// Fill `screen` with the live screen (not scrolled back) of `term`.
fn snapshot(term: &Term<Listener>, screen: &mut TerminalScreen) {
    let grid = term.grid();
    let (rows, cols) = (term.screen_lines(), term.columns());
    screen.rows = rows as u16;
    screen.cols = cols as u16;
    let colors = term.colors();
    let mode = *term.mode();
    screen.app_cursor = mode.contains(TermMode::APP_CURSOR);
    let cursor = grid.cursor.point;
    screen.cursor = (mode.contains(TermMode::SHOW_CURSOR) && cursor.line.0 >= 0).then_some([cursor.line.0 as u16, cursor.column.0 as u16]);
    screen.lines = (0..rows)
        .map(|r| {
            let row = &grid[Line(r as i32)];
            // Trailing blank cells with no background carry nothing.
            let mut end = cols;
            while end > 0 {
                let c = &row[Column(end - 1)];
                if c.c != ' ' || c.bg != Color::Named(NamedColor::Background) || c.flags.intersects(Flags::INVERSE | Flags::ALL_UNDERLINES) {
                    break;
                }
                end -= 1;
            }
            let mut spans: Vec<TermSpan> = vec![];
            for col in 0..end {
                let c = &row[Column(col)];
                if c.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                    continue;
                }
                let style = TermSpan {
                    text: String::new(),
                    fg: term_color(c.fg, colors),
                    bg: term_color(c.bg, colors),
                    bold: c.flags.contains(Flags::BOLD),
                    italic: c.flags.contains(Flags::ITALIC),
                    underline: c.flags.intersects(Flags::ALL_UNDERLINES),
                    dim: c.flags.contains(Flags::DIM),
                    inverse: c.flags.contains(Flags::INVERSE),
                };
                let ch = if c.flags.contains(Flags::HIDDEN) || c.c == '\t' || c.c == '\0' { ' ' } else { c.c };
                match spans.last_mut() {
                    Some(last) if TermSpan { text: String::new(), ..last.clone() } == style => last.text.push(ch),
                    _ => spans.push(TermSpan { text: ch.to_string(), ..style }),
                }
                if let Some(zw) = c.zerowidth() {
                    spans.last_mut().expect("just pushed").text.extend(zw.iter());
                }
            }
            spans
        })
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn a_shell_runs_in_the_folder_and_shows_its_output() {
        if cfg!(windows) {
            return;
        }
        let dir = std::env::temp_dir();
        let terminals = Terminals::new(|_| {});
        let id = TerminalId::new_v4();
        let t = terminals.open(id, dir.clone());
        t.resize(60, 10, None);
        // Quotes split the word, so only the output (not the echoed command) reads `mothr-ok`.
        t.write(b"echo mothr-o''k\r".to_vec());
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut text = String::new();
        while Instant::now() < deadline {
            text = t.screen().text();
            if text.contains("mothr-ok") {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(text.contains("mothr-ok"), "screen was:\n{text}");
        let s = t.screen();
        assert_eq!((s.cols, s.rows), (60, 10));
        assert!(terminals.close(id));
        assert!(terminals.get(id).is_none());
    }
}
