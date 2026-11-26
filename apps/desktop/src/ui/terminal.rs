//! A terminal beside a chat (Ctrl+` or Ctrl+~): your shell, started in the chat's primary repo.
//!
//! The hub owns the shell ([`mothr_terminal`]), so the web client and the phone see the same
//! one. This view paints its grid and turns keystrokes into the bytes a terminal sends.

use super::kit::*;
use super::tokens::{self, Palette, sz};
use crate::store::Store;
use gpui_kit::component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    menu::{DropdownMenu as _, PopupMenuItem},
    resizable::{h_resizable, resizable_panel, v_resizable},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mothr_core::{Dock, TerminalId, tildify};
use mothr_terminal::alacritty_terminal::grid::{Dimensions as _, Scroll};
use mothr_terminal::alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use mothr_terminal::alacritty_terminal::selection::{Selection, SelectionType};
use mothr_terminal::alacritty_terminal::term::cell::Flags;
use mothr_terminal::alacritty_terminal::term::color::Colors;
use mothr_terminal::alacritty_terminal::term::{Term, TermMode};
use mothr_terminal::alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use mothr_terminal::{Listener, SharedTerm, Terminal, Terminals};
use std::borrow::Cow;
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

const FONT_SIZE: f32 = 13.0;
const LINE_HEIGHT: f32 = 1.35;
/// Narrower than this, a terminal docked to a side goes below the chat instead.
const SIDE_MIN_WINDOW: f32 = 1080.0;

/// Lay out `main` with the terminal docked where the user keeps it, with a divider to drag.
/// A side dock moves to the bottom while the window is too narrow for both side by side.
pub fn docked(id: &str, main: AnyElement, terminal: AnyElement, window: &Window) -> AnyElement {
    let mut dock = tokens::appearance().terminal_dock;
    if dock.is_side() && window.viewport_size().width < sz(SIDE_MIN_WINDOW) {
        dock = Dock::Bottom;
    }
    let id = SharedString::from(format!("{id}-{}", dock.label()));
    // The chat takes what the terminal leaves; sizing it by its content squeezes wrapped text.
    let main = resizable_panel().flex_basis(px(0.)).size_range(sz(240.)..Pixels::MAX).child(div().size_full().min_w_0().min_h_0().child(main));
    let term = if dock.is_side() {
        resizable_panel().size(sz(480.)).size_range(sz(300.)..Pixels::MAX)
    } else {
        resizable_panel().size(sz(300.)).size_range(sz(140.)..Pixels::MAX)
    }
    .fixed()
    .child(div().size_full().child(terminal));
    match dock {
        Dock::Right => h_resizable(id).child(main).child(term),
        Dock::Left => h_resizable(id).child(term).child(main),
        Dock::Bottom => v_resizable(id).child(main).child(term),
        Dock::Top => v_resizable(id).child(term).child(main),
    }
    .into_any_element()
}

fn dock_icon(d: Dock) -> &'static str {
    match d {
        Dock::Top => "panel-top",
        Dock::Right => "panel-right",
        Dock::Bottom => "panel-bottom",
        Dock::Left => "panel-left",
    }
}

pub enum TerminalEvent {
    /// Put the terminal away (the shell keeps running).
    Hide,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

/// The grid size this view has room for.
#[derive(Clone, Copy, PartialEq)]
struct Size {
    cols: u16,
    rows: u16,
}

pub struct TerminalView {
    store: Entity<Store>,
    terminals: Arc<Terminals>,
    terminal: Arc<Terminal>,
    /// Set for a new-chat draft's terminal: it ends with the view unless it was filed under the
    /// chat the draft became.
    draft: Option<TerminalId>,
    focus: FocusHandle,
    /// The size this view last asked for.
    size: Rc<Cell<Option<Size>>>,
    /// Where the grid was painted, and its cell size, to map the mouse onto cells.
    layout: Rc<Cell<Option<GridLayout>>>,
    selecting: bool,
    _tasks: Vec<Task<()>>,
    _intercept: Subscription,
}

/// The painted grid's bounds and cell size.
type GridLayout = (Bounds<Pixels>, Size2);

#[derive(Clone, Copy)]
struct Size2 {
    w: Pixels,
    h: Pixels,
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        if let Some(draft) = self.draft
            && self.terminal.id() == draft
        {
            self.terminals.close(draft);
        }
    }
}

impl TerminalView {
    /// The terminal `id` (a chat, or a new-chat draft when `draft`), started in `cwd` if the
    /// hub doesn't have it running yet.
    pub fn new(store: Entity<Store>, id: TerminalId, cwd: PathBuf, draft: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let terminals = store.read(cx).hub.terminals().clone();
        let terminal = terminals.open(id, cwd);
        if terminal.exited().is_some() {
            terminal.restart();
        }
        let focus = cx.focus_handle();
        // Keys go to the shell before the app's shortcuts see them (Ctrl+P, Ctrl+N, Ctrl+K …
        // mean something in a shell); a few stay with the app.
        let weak = cx.entity().downgrade();
        let intercept = cx.intercept_keystrokes(move |ev, window, cx| {
            let Some(this) = weak.upgrade() else { return };
            if !this.read(cx).focus.is_focused(window) || is_app_key(&ev.keystroke) {
                return;
            }
            if this.update(cx, |t, cx| t.on_key(&ev.keystroke, cx)) {
                cx.stop_propagation();
            }
        });
        let _ = window;
        let mut changes = terminal.subscribe();
        let mut clipboard = terminal.clipboard_requests();
        let tasks = vec![
            cx.spawn(async move |this, cx| {
                while changes.changed().await.is_ok() {
                    if this.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            }),
            cx.spawn(async move |_, cx| {
                while let Ok(text) = clipboard.recv().await {
                    cx.update(|cx| cx.write_to_clipboard(ClipboardItem::new_string(text)));
                }
            }),
        ];
        Self {
            store,
            terminals,
            terminal,
            draft: draft.then_some(id),
            focus,
            size: Rc::new(Cell::new(None)),
            layout: Rc::new(Cell::new(None)),
            selecting: false,
            _tasks: tasks,
            _intercept: intercept,
        }
    }

    pub fn cwd(&self) -> &Path {
        self.terminal.cwd()
    }

    pub fn id(&self) -> TerminalId {
        self.terminal.id()
    }

    /// The draft this terminal was opened for became a chat; it's that chat's terminal now.
    pub fn adopted(&mut self) {
        self.draft = None;
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
    }

    fn term(&self) -> Option<SharedTerm> {
        self.terminal.term()
    }

    fn restart(&mut self, cx: &mut Context<Self>) {
        self.terminal.restart();
        self.size.set(None);
        cx.notify();
    }

    fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        self.terminal.write(bytes);
    }

    fn mode(&self) -> TermMode {
        self.term().map(|t| *t.lock().mode()).unwrap_or_default()
    }

    /// Type into the shell. Returns whether the key was used.
    fn on_key(&mut self, ks: &Keystroke, cx: &mut Context<Self>) -> bool {
        let m = &ks.modifiers;
        if self.terminal.exited().is_some() {
            if ks.key == "enter" && !m.modified() {
                self.restart(cx);
                return true;
            }
            return false;
        }
        let secondary = if cfg!(target_os = "macos") { m.platform && !m.control } else { m.control && m.shift };
        if secondary && ks.key == "c" {
            self.copy(cx);
            return true;
        }
        if secondary && ks.key == "v" {
            self.paste(cx);
            return true;
        }
        if m.shift && !m.control && !m.alt && matches!(ks.key.as_str(), "pageup" | "pagedown") {
            self.scroll(if ks.key == "pageup" { Scroll::PageUp } else { Scroll::PageDown }, cx);
            return true;
        }
        match key_bytes(ks, self.mode()) {
            Some(bytes) => {
                // Whoever types sets the size: take the shell back from a phone that resized it.
                if let Some(want) = self.size.get()
                    && self.terminal.size() != Some((want.cols, want.rows))
                {
                    let cell = self.layout.get().map(|(_, c)| (f32::from(c.w) as u16, f32::from(c.h) as u16));
                    self.terminal.resize(want.cols, want.rows, cell);
                }
                // Typing jumps back to the prompt and drops the selection.
                if let Some(term) = self.term() {
                    let mut term = term.lock();
                    term.scroll_display(Scroll::Bottom);
                    term.selection = None;
                }
                self.write(bytes);
                cx.notify();
                true
            }
            None => false,
        }
    }

    fn copy(&self, cx: &mut Context<Self>) {
        if let Some(text) = self.term().and_then(|t| t.lock().selection_to_string()).filter(|t| !t.is_empty()) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&self, cx: &mut Context<Self>) {
        let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) else { return };
        let text = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.mode().contains(TermMode::BRACKETED_PASTE) {
            // Strip the end marker so pasted text can't end the paste early.
            let inner = text.replace("\x1b[201~", "");
            self.write(format!("\x1b[200~{inner}\x1b[201~").into_bytes());
        } else {
            self.write(text.into_bytes());
        }
    }

    fn scroll(&mut self, scroll: Scroll, cx: &mut Context<Self>) {
        if let Some(term) = self.term() {
            term.lock().scroll_display(scroll);
            cx.notify();
        }
    }

    fn on_wheel(&mut self, ev: &ScrollEvent, cx: &mut Context<Self>) {
        let line_h = sz(FONT_SIZE * LINE_HEIGHT);
        let dy = ev.delta.pixel_delta(line_h).y;
        let lines = (dy / line_h).round() as i32;
        if lines == 0 {
            return;
        }
        let mode = self.mode();
        if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
            // Full-screen programs (less, vim) scroll by arrow keys.
            let key: &[u8] = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            self.write(key.repeat(lines.unsigned_abs() as usize));
            return;
        }
        self.scroll(Scroll::Delta(lines), cx);
    }

    /// The grid cell under a window position.
    fn cell_at(&self, pos: Point<Pixels>) -> Option<(GridPoint, Side)> {
        let (bounds, cell) = self.layout.get()?;
        let term = self.term()?;
        let term = term.lock();
        let (cols, rows) = (term.columns(), term.screen_lines());
        let x = (pos.x - bounds.origin.x).max(px(0.));
        let y = (pos.y - bounds.origin.y).max(px(0.));
        let col_f = x / cell.w;
        let col = (col_f.floor() as usize).min(cols.saturating_sub(1));
        let row = ((y / cell.h).floor() as usize).min(rows.saturating_sub(1));
        let side = if col_f.fract() < 0.5 { Side::Left } else { Side::Right };
        let offset = term.grid().display_offset();
        Some((GridPoint::new(Line(row as i32 - offset as i32), Column(col)), side))
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let Some((point, side)) = self.cell_at(ev.position) else { return };
        if let Some(term) = self.term() {
            let ty = match ev.click_count {
                2 => SelectionType::Semantic,
                3 => SelectionType::Lines,
                _ => SelectionType::Simple,
            };
            term.lock().selection = Some(Selection::new(ty, point, side));
            self.selecting = true;
            cx.notify();
        }
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        if !self.selecting || ev.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let Some((point, side)) = self.cell_at(ev.position) else { return };
        if let Some(term) = self.term()
            && let Some(sel) = term.lock().selection.as_mut()
        {
            sel.update(point, side);
        }
        cx.notify();
    }

    fn on_mouse_up(&mut self, cx: &mut Context<Self>) {
        self.selecting = false;
        // A click without a drag leaves nothing selected.
        if let Some(term) = self.term() {
            let mut term = term.lock();
            if term.selection.as_ref().is_some_and(|sel| sel.is_empty()) {
                term.selection = None;
            }
        }
        cx.notify();
    }

    fn render_grid(&self, pal: Palette, focused: bool) -> impl IntoElement + use<> {
        let terminal = self.terminal.clone();
        let paint_term = self.term();
        let size = self.size.clone();
        let layout = self.layout.clone();
        canvas(
            move |bounds, window, _cx| {
                let font_size = sz(FONT_SIZE);
                let ts = window.text_system();
                let id = ts.resolve_font(&mono_font());
                let w = ts.advance(id, font_size, 'm').map(|s| s.width).unwrap_or(font_size * 0.6);
                let cell = Size2 { w, h: (font_size * LINE_HEIGHT).round() };
                let want = Size {
                    cols: ((bounds.size.width / cell.w).floor() as u16).max(2),
                    rows: ((bounds.size.height / cell.h).floor() as u16).max(1),
                };
                // The view's room changed: fit the shell to it.
                if size.get() != Some(want) {
                    size.set(Some(want));
                    terminal.resize(want.cols, want.rows, Some((f32::from(cell.w) as u16, f32::from(cell.h) as u16)));
                }
                layout.set(Some((bounds, cell)));
                cell
            },
            move |bounds, cell, window, cx| {
                if let Some(term) = paint_term {
                    paint_grid(&term.lock(), bounds, cell, &pal, focused, window, cx);
                }
            },
        )
        .size_full()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = tokens::p(cx);
        let focused = self.focus.is_focused(window);
        let place = match self.terminal.title() {
            Some(t) => t,
            None => tildify(self.terminal.cwd()),
        };
        let ended = self.terminal.exited();
        v_flex()
            .size_full()
            .bg(pal.code_bg)
            .child(
                h_flex()
                    .flex_none()
                    .h(sz(40.))
                    .pl(sz(14.))
                    .pr(sz(6.))
                    .gap(sz(8.))
                    .items_center()
                    .border_b_1()
                    .border_color(pal.border)
                    .child(icon("square-terminal").size(sz(14.)).text_color(if focused { pal.text } else { pal.text_muted }))
                    .child(div().flex_none().text_size(sz(tokens::text::BASE)).font_weight(FontWeight::SEMIBOLD).child("Terminal"))
                    .child(div().flex_1().min_w_0().truncate().text_size(sz(tokens::text::SM)).text_color(pal.text_faint).child(place))
                    .child({
                        let store = self.store.clone();
                        let current = tokens::appearance().terminal_dock;
                        Button::new("terminal-dock")
                            .ghost()
                            .small()
                            .icon(icon(dock_icon(current)))
                            .tooltip("Move the terminal")
                            .dropdown_menu(move |mut menu, _, _| {
                                for d in Dock::ALL {
                                    let store = store.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(format!("Dock {}", d.label().to_lowercase()))
                                            .icon(icon(dock_icon(d)))
                                            .checked(d == current)
                                            .on_click(move |_, _, cx| super::theme::edit_appearance(&store, |a| a.terminal_dock = d, cx)),
                                    );
                                }
                                menu
                            })
                    })
                    .child(
                        Button::new("terminal-restart")
                            .ghost()
                            .small()
                            .icon(icon("rotate-ccw"))
                            .tooltip("Restart the shell")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.restart(cx);
                                window.focus(&this.focus, cx);
                            })),
                    )
                    .child(
                        Button::new("terminal-close")
                            .ghost()
                            .small()
                            .icon(icon("x"))
                            .tooltip("Hide  (Ctrl+`)")
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(TerminalEvent::Hide))),
                    ),
            )
            .child(
                div()
                    .id("terminal-body")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .px(sz(10.))
                    .py(sz(8.))
                    .key_context("Terminal")
                    .track_focus(&self.focus)
                    .cursor(CursorStyle::IBeam)
                    .on_mouse_down(MouseButton::Left, cx.listener(|this, ev: &MouseDownEvent, window, cx| this.on_mouse_down(ev, window, cx)))
                    .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, _, cx| this.on_mouse_move(ev, cx)))
                    .on_mouse_up(MouseButton::Left, cx.listener(|this, _: &MouseUpEvent, _, cx| this.on_mouse_up(cx)))
                    .on_scroll(cx.listener(|this, ev: &ScrollEvent, _, cx| this.on_wheel(ev, cx)))
                    .child(self.render_grid(pal, focused))
                    .when_some(ended, |d, why| {
                        d.child(
                            h_flex()
                                .absolute()
                                .left(sz(10.))
                                .right(sz(10.))
                                .bottom(sz(10.))
                                .px(sz(12.))
                                .py(sz(8.))
                                .gap(sz(8.))
                                .items_center()
                                .rounded(sz(tokens::radius::MD))
                                .border_1()
                                .border_color(pal.border_strong)
                                .bg(pal.raised)
                                .text_size(sz(tokens::text::SM))
                                .text_color(pal.text_muted)
                                .child(div().flex_1().child(why))
                                .child(kbd_hint("↵", cx))
                                .child("restart"),
                        )
                    }),
            )
    }
}

/// Paint the visible part of the grid: backgrounds, text, then the cursor.
fn paint_grid(term: &Term<Listener>, bounds: Bounds<Pixels>, cell: Size2, pal: &Palette, focused: bool, window: &mut Window, cx: &mut App) {
    struct Seg {
        col: usize,
        cells: usize,
        text: String,
        fg: Hsla,
        flags: Flags,
    }
    const STYLE: Flags = Flags::BOLD.union(Flags::ITALIC).union(Flags::ALL_UNDERLINES).union(Flags::STRIKEOUT);
    window.paint_quad(fill(bounds, pal.code_bg));
    let content = term.renderable_content();
    let offset = content.display_offset as i32;
    let colors = content.colors;
    let rows = term.screen_lines();
    let cursor = content.cursor;
    let cursor_at = (content.mode.contains(TermMode::SHOW_CURSOR) && cursor.shape != CursorShape::Hidden)
        .then(|| ((cursor.point.line.0 + offset) as usize, cursor.point.column.0));
    let block_cursor = focused && cursor.shape == CursorShape::Block;
    let custom_bg = colors[NamedColor::Background as usize].is_some();
    let mut segs: Vec<Vec<Seg>> = (0..rows).map(|_| vec![]).collect();
    let mut bgs: Vec<(usize, usize, usize, Hsla)> = vec![];
    for item in content.display_iter {
        let row = item.point.line.0 + offset;
        if row < 0 || row as usize >= rows {
            continue;
        }
        let row = row as usize;
        let col = item.point.column.0;
        let c = item.cell;
        if c.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        let mut fg = resolve(c.fg, colors, pal);
        let mut bg = resolve(c.bg, colors, pal);
        let mut plain_bg = !custom_bg && c.bg == Color::Named(NamedColor::Background);
        if c.flags.contains(Flags::INVERSE) {
            std::mem::swap(&mut fg, &mut bg);
            plain_bg = false;
        }
        if c.flags.contains(Flags::DIM) {
            fg = fg.opacity(0.66);
        }
        if c.flags.contains(Flags::HIDDEN) {
            fg = bg;
        }
        if content.selection.is_some_and(|s| s.contains(item.point)) {
            bg = pal.brand.opacity(0.35);
            plain_bg = false;
        }
        if block_cursor && cursor_at == Some((row, col)) {
            bg = pal.text;
            fg = pal.code_bg;
            plain_bg = false;
        }
        let wide = c.flags.contains(Flags::WIDE_CHAR);
        let width = if wide { 2 } else { 1 };
        if !plain_bg {
            match bgs.last_mut() {
                Some(last) if last.0 == row && last.2 == col && last.3 == bg => last.2 = col + width,
                _ => bgs.push((row, col, col + width, bg)),
            }
        }
        let mut text = String::new();
        text.push(if c.c == '\t' || c.c == '\0' { ' ' } else { c.c });
        if let Some(zw) = c.zerowidth() {
            text.extend(zw.iter());
        }
        let flags = c.flags & STYLE;
        let line = &mut segs[row];
        match line.last_mut() {
            Some(last) if !wide && last.cells == last.text.chars().count() && last.col + last.cells == col && last.fg == fg && last.flags == flags && text.chars().count() == 1 => {
                last.text.push_str(&text);
                last.cells += 1;
            }
            _ => line.push(Seg { col, cells: width, text, fg, flags }),
        }
    }
    let origin = bounds.origin;
    let at = |row: usize, col: usize| point(origin.x + cell.w * col as f32, origin.y + cell.h * row as f32);
    for (row, start, end, color) in bgs {
        window.paint_quad(fill(Bounds::new(at(row, start), size(cell.w * (end - start) as f32, cell.h)), color));
    }
    let font_size = sz(FONT_SIZE);
    let base = mono_font();
    for (row, line) in segs.into_iter().enumerate() {
        for seg in line {
            let decorated = seg.flags.intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT);
            if !decorated && seg.text.trim().is_empty() {
                continue;
            }
            let mut font = base.clone();
            if seg.flags.contains(Flags::BOLD) {
                font.weight = FontWeight::BOLD;
            }
            if seg.flags.contains(Flags::ITALIC) {
                font.style = FontStyle::Italic;
            }
            let run = TextRun {
                len: seg.text.len(),
                font,
                color: seg.fg,
                background_color: None,
                underline: seg.flags.intersects(Flags::ALL_UNDERLINES).then_some(UnderlineStyle { thickness: px(1.), color: Some(seg.fg), wavy: seg.flags.contains(Flags::UNDERCURL) }),
                strikethrough: seg.flags.contains(Flags::STRIKEOUT).then_some(StrikethroughStyle { thickness: px(1.), color: Some(seg.fg) }),
            };
            let per_glyph = if seg.cells > seg.text.chars().count() { cell.w * 2. } else { cell.w };
            let shaped = window.text_system().shape_line(seg.text.into(), font_size, &[run], Some(per_glyph));
            let _ = shaped.paint(at(row, seg.col), cell.h, TextAlign::Left, None, window, cx);
        }
    }
    // The cursor, when it isn't the filled block painted with the text.
    if let Some((row, col)) = cursor_at.filter(|(r, _)| *r < rows)
        && !block_cursor
    {
        let o = at(row, col);
        let thin = px(2.);
        let quad = match (focused, cursor.shape) {
            (true, CursorShape::Beam) => fill(Bounds::new(o, size(thin, cell.h)), pal.text),
            (true, CursorShape::Underline) => fill(Bounds::new(point(o.x, o.y + cell.h - thin), size(cell.w, thin)), pal.text),
            _ => outline(Bounds::new(o, size(cell.w, cell.h)), pal.text_muted, BorderStyle::Solid),
        };
        window.paint_quad(quad);
    }
}

/// Shortcuts that stay with the app while the terminal has focus.
fn is_app_key(ks: &Keystroke) -> bool {
    let m = &ks.modifiers;
    if matches!(ks.key.as_str(), "shift" | "control" | "alt" | "platform" | "function") {
        return true;
    }
    // Toggle the terminal; and on macOS every ⌘ shortcut belongs to the app.
    if m.control && matches!(ks.key.as_str(), "`" | "~") {
        return true;
    }
    if cfg!(target_os = "macos") {
        return m.platform && !matches!(ks.key.as_str(), "c" | "v");
    }
    // Ctrl+Shift shortcuts (the repo finder, new design, resume …) except copy and paste.
    m.control && m.shift && !matches!(ks.key.as_str(), "c" | "v")
}

fn mono_font() -> Font {
    font(tokens::FONT_MONO)
}

/// The bytes a terminal sends for a keystroke.
fn key_bytes(ks: &Keystroke, mode: TermMode) -> Option<Vec<u8>> {
    let m = &ks.modifiers;
    // xterm's modifier parameter: 1 + shift + 2·alt + 4·ctrl.
    let modp = 1 + m.shift as u8 + 2 * m.alt as u8 + 4 * m.control as u8;
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let csi = |final_: &str| -> Vec<u8> {
        if modp > 1 { format!("\x1b[1;{modp}{final_}").into_bytes() } else if app_cursor { format!("\x1bO{final_}").into_bytes() } else { format!("\x1b[{final_}").into_bytes() }
    };
    let tilde = |n: u8| -> Vec<u8> { if modp > 1 { format!("\x1b[{n};{modp}~").into_bytes() } else { format!("\x1b[{n}~").into_bytes() } };
    let alt = |mut b: Vec<u8>| -> Vec<u8> {
        if m.alt {
            b.insert(0, 0x1b);
        }
        b
    };
    let bytes = match ks.key.as_str() {
        "enter" => alt(b"\r".to_vec()),
        "backspace" if m.control => alt(vec![0x08]),
        "backspace" => alt(vec![0x7f]),
        "tab" if m.shift => b"\x1b[Z".to_vec(),
        "tab" => alt(b"\t".to_vec()),
        "escape" => alt(vec![0x1b]),
        "up" => csi("A"),
        "down" => csi("B"),
        "right" => csi("C"),
        "left" => csi("D"),
        "home" => csi("H"),
        "end" => csi("F"),
        "insert" => tilde(2),
        "delete" => tilde(3),
        "pageup" => tilde(5),
        "pagedown" => tilde(6),
        "f1" => csi("P"),
        "f2" => csi("Q"),
        "f3" => csi("R"),
        "f4" => csi("S"),
        "f5" => tilde(15),
        "f6" => tilde(17),
        "f7" => tilde(18),
        "f8" => tilde(19),
        "f9" => tilde(20),
        "f10" => tilde(21),
        "f11" => tilde(23),
        "f12" => tilde(24),
        "space" if m.control => vec![0],
        key if m.control && !m.platform => {
            let c = key.chars().next().filter(|_| key.chars().count() == 1)?;
            let code = match c.to_ascii_lowercase() {
                c @ 'a'..='z' => c as u8 - b'a' + 1,
                '@' | '2' => 0,
                '[' | '3' => 0x1b,
                '\\' | '4' => 0x1c,
                ']' | '5' => 0x1d,
                '^' | '6' => 0x1e,
                '_' | '-' | '7' => 0x1f,
                '/' => 0x1f,
                '8' => 0x7f,
                _ => return None,
            };
            alt(vec![code])
        }
        _ if m.platform => return None,
        _ => {
            let text = ks.key_char.clone().or_else(|| (ks.key.chars().count() == 1).then(|| ks.key.clone()))?;
            alt(text.into_bytes())
        }
    };
    Some(bytes)
}

fn rgb_hsla(c: Rgb) -> Hsla {
    gpui_kit::rgb(((c.r as u32) << 16) | ((c.g as u32) << 8) | c.b as u32).into()
}

/// A color of the 256-color table (or one of the special named slots past it).
fn resolve_index(i: usize, colors: &Colors, pal: &Palette) -> Hsla {
    if let Some(c) = colors[i] {
        return rgb_hsla(c);
    }
    match i {
        0..16 => pal.ansi[i],
        16..232 => {
            let i = i - 16;
            let v = |n: usize| if n == 0 { 0 } else { (55 + n * 40) as u8 };
            rgb_hsla(Rgb { r: v(i / 36), g: v((i / 6) % 6), b: v(i % 6) })
        }
        232..256 => {
            let g = (8 + (i - 232) * 10) as u8;
            rgb_hsla(Rgb { r: g, g, b: g })
        }
        _ => pal.text,
    }
}

fn resolve(color: Color, colors: &Colors, pal: &Palette) -> Hsla {
    match color {
        Color::Spec(c) => rgb_hsla(c),
        Color::Indexed(i) => resolve_index(i as usize, colors, pal),
        Color::Named(n) => {
            if let Some(c) = colors[n as usize] {
                return rgb_hsla(c);
            }
            match n {
                NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::Cursor => pal.text,
                NamedColor::Background => pal.code_bg,
                NamedColor::DimForeground => pal.text_muted,
                NamedColor::DimBlack => pal.ansi[0].opacity(0.7),
                NamedColor::DimRed => pal.ansi[1].opacity(0.7),
                NamedColor::DimGreen => pal.ansi[2].opacity(0.7),
                NamedColor::DimYellow => pal.ansi[3].opacity(0.7),
                NamedColor::DimBlue => pal.ansi[4].opacity(0.7),
                NamedColor::DimMagenta => pal.ansi[5].opacity(0.7),
                NamedColor::DimCyan => pal.ansi[6].opacity(0.7),
                NamedColor::DimWhite => pal.ansi[7].opacity(0.7),
                n => pal.ansi[(n as usize).min(15)],
            }
        }
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI's glob brings its own `test` attribute.
    use super::{TermMode, is_app_key, key_bytes};
    use gpui_kit::Keystroke;

    fn ks(s: &'static str) -> Keystroke {
        Keystroke::parse(s).unwrap()
    }

    #[test]
    fn keys_become_terminal_bytes() {
        let none = TermMode::empty();
        assert_eq!(key_bytes(&ks("ctrl-c"), none), Some(vec![3]));
        assert_eq!(key_bytes(&ks("enter"), none), Some(b"\r".to_vec()));
        assert_eq!(key_bytes(&ks("up"), none), Some(b"\x1b[A".to_vec()));
        assert_eq!(key_bytes(&ks("up"), TermMode::APP_CURSOR), Some(b"\x1bOA".to_vec()));
        assert_eq!(key_bytes(&ks("ctrl-left"), none), Some(b"\x1b[1;5D".to_vec()));
        assert_eq!(key_bytes(&ks("alt-b"), none), Some(b"\x1bb".to_vec()));
        assert_eq!(key_bytes(&ks("delete"), none), Some(b"\x1b[3~".to_vec()));
    }

    #[test]
    fn app_keeps_its_shortcuts() {
        assert!(is_app_key(&ks("ctrl-`")));
        if !cfg!(target_os = "macos") {
            assert!(is_app_key(&ks("ctrl-shift-p")));
            assert!(!is_app_key(&ks("ctrl-shift-c")));
            assert!(!is_app_key(&ks("ctrl-p")));
        }
    }
}
