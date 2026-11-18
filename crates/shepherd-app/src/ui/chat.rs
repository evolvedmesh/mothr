//! A single agent conversation: header, transcript and composer (+ design canvas).

use super::design::DesignView;
use super::home::mode_menu;
use super::kit::*;
use super::tokens;
use super::theme::card;
use crate::store::{Store, StoreEvent};
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{DropdownMenu as _, PopupMenuItem},
    notification::Notification,
    spinner::Spinner,
    text::{TextView, TextViewState},
    v_flex,
};
use gpui_kit::*;
use super::tokens::sz;
use gpui_kit::prelude::FluentBuilder as _;
use shepherd_core::*;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

pub enum ChatEvent {
    Open(SessionId),
}

impl EventEmitter<ChatEvent> for ChatView {}

pub struct ChatView {
    id: SessionId,
    store: Entity<Store>,
    input: Entity<TextareaState>,
    attachments: Vec<PathBuf>,
    markdown: HashMap<EntryId, (Entity<TextViewState>, usize)>,
    expanded: HashSet<EntryId>,
    /// Selected answers per question entry: one list per question.
    answers: HashMap<EntryId, Vec<Vec<String>>>,
    other_inputs: HashMap<(EntryId, usize), Entity<InputState>>,
    scroll: ScrollHandle,
    follow: bool,
    working_since: Option<Instant>,
    /// The user message being edited (edit & resend).
    editing: Option<EntryId>,
    /// Highlighted row in the slash-command menu.
    slash_sel: usize,
    design: Option<Entity<DesignView>>,
    _subs: Vec<Subscription>,
}

impl ChatView {
    pub fn new(id: SessionId, store: Entity<Store>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 10)
                .submit_on_enter(true)
                .placeholder("Reply… (↵ to send, ⇧↵ for a newline)")
        });
        let meta = store.read(cx).session(id).map(|s| s.meta.clone());
        let design = meta
            .as_ref()
            .filter(|m| m.kind == SessionKind::Design)
            .map(|m| {
                let project = m.project.clone();
                cx.new(|cx| DesignView::new(project, window, cx))
            });
        let weak = cx.entity().downgrade();
        let subs = vec![
            cx.subscribe_in(&input, window, |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::PressEnter { shift: false, .. } => this.send(Delivery::Normal, window, cx),
                InputEvent::Change => {
                    this.slash_sel = 0;
                    cx.notify();
                }
                _ => {}
            }),
            cx.intercept_keystrokes(move |ev, window, cx| {
                if let Some(this) = weak.upgrade() {
                    this.update(cx, |this, cx| this.on_keystroke(ev, window, cx));
                }
            }),
            cx.subscribe_in(&store, window, move |this, _, ev: &StoreEvent, window, cx| {
                if let StoreEvent::Session(sid, e) = ev {
                    if *sid != this.id {
                        return;
                    }
                    if let SessionEvent::Upsert { entry: Entry::Question(q) } = e {
                        for (i, _) in q.questions.iter().enumerate() {
                            let key = (q.id.clone(), i);
                            this.other_inputs
                                .entry(key)
                                .or_insert_with(|| cx.new(|cx| InputState::new(window, cx).placeholder("Other…")));
                        }
                    }
                    if let SessionEvent::Status { status } = e {
                        match status {
                            SessionStatus::Working => {
                                this.working_since.get_or_insert_with(Instant::now);
                            }
                            _ => this.working_since = None,
                        }
                    }
                    if this.follow {
                        this.scroll.scroll_to_bottom();
                    }
                    cx.notify();
                }
            }),
        ];
        let mut this = Self {
            id,
            store,
            input,
            attachments: vec![],
            markdown: HashMap::new(),
            expanded: HashSet::new(),
            answers: HashMap::new(),
            other_inputs: HashMap::new(),
            scroll: ScrollHandle::new(),
            follow: true,
            working_since: None,
            editing: None,
            slash_sel: 0,
            design,
            _subs: subs,
        };
        // Inputs for questions that already exist (e.g. after a restart).
        let pending: Vec<(EntryId, usize)> = this
            .store
            .read(cx)
            .session(id)
            .map(|s| {
                s.entries
                    .iter()
                    .filter_map(|e| match e {
                        Entry::Question(q) => Some((q.id.clone(), q.questions.len())),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        for (qid, n) in pending {
            for i in 0..n {
                this.other_inputs.insert((qid.clone(), i), cx.new(|cx| InputState::new(window, cx).placeholder("Other…")));
            }
        }
        this.scroll.scroll_to_bottom();
        this
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |i, cx| i.focus(window, cx));
    }

    fn dispatch(&self, command: SessionCommand, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.id;
        self.store.update(cx, |s, cx| {
            s.dispatch(AppCommand::Session { id, command }, window, cx);
        });
    }

    fn send(&mut self, delivery: Delivery, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.input.read(cx).value().trim().to_string();
        if text.is_empty() && self.attachments.is_empty() {
            return;
        }
        if let Some(entry_id) = self.editing.take() {
            let id = self.id;
            self.store.update(cx, |s, cx| {
                s.dispatch(AppCommand::EditMessage { id, entry_id, text }, window, cx);
            });
        } else {
            let attachments = std::mem::take(&mut self.attachments)
                .into_iter()
                .map(|path| Attachment { mime: mime_for(&path), path })
                .collect();
            self.dispatch(SessionCommand::Prompt { text, attachments, delivery }, window, cx);
        }
        self.input.update(cx, |i, cx| i.set_value("", window, cx));
        self.follow = true;
        self.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// Put text in the composer and focus it (dev hooks, shortcuts).
    pub fn prefill(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let t = text.to_string();
        self.input.update(cx, |i, cx| {
            i.set_value(t, window, cx);
            i.focus(window, cx);
        });
        cx.notify();
    }

    fn start_edit(&mut self, entry_id: EntryId, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = Some(entry_id);
        self.input.update(cx, |i, cx| {
            i.set_value(text, window, cx);
            i.focus(window, cx);
        });
        cx.notify();
    }

    fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.editing.take().is_some() {
            self.input.update(cx, |i, cx| i.set_value("", window, cx));
            cx.notify();
        }
    }

    fn copy(text: &str, window: &mut Window, cx: &mut App) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
        window.push_notification(Notification::success("Copied to clipboard").autohide(true), cx);
    }

    /// Slash commands matching what's typed (only while typing the command name).
    fn slash_matches(&self, cx: &App) -> Vec<CommandInfo> {
        let text = self.input.read(cx).value();
        let Some(query) = text.strip_prefix('/') else { return vec![] };
        if query.contains(char::is_whitespace) {
            return vec![];
        }
        let Some(session) = self.store.read(cx).session(self.id) else { return vec![] };
        let commands = if session.commands.is_empty() {
            self.store.read(cx).known_commands(session.meta.agent)
        } else {
            session.commands.clone()
        };
        rank_commands(&commands, query)
    }

    fn pick_slash(&mut self, cmd: &CommandInfo, window: &mut Window, cx: &mut Context<Self>) {
        let value = format!("/{} ", cmd.name);
        self.input.update(cx, |i, cx| i.set_value(value, window, cx));
        self.slash_sel = 0;
        cx.notify();
    }

    /// Keyboard handling for the slash menu and edit mode (runs before the text box).
    fn on_keystroke(&mut self, ev: &KeystrokeEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.input.read(cx).focus_handle(cx).is_focused(window) {
            return;
        }
        let k = &ev.keystroke;
        if k.modifiers.modified() {
            return;
        }
        let matches = self.slash_matches(cx);
        if matches.is_empty() {
            if k.key == "escape" && self.editing.is_some() {
                self.cancel_edit(window, cx);
                cx.stop_propagation();
            }
            return;
        }
        match k.key.as_str() {
            "up" => self.slash_sel = (self.slash_sel + matches.len() - 1) % matches.len(),
            "down" => self.slash_sel = (self.slash_sel + 1) % matches.len(),
            "tab" | "enter" => {
                let cmd = matches[self.slash_sel.min(matches.len() - 1)].clone();
                self.pick_slash(&cmd, window, cx);
            }
            "escape" => {
                self.input.update(cx, |i, cx| i.set_value("", window, cx));
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn attach(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: Some("Attach".into()) });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                let _ = this.update(cx, |this, cx| {
                    this.attachments.extend(paths);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Keep a markdown state per assistant entry, syncing streamed text incrementally.
    fn markdown_for(&mut self, id: &str, text: &str, cx: &mut Context<Self>) -> Entity<TextViewState> {
        if let Some((state, len)) = self.markdown.get_mut(id) {
            if *len != text.len() {
                let state = state.clone();
                if text.len() > *len && text.is_char_boundary(*len) {
                    let delta = text[*len..].to_string();
                    state.update(cx, |s, cx| s.push_str(&delta, cx));
                } else {
                    let t = text.to_string();
                    state.update(cx, |s, cx| s.set_text(&t, cx));
                }
                *len = text.len();
            }
            return state.clone();
        }
        let t = text.to_string();
        let state = cx.new(|cx| TextViewState::markdown(&t, cx).selectable(true));
        self.markdown.insert(id.to_string(), (state.clone(), text.len()));
        state
    }

    // ------------------------------------------------------------------ header

    fn render_header(&mut self, s: &SessionState, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let m = &s.meta;
        let weak = cx.entity().downgrade();
        let id = self.id;
        let models = s.models.clone();
        let current_model = m.model.clone();
        let model_label = current_model
            .as_ref()
            .map(|cm| models.iter().find(|x| &x.id == cm).map(|x| x.name.clone()).unwrap_or_else(|| cm.clone()))
            .unwrap_or_else(|| "Default model".into());
        let rc = m.remote_control.clone();
        let ctx = m.context;
        let store = self.store.clone();
        let project = m.project.clone();
        let pinned = m.pinned;

        h_flex()
            .px_4()
            .py_2()
            .gap_x_3()
            .gap_y_2()
            .flex_wrap()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(agent_avatar(m.agent, 30.))
            .child(
                v_flex()
                    .flex_1()
                    .min_w(sz(220.))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().font_weight(FontWeight::SEMIBOLD).truncate().child(m.title.clone()))
                            .child(
                                h_flex()
                                    .gap_1()
                                    .px(sz(6.))
                                    .py(sz(1.))
                                    .rounded(sz(5.))
                                    .bg(status_color(&m.status, cx).opacity(0.12))
                                    .items_center()
                                    .child(status_dot(ElementId::Name(format!("hdr-{id}").into()), &m.status, cx))
                                    .child(div().text_xs().text_color(status_color(&m.status, cx)).child(m.status.label().to_string())),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(format!("{} · {}", m.agent.display_name(), tildify(&m.project))),
                    ),
            )
            .child({
                let pal = tokens::p(cx);
                let f = ctx.fraction();
                let color = level_color(UsageLevel::from_fraction(f), cx);
                h_flex()
                    .id("ctx-gauge")
                    .gap(sz(8.))
                    .px(sz(10.))
                    .h(sz(30.))
                    .items_center()
                    .rounded(sz(tokens::radius::MD))
                    .border_1()
                    .border_color(pal.border)
                    .text_size(sz(tokens::text::SM))
                    .text_color(pal.text_muted)
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(format!(
                            "Context window: {} of {} tokens{}",
                            format_tokens(ctx.used_tokens),
                            if ctx.window_tokens > 0 { format_tokens(ctx.window_tokens) } else { "?".into() },
                            ctx.cost_usd.map(|c| format!(" · session cost ${c:.2}")).unwrap_or_default()
                        ))
                        .build(window, cx)
                    })
                    .child(div().w(sz(36.)).child(meter(f, color, cx)))
                    .child(if ctx.window_tokens > 0 {
                        format!("{:.0}%", f * 100.0)
                    } else if ctx.used_tokens > 0 {
                        format_tokens(ctx.used_tokens)
                    } else {
                        "—".into()
                    })
            })
            .when(!models.is_empty(), |el| {
                let weak = weak.clone();
                el.child(
                    Button::new("model-pick")
                        .ghost()
                        .icon(icon("brain"))
                        .label(model_label)
                        .dropdown_caret(true)
                        .dropdown_menu(move |mut menu, _, _| {
                            menu = menu.label("Model").max_h(sz(420.)).scrollable(true);
                            for mi in &models {
                                let weak = weak.clone();
                                let mid = mi.id.clone();
                                let label = match &mi.description {
                                    Some(d) => format!("{} — {}", mi.name, d),
                                    None => mi.name.clone(),
                                };
                                menu = menu.item(PopupMenuItem::new(label).checked(current_model.as_deref() == Some(mi.id.as_str())).on_click(
                                    move |_, window, cx| {
                                        if let Some(v) = weak.upgrade() {
                                            v.update(cx, |v, cx| {
                                                v.dispatch(SessionCommand::SetModel { model: mid.clone() }, window, cx);
                                                let agent = v.store.read(cx).session(v.id).map(|s| s.meta.agent);
                                                if let Some(agent) = agent {
                                                    v.store.read(cx).record_selection(agent, Some(&mid));
                                                }
                                            });
                                        }
                                    },
                                ));
                            }
                            menu
                        }),
                )
            })
            .when(m.agent.supports_remote_control(), |el| {
                el.child(
                    Button::new("remote-control")
                        .map(|b| if rc.enabled { b.primary() } else { b.ghost() })
                        .loading(rc.pending)
                        .icon(icon("smartphone"))
                        .tooltip(rc.error.clone().unwrap_or_else(|| {
                            if rc.enabled {
                                rc.url.clone().unwrap_or_else(|| "Continue this chat from claude.ai or the Claude app".into())
                            } else {
                                "Continue this chat from your phone via Claude Remote Control".into()
                            }
                        }))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let enabled = !this.store.read(cx).session(id).map(|s| s.meta.remote_control.enabled).unwrap_or(false);
                            this.dispatch(SessionCommand::SetRemoteControl { enabled }, window, cx);
                        })),
                )
            })
            .child(
                Button::new("more")
                    .ghost()
                    .icon(icon("ellipsis"))
                    .dropdown_menu(move |menu, _, _| {
                        let (s1, s2, s3) = (store.clone(), store.clone(), store.clone());
                        let project = project.clone();
                        menu.item(PopupMenuItem::new(if pinned { "Unpin chat" } else { "Pin chat" }).icon(icon("pin")).on_click(move |_, w, cx| {
                            s1.update(cx, |s, cx| {
                                s.dispatch(AppCommand::PinSession { id, pinned: !pinned }, w, cx);
                            });
                        }))
                        .item(PopupMenuItem::new("Open project folder").icon(icon("folder-open")).on_click(move |_, _, cx| {
                            cx.open_url(&format!("file://{}", project.display()));
                        }))
                        .separator()
                        .item(PopupMenuItem::new("Archive").icon(icon("archive")).on_click(move |_, w, cx| {
                            s2.update(cx, |s, cx| {
                                s.dispatch(AppCommand::Archive { id, archived: true }, w, cx);
                            });
                        }))
                        .item(PopupMenuItem::new("Delete").icon(icon("delete")).on_click(move |_, w, cx| {
                            s3.update(cx, |s, cx| {
                                s.dispatch(AppCommand::Delete { id }, w, cx);
                            });
                        }))
                    }),
            )
    }

    // ------------------------------------------------------------------ entries

    fn render_entry(&mut self, e: &Entry, live: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        match e {
            Entry::User { id, text, attachments, agent_ref, .. } => {
                let pal = tokens::p(cx);
                let (editable, live) = self
                    .store
                    .read(cx)
                    .session(self.id)
                    .map(|s| (s.meta.agent.supports_edit() && s.meta.parent.is_none() && agent_ref.is_some(), s.meta.status.is_live()))
                    .unwrap_or((false, false));
                let _ = live;
                let group: SharedString = format!("msg-{id}").into();
                let copy_text = text.clone();
                let edit_text = text.clone();
                let eid = id.clone();
                let being_edited = self.editing.as_deref() == Some(id.as_str());
                v_flex()
                    .group(group.clone())
                    .w_full()
                    .items_end()
                    .gap(sz(4.))
                    .child(
                        v_flex()
                            .max_w(relative(0.82))
                            .gap_1()
                            .items_end()
                            .child(
                                div()
                                    .px(sz(16.))
                                    .py(sz(10.))
                                    .rounded(sz(tokens::radius::XL))
                                    .rounded_br(sz(tokens::radius::SM))
                                    .bg(pal.surface_hover)
                                    .border_1()
                                    .border_color(if being_edited { pal.warning.opacity(0.6) } else { pal.border })
                                    .text_size(sz(tokens::text::READ))
                                    .line_height(relative(1.55))
                                    .child(text.clone()),
                            )
                            .when(!attachments.is_empty(), |d| {
                                d.child(h_flex().gap_1().flex_wrap().children(attachments.iter().map(|a| file_chip(&a.path, cx))))
                            }),
                    )
                    .child(
                        h_flex()
                            .gap(sz(2.))
                            .invisible()
                            .group_hover(group, |d| d.visible())
                            .child(msg_action(ElementId::Name(format!("copy-{id}").into()), "copy", "Copy", &pal).on_click(move |_, window, cx| {
                                Self::copy(&copy_text, window, cx)
                            }))
                            .when(editable, |d| {
                                d.child(
                                    msg_action(ElementId::Name(format!("edit-{id}").into()), "pencil", "Edit and resend", &pal).on_click(cx.listener(
                                        move |this, _, window, cx| this.start_edit(eid.clone(), edit_text.clone(), window, cx),
                                    )),
                                )
                            }),
                    )
                    .into_any_element()
            }
            Entry::Assistant { id, text, streaming } => {
                let pal = tokens::p(cx);
                let state = self.markdown_for(id, text, cx);
                let group: SharedString = format!("msg-{id}").into();
                let copy_text = text.clone();
                v_flex()
                    .group(group.clone())
                    .w_full()
                    .gap(sz(4.))
                    .child(
                        div()
                            .w_full()
                            .text_size(sz(tokens::text::READ))
                            .line_height(relative(tokens::text::READ_LEADING))
                            .child(TextView::new(&state).selectable(true).stream_fade(true).style(markdown_style(cx))),
                    )
                    .when(!*streaming, |d| {
                        d.child(
                            h_flex()
                                .gap(sz(2.))
                                .invisible()
                                .group_hover(group, |d| d.visible())
                                .child(msg_action(ElementId::Name(format!("copy-{id}").into()), "copy", "Copy", &pal).on_click(move |_, window, cx| {
                                    Self::copy(&copy_text, window, cx)
                                })),
                        )
                    })
                    .into_any_element()
            }
            Entry::Thinking { id, text, streaming } => {
                let open = self.expanded.contains(id);
                let key = id.clone();
                let words = text.split_whitespace().count();
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .id(ElementId::Name(format!("think-{id}").into()))
                            .gap_1()
                            .items_center()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.expanded.remove(&key) {
                                    this.expanded.insert(key.clone());
                                }
                                cx.notify();
                            }))
                            .child(icon("brain").size(sz(13.)))
                            .child(if *streaming && live {
                                shimmer_text("Thinking…", ElementId::Name(format!("tsh-{id}").into()), cx).into_any_element()
                            } else {
                                div().child(format!("Thought · {words} words")).into_any_element()
                            })
                            .child(icon(if open { "chevron-down" } else { "chevron-right" }).size(sz(12.))),
                    )
                    .when(open, |d| {
                        d.child(
                            div()
                                .ml_2()
                                .pl_3()
                                .border_l_2()
                                .border_color(theme.border)
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .line_height(relative(1.55))
                                .child(text.clone()),
                        )
                    })
                    .into_any_element()
            }
            Entry::Tool(t) => self.render_tool(t, cx),
            Entry::Plan { items, .. } => {
                let done = items.iter().filter(|i| i.status == PlanStatus::Completed).count();
                card(cx)
                    .p_3()
                    .child(
                        v_flex()
                            .gap_2()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(icon("list-checks").size(sz(15.)).text_color(theme.primary))
                                    .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).flex_1().child("Plan"))
                                    .child(div().text_xs().text_color(theme.muted_foreground).child(format!("{done}/{}", items.len()))),
                            )
                            .child(meter(if items.is_empty() { 0.0 } else { done as f32 / items.len() as f32 }, theme.primary, cx))
                            .children(items.iter().map(|it| {
                                let (ic, color, strike) = match it.status {
                                    PlanStatus::Completed => ("circle-check", theme.success, true),
                                    PlanStatus::InProgress => ("circle-dot", theme.primary, false),
                                    PlanStatus::Pending => ("circle", theme.muted_foreground, false),
                                };
                                h_flex()
                                    .gap_2()
                                    .items_start()
                                    .text_sm()
                                    .child(div().pt(sz(2.)).child(icon(ic).size(sz(14.)).text_color(color)))
                                    .child(
                                        div()
                                            .when(strike, |d| d.line_through().text_color(theme.muted_foreground))
                                            .when(it.status == PlanStatus::InProgress, |d| d.font_weight(FontWeight::MEDIUM))
                                            .child(it.content.clone()),
                                    )
                            })),
                    )
                    .into_any_element()
            }
            Entry::Permission(p) => self.render_permission(p, cx),
            Entry::Question(q) => self.render_question(q, cx),
            Entry::Notice { level, text, .. } => {
                let (ic, color) = match level {
                    NoticeLevel::Info => ("info", theme.muted_foreground),
                    NoticeLevel::Warning => ("triangle-alert", hex(0xF59E0B)),
                    NoticeLevel::Error => ("circle-x", theme.danger),
                };
                h_flex()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(sz(8.))
                    .bg(color.opacity(0.08))
                    .text_xs()
                    .text_color(color)
                    .items_start()
                    .child(icon(ic).size(sz(14.)))
                    .child(div().flex_1().child(text.clone()))
                    .into_any_element()
            }
            Entry::TurnEnd { duration_ms, cost_usd, stop_reason, .. } => {
                let mut parts = vec![];
                if stop_reason != "end_turn" && stop_reason != "success" {
                    parts.push(stop_reason.replace('_', " "));
                }
                if let Some(ms) = duration_ms {
                    parts.push(if *ms > 60_000 { format!("{}m {}s", ms / 60_000, (ms / 1000) % 60) } else { format!("{:.1}s", *ms as f64 / 1000.0) });
                }
                if let Some(c) = cost_usd {
                    parts.push(format!("${c:.3}"));
                }
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_xs()
                    .text_color(theme.muted_foreground.opacity(0.7))
                    .child(div().h(sz(1.)).w(sz(18.)).bg(theme.border))
                    .child(parts.join(" · "))
                    .into_any_element()
            }
        }
    }

    /// The subagent session spawned by this tool call, if any.
    fn subagent_for(&self, tool_id: &str, cx: &App) -> Option<SessionId> {
        self.store
            .read(cx)
            .children(self.id)
            .into_iter()
            .find(|c| c.meta.subagent.as_ref().is_some_and(|i| i.tool_use_id == tool_id))
            .map(|c| c.meta.id)
    }

    /// Live strip of an orchestrator's subagents.
    fn render_subagent_strip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let pal = tokens::p(cx);
        let kids: Vec<SessionMeta> = self.store.read(cx).children(self.id).into_iter().map(|s| s.meta.clone()).collect();
        if kids.is_empty() {
            return None;
        }
        let running = kids.iter().filter(|k| k.status.is_live()).count();
        let mut row = h_flex().gap_2();
        for k in kids.iter().rev() {
            let id = k.id;
            let info = k.subagent.clone().unwrap_or_default();
            let needs = k.status == SessionStatus::NeedsInput;
            row = row.child(
                h_flex()
                    .id(ElementId::Name(format!("strip-{id}").into()))
                    .flex_none()
                    .max_w(sz(260.))
                    .gap(sz(8.))
                    .pl(sz(10.))
                    .pr(sz(12.))
                    .h(sz(34.))
                    .items_center()
                    .rounded(sz(tokens::radius::MD))
                    .border_1()
                    .border_color(if needs { pal.warning.opacity(0.5) } else { pal.border })
                    .bg(if needs { pal.warning.opacity(0.08) } else { pal.surface })
                    .cursor_pointer()
                    .hover(|d| d.border_color(pal.border_strong).bg(pal.surface_hover))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(ChatEvent::Open(id))))
                    .child(status_dot(ElementId::Name(format!("sdot-{id}").into()), &k.status, cx))
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(div().text_size(sz(tokens::text::SM)).truncate().child(k.title.clone()))
                            .child(div().text_size(sz(10.5)).text_color(if needs { pal.warning } else { pal.text_faint }).truncate().child(if needs {
                                "Needs your approval".to_string()
                            } else {
                                format!("{} tools · {}", info.tool_uses, format_tokens(info.tokens))
                            })),
                    ),
            );
        }
        Some(
            h_flex()
                .px_4()
                .py_2()
                .gap_3()
                .items_center()
                .border_b_1()
                .border_color(pal.border)
                .bg(pal.chrome.opacity(0.5))
                .child(
                    h_flex()
                        .flex_none()
                        .gap(sz(6.))
                        .items_center()
                        .text_size(sz(tokens::text::XS))
                        .text_color(pal.text_muted)
                        .child(icon("network").size(sz(13.)).text_color(pal.brand))
                        .child(format!("{running} of {} subagents running", kids.len())),
                )
                .child(div().id("strip-scroll").flex_1().min_w_0().overflow_x_scroll().child(row))
                .into_any_element(),
        )
    }

    /// Footer shown instead of the composer in a subagent chat.
    fn render_subagent_footer(&self, s: &SessionState, cx: &mut Context<Self>) -> AnyElement {
        let pal = tokens::p(cx);
        let parent = s.meta.parent;
        let live = s.meta.status.is_live();
        h_flex()
            .w_full()
            .gap_3()
            .px_4()
            .py_3()
            .items_center()
            .rounded(sz(tokens::radius::XL))
            .border_1()
            .border_color(pal.border)
            .bg(pal.surface)
            .child(icon("network").size(sz(16.)).text_color(pal.brand))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(sz(tokens::text::SM))
                    .text_color(pal.text_muted)
                    .child(if live {
                        "Runs on its own. Approve or answer its requests above."
                    } else {
                        "Finished. Its result went back to the orchestrator."
                    }),
            )
            .when(live, |d| {
                d.child(
                    Button::new("stop-sub")
                        .small()
                        .danger()
                        .icon(icon("square"))
                        .label("Stop")
                        .on_click(cx.listener(|this, _, window, cx| this.dispatch(SessionCommand::Cancel, window, cx))),
                )
            })
            .when_some(parent, |d, pid| {
                d.child(
                    Button::new("open-orch")
                        .small()
                        .outline()
                        .icon(icon("arrow-up-left"))
                        .label("Orchestrator")
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(ChatEvent::Open(pid)))),
                )
            })
            .into_any_element()
    }

    fn render_tool(&mut self, t: &ToolCall, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let open = self.expanded.contains(&t.id);
        let key = t.id.clone();
        let ic = match t.kind {
            ToolKind::Read => "file-text",
            ToolKind::Edit => "file-pen",
            ToolKind::Delete => "delete",
            ToolKind::Move => "replace",
            ToolKind::Search => "file-search",
            ToolKind::Execute => "terminal",
            ToolKind::Think => "brain",
            ToolKind::Fetch => "globe",
            ToolKind::Task => "users",
            ToolKind::Other => "wrench",
        };
        let status: AnyElement = match t.status {
            ToolStatus::Pending | ToolStatus::Running => Spinner::new().small().into_any_element(),
            ToolStatus::Completed => icon("check").size(sz(14.)).text_color(theme.success).into_any_element(),
            ToolStatus::Failed => icon("x").size(sz(14.)).text_color(theme.danger).into_any_element(),
        };
        let (adds, dels) = t.diffs.iter().fold((0, 0), |(a, d), f| {
            let (x, y) = diff_counts(f);
            (a + x, d + y)
        });
        let has_body = t.output.as_ref().is_some_and(|o| !o.trim().is_empty()) || !t.diffs.is_empty();
        v_flex()
            .rounded(sz(10.))
            .border_1()
            .border_color(theme.border)
            .overflow_hidden()
            .child(
                h_flex()
                    .id(ElementId::Name(format!("tool-{}", t.id).into()))
                    .px_3()
                    .py(sz(7.))
                    .gap_2()
                    .items_center()
                    .when(has_body, |d| d.cursor_pointer().hover(|d| d.bg(theme.list_hover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.expanded.remove(&key) {
                            this.expanded.insert(key.clone());
                        }
                        cx.notify();
                    }))
                    .child(icon(ic).size(sz(14.)).text_color(theme.muted_foreground))
                    .child(div().text_sm().font_weight(FontWeight::MEDIUM).flex_none().child(t.title.clone()))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .font_family(theme.mono_font_family.clone())
                            .text_color(theme.muted_foreground)
                            .child(t.input.clone().unwrap_or_default()),
                    )
                    .when(adds + dels > 0, |d| {
                        d.child(
                            h_flex()
                                .gap_1()
                                .text_xs()
                                .child(div().text_color(theme.success).child(format!("+{adds}")))
                                .child(div().text_color(theme.danger).child(format!("-{dels}"))),
                        )
                    })
                    .when_some(self.subagent_for(&t.id, cx), |d, child| {
                        d.child(
                            Button::new(ElementId::Name(format!("open-sub-{}", t.id).into()))
                                .small()
                                .outline()
                                .icon(icon("arrow-up-right"))
                                .label("Open")
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.stop_propagation();
                                    cx.emit(ChatEvent::Open(child));
                                })),
                        )
                    })
                    .child(status)
                    .when(has_body, |d| d.child(icon(if open { "chevron-down" } else { "chevron-right" }).size(sz(12.)).text_color(theme.muted_foreground))),
            )
            .when(open, |d| {
                let mut body = v_flex().border_t_1().border_color(theme.border).bg(theme.muted.opacity(0.4));
                for f in &t.diffs {
                    body = body.child(render_diff(f, cx));
                }
                if let Some(out) = &t.output
                    && !out.trim().is_empty() {
                        let shown: String = out.lines().take(400).collect::<Vec<_>>().join("\n");
                        body = body.child(
                            div()
                                .id(ElementId::Name(format!("out-{}", t.id).into()))
                                .max_h(sz(320.))
                                .overflow_y_scroll()
                                .p_3()
                                .text_xs()
                                .font_family(theme.mono_font_family.clone())
                                .text_color(theme.muted_foreground)
                                .child(shown),
                        );
                    }
                d.child(body)
            })
            .into_any_element()
    }

    fn render_permission(&mut self, p: &PermissionRequest, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        if let Some(choice) = &p.resolved {
            let opt = p.options.iter().find(|o| &o.id == choice);
            let allowed = opt.is_some_and(|o| matches!(o.kind, PermissionOptionKind::AllowOnce | PermissionOptionKind::AllowAlways));
            let label = opt.map(|o| o.label.clone()).unwrap_or_else(|| choice.replace('_', " "));
            return h_flex()
                .gap_2()
                .items_center()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(icon(if allowed { "shield-check" } else { "shield-alert" }).size(sz(13.)).text_color(if allowed { theme.success } else { theme.danger }))
                .child(format!("{} — {}", p.title, label))
                .into_any_element();
        }
        let amber = hex(0xF59E0B);
        let mut buttons = h_flex().gap_2().flex_wrap();
        for o in &p.options {
            let eid = p.id.clone();
            let oid = o.id.clone();
            let b = Button::new(ElementId::Name(format!("perm-{}-{}", p.id, o.id).into())).small().label(o.label.clone());
            let b = match o.kind {
                PermissionOptionKind::AllowOnce => b.primary(),
                PermissionOptionKind::AllowAlways => b.outline(),
                PermissionOptionKind::RejectOnce | PermissionOptionKind::RejectAlways => b.ghost(),
            };
            buttons = buttons.child(b.on_click(cx.listener(move |this, _, window, cx| {
                this.dispatch(SessionCommand::RespondPermission { entry_id: eid.clone(), option_id: oid.clone() }, window, cx);
            })));
        }
        appear(
            ElementId::Name(format!("perm-in-{}", p.id).into()),
            v_flex()
                .gap_2()
                .p_3()
                .rounded(sz(12.))
                .border_1()
                .border_color(amber.opacity(0.5))
                .bg(amber.opacity(0.06))
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(icon("shield-alert").size(sz(16.)).text_color(amber))
                        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("Permission needed"))
                        .child(div().text_sm().text_color(theme.muted_foreground).child(p.title.clone())),
                )
                .when_some(p.detail.clone(), |d, detail| {
                    let long = detail.len() > 300 || detail.lines().count() > 6;
                    d.child(
                        div()
                            .id(ElementId::Name(format!("perm-detail-{}", p.id).into()))
                            .max_h(sz(260.))
                            .overflow_y_scroll()
                            .p_2()
                            .rounded(sz(8.))
                            .bg(theme.background)
                            .border_1()
                            .border_color(theme.border)
                            .text_xs()
                            .when(!long, |d| d.font_family(theme.mono_font_family.clone()))
                            .child(detail),
                    )
                })
                .child(buttons),
        )
        .into_any_element()
    }

    fn render_question(&mut self, q: &QuestionRequest, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        if let Some(ans) = &q.answers {
            return v_flex()
                .gap_1()
                .children(q.questions.iter().zip(ans.iter().chain(std::iter::repeat(&vec![]))).map(|(qq, a)| {
                    h_flex()
                        .gap_2()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(icon("circle-question-mark").size(sz(13.)))
                        .child(qq.question.clone())
                        .child(div().text_color(theme.foreground).font_weight(FontWeight::MEDIUM).child(if a.is_empty() { "—".to_string() } else { a.join(", ") }))
                }))
                .into_any_element();
        }
        let eid = q.id.clone();
        let selections = self.answers.entry(eid.clone()).or_insert_with(|| vec![vec![]; q.questions.len()]).clone();
        let mut col = v_flex().gap_4();
        for (qi, qq) in q.questions.iter().enumerate() {
            let mut opts = h_flex().gap_2().flex_wrap();
            for o in &qq.options {
                let selected = selections.get(qi).is_some_and(|s| s.contains(&o.label));
                let label = o.label.clone();
                let eid2 = eid.clone();
                let multi = qq.multi_select;
                opts = opts.child(
                    v_flex()
                        .id(ElementId::Name(format!("q-{}-{}-{}", q.id, qi, o.label).into()))
                        .px_3()
                        .py_2()
                        .max_w(sz(320.))
                        .rounded(sz(10.))
                        .border_1()
                        .cursor_pointer()
                        .border_color(if selected { theme.primary } else { theme.border })
                        .when(selected, |d| d.bg(theme.primary.opacity(0.1)))
                        .hover(|d| d.bg(theme.list_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let sel = this.answers.entry(eid2.clone()).or_default();
                            while sel.len() <= qi {
                                sel.push(vec![]);
                            }
                            let s = &mut sel[qi];
                            if s.contains(&label) {
                                s.retain(|x| x != &label);
                            } else {
                                if !multi {
                                    s.clear();
                                }
                                s.push(label.clone());
                            }
                            cx.notify();
                        }))
                        .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(o.label.clone()))
                        .when_some(o.description.clone(), |d, desc| d.child(div().text_xs().text_color(theme.muted_foreground).child(desc))),
                );
            }
            let other = self.other_inputs.get(&(eid.clone(), qi)).cloned();
            col = col.child(
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .when_some(qq.header.clone(), |d, h| {
                                d.child(div().text_xs().px(sz(6.)).py(sz(1.)).rounded(sz(4.)).bg(theme.primary.opacity(0.15)).text_color(theme.primary).child(h))
                            })
                            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(qq.question.clone()))
                            .when(qq.multi_select, |d| d.child(div().text_xs().text_color(theme.muted_foreground).child("choose any"))),
                    )
                    .child(opts)
                    .when_some(other, |d, input| d.child(div().max_w(sz(420.)).child(Input::new(&input).small()))),
            );
        }
        let eid3 = eid.clone();
        let n = q.questions.len();
        appear(
            ElementId::Name(format!("q-in-{}", q.id).into()),
            v_flex()
                .gap_3()
                .p_4()
                .rounded(sz(12.))
                .border_1()
                .border_color(theme.primary.opacity(0.45))
                .bg(theme.primary.opacity(0.05))
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(icon("circle-question-mark").size(sz(16.)).text_color(theme.primary))
                        .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child("The agent has a question")),
                )
                .child(col)
                .child(
                    h_flex().gap_2().child(
                        Button::new(ElementId::Name(format!("q-submit-{}", q.id).into()))
                            .primary()
                            .small()
                            .icon(icon("send"))
                            .label("Answer")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let mut answers = this.answers.get(&eid3).cloned().unwrap_or_default();
                                answers.resize(n, vec![]);
                                for (i, a) in answers.iter_mut().enumerate() {
                                    if let Some(inp) = this.other_inputs.get(&(eid3.clone(), i)) {
                                        let v = inp.read(cx).value().trim().to_string();
                                        if !v.is_empty() {
                                            a.push(v);
                                        }
                                    }
                                }
                                this.dispatch(SessionCommand::AnswerQuestion { entry_id: eid3.clone(), answers }, window, cx);
                            })),
                    ),
                ),
        )
        .into_any_element()
    }

    fn render_composer(&mut self, s: &SessionState, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let working = matches!(s.meta.status, SessionStatus::Working | SessionStatus::NeedsInput | SessionStatus::Starting);
        let ended = matches!(s.meta.status, SessionStatus::Ended | SessionStatus::Error(_));
        let weak = cx.entity().downgrade();
        let agent = s.meta.agent;
        let mode = s.meta.permission_mode;
        let attachments = self.attachments.clone();
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let has_text = !self.input.read(cx).value().trim().is_empty();
        let matches = self.slash_matches(cx);
        let sel = self.slash_sel.min(matches.len().saturating_sub(1));
        let queue = s.meta.queue.clone();
        let editing = self.editing.is_some();
        let steer = agent.supports_steering();

        // Messages waiting for the current turn to end.
        let queue_view = (!queue.is_empty()).then(|| {
            let mut col = v_flex().gap(sz(4.)).mb_2();
            for q in &queue {
                let qid = q.id.clone();
                let qid2 = q.id.clone();
                let text = q.text.clone();
                col = col.child(
                    h_flex()
                        .gap_2()
                        .px(sz(12.))
                        .py(sz(8.))
                        .items_center()
                        .rounded(sz(tokens::radius::LG))
                        .bg(pal.surface_hover)
                        .border_1()
                        .border_color(pal.border)
                        .child(icon("clock").size(sz(14.)).text_color(pal.text_faint))
                        .child(div().flex_1().min_w_0().truncate().text_size(sz(tokens::text::BASE)).text_color(pal.text_muted).child(q.text.clone()))
                        .child(
                            Button::new(ElementId::Name(format!("q-now-{qid}").into()))
                                .ghost()
                                .small()
                                .label("Send now")
                                .tooltip("Interrupt the agent and send this now")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let id = this.id;
                                    let qid = qid.clone();
                                    this.store.update(cx, |s, cx| {
                                        s.dispatch(AppCommand::Dequeue { id, queue_id: qid }, window, cx);
                                    });
                                    this.dispatch(SessionCommand::Prompt { text: text.clone(), attachments: vec![], delivery: Delivery::Interrupt }, window, cx);
                                })),
                        )
                        .child(
                            Button::new(ElementId::Name(format!("q-rm-{qid2}").into()))
                                .ghost()
                                .small()
                                .icon(icon("x"))
                                .tooltip("Remove from queue")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let id = this.id;
                                    let qid = qid2.clone();
                                    this.store.update(cx, |s, cx| {
                                        s.dispatch(AppCommand::Dequeue { id, queue_id: qid }, window, cx);
                                    });
                                })),
                        ),
                );
            }
            col
        });

        let slash_menu = (!matches.is_empty()).then(|| {
            let mut list = v_flex().p(sz(4.)).gap(sz(1.));
            for (i, c) in matches.iter().enumerate() {
                let active = i == sel;
                let cmd = c.clone();
                list = list.child(
                    h_flex()
                        .id(ElementId::Name(format!("slash-{}", c.name).into()))
                        .gap_3()
                        .px(sz(10.))
                        .py(sz(7.))
                        .rounded(sz(tokens::radius::MD))
                        .items_center()
                        .cursor_pointer()
                        .when(active, |d| d.bg(pal.surface_hover))
                        .hover(|d| d.bg(pal.surface_hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.pick_slash(&cmd, window, cx);
                            this.input.update(cx, |i, cx| i.focus(window, cx));
                        }))
                        .child(div().flex_none().text_size(sz(tokens::text::BASE)).font_family(tokens::FONT_MONO).text_color(pal.text).child(format!("/{}", c.name)))
                        .when_some(c.hint.clone(), |d, h| d.child(div().flex_none().text_size(sz(tokens::text::SM)).text_color(pal.text_faint).child(h)))
                        .child(div().flex_1().min_w_0().truncate().text_size(sz(tokens::text::SM)).text_color(pal.text_muted).child(c.description.clone().unwrap_or_default())),
                );
            }
            div()
                .absolute()
                .bottom_full()
                .left_0()
                .right_0()
                .mb_2()
                .rounded(sz(tokens::radius::LG))
                .border_1()
                .border_color(pal.border_strong)
                .bg(pal.raised)
                .shadow(tokens::shadow_lg(&pal))
                .child(list)
                .child(
                    h_flex()
                        .px(sz(12.))
                        .py(sz(6.))
                        .gap_3()
                        .border_t_1()
                        .border_color(pal.border)
                        .text_size(sz(tokens::text::XS))
                        .text_color(pal.text_faint)
                        .child("↑↓ choose")
                        .child("Tab or Enter to insert")
                        .child("Esc to close"),
                )
        });

        let card = div()
            .relative()
            .w_full()
            .rounded(sz(tokens::radius::XL))
            .border_1()
            .border_color(if editing { pal.warning.opacity(0.6) } else if focused { pal.text_faint } else { pal.border_strong })
            .bg(pal.surface)
            .shadow(tokens::shadow_md(&pal))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.attachments.extend(paths.paths().iter().cloned());
                cx.notify();
            }))
            .when_some(slash_menu, |d, m| d.child(m))
            .when(editing, |d| {
                d.child(
                    h_flex()
                        .px(sz(14.))
                        .pt(sz(10.))
                        .gap_2()
                        .items_center()
                        .text_size(sz(tokens::text::SM))
                        .text_color(pal.warning)
                        .child(icon("pencil").size(sz(13.)))
                        .child(div().flex_1().child("Editing — the chat and any file changes rewind to this message, then it's sent again"))
                        .child(
                            Button::new("cancel-edit")
                                .ghost()
                                .small()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, window, cx| this.cancel_edit(window, cx))),
                        ),
                )
            })
            .when(!attachments.is_empty(), |d| {
                d.child(h_flex().px_3().pt_2().gap_1().flex_wrap().children(attachments.iter().enumerate().map(|(i, p)| {
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(file_chip(p, cx))
                        .child(
                            Button::new(ElementId::Name(format!("rm-att-{i}").into()))
                                .ghost()
                                .small()
                                .icon(icon("x"))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if i < this.attachments.len() {
                                        this.attachments.remove(i);
                                    }
                                    cx.notify();
                                })),
                        )
                })))
            })
            .child(div().px(sz(16.)).pt(sz(12.)).pb_1().text_size(sz(tokens::text::READ)).child(Textarea::new(&self.input).appearance(false)))
            .child(
                h_flex()
                    .px(sz(8.))
                    .pb(sz(8.))
                    .gap_1()
                    .items_center()
                    .child(
                        Button::new("attach")
                            .ghost()
                            .icon(icon("paperclip"))
                            .tooltip("Attach files (or drop them here)")
                            .on_click(cx.listener(|this, _, window, cx| this.attach(window, cx))),
                    )
                    .child(mode_menu("chat-mode", Some(agent), mode, weak, |this: &mut ChatView, m, window, cx| {
                        this.dispatch(SessionCommand::SetMode { mode: m }, window, cx);
                    }))
                    .child(div().flex_1())
                    .when(ended && !editing, |d| d.child(div().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).mr_2().child("Sending resumes this chat")))
                    .when_some(self.working_since.filter(|_| working), |d, since| {
                        let secs = since.elapsed().as_secs();
                        d.child(div().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).mr_1().child(format!("{}:{:02}", secs / 60, secs % 60)))
                    })
                    .when(working && has_text && !editing, |d| {
                        d.when(steer, |d| {
                            d.child(
                                Button::new("steer")
                                    .ghost()
                                    .icon(icon("corner-down-right"))
                                    .label("Steer")
                                    .tooltip("Send into the running turn; the agent reads it at its next step")
                                    .on_click(cx.listener(|this, _, window, cx| this.send(Delivery::Steer, window, cx))),
                            )
                        })
                        .child(
                            Button::new("send-now")
                                .ghost()
                                .icon(icon("zap"))
                                .label("Send now")
                                .tooltip("Stop the current turn and send this immediately")
                                .on_click(cx.listener(|this, _, window, cx| this.send(Delivery::Interrupt, window, cx))),
                        )
                    })
                    .when(working, |d| {
                        d.child(
                            Button::new("stop")
                                .ghost()
                                .icon(icon("square"))
                                .tooltip("Stop the agent")
                                .on_click(cx.listener(|this, _, window, cx| this.dispatch(SessionCommand::Cancel, window, cx))),
                        )
                    })
                    .child(
                        div()
                            .id("send")
                            .size(sz(34.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(sz(tokens::radius::LG))
                            .bg(if has_text { pal.text } else { pal.surface_hover })
                            .text_color(if has_text { pal.bg } else { pal.text_faint })
                            .cursor_pointer()
                            .when(has_text, |d| d.hover(|d| d.opacity(0.85)))
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(if working && !editing { "Queue — sends when the agent finishes (Enter)" } else { "Send (Enter)" }).build(window, cx)
                            })
                            .on_click(cx.listener(|this, _, window, cx| this.send(Delivery::Normal, window, cx)))
                            .child(icon(if working && !editing { "list-plus" } else { "arrow-up" }).size(sz(17.))),
                    ),
            );

        v_flex().w_full().when_some(queue_view, |d, q| d.child(q)).child(card)
    }
}

impl Render for ChatView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(s) = self.store.read(cx).session(self.id).cloned() else {
            return div().size_full().child(empty_state("message-square", "Chat not found", "It may have been deleted.", cx)).into_any_element();
        };
        if self.design.is_none() && s.meta.kind == SessionKind::Design {
            let project = s.meta.project.clone();
            self.design = Some(cx.new(|cx| DesignView::new(project, window, cx)));
        }
        let theme = cx.theme().clone();
        let live = s.meta.status.is_live();
        let header = self.render_header(&s, cx);
        let mut transcript = v_flex().gap_4().w_full().max_w(sz(860.)).mx_auto().px_6().py_6();
        if s.entries.is_empty() {
            transcript = transcript.child(
                v_flex().items_center().pt(sz(80.)).child(match s.meta.status {
                    SessionStatus::Starting => v_flex()
                        .items_center()
                        .gap_3()
                        .child(Spinner::new())
                        .child(div().text_sm().text_color(theme.muted_foreground).child(format!("Starting {}…", s.meta.agent.display_name())))
                        .into_any_element(),
                    SessionStatus::Error(ref e) => empty_state("circle-x", "The agent couldn't start", e.clone(), cx).into_any_element(),
                    _ if s.meta.is_subagent() => empty_state("network", "Subagent starting", "Its steps will appear here as it works.", cx).into_any_element(),
                    _ => empty_state(
                        if s.meta.kind == SessionKind::Design { "palette" } else { "message-square" },
                        "Say hello",
                        format!("{} is ready in {}", s.meta.agent.display_name(), s.meta.project_name()),
                        cx,
                    )
                    .into_any_element(),
                }),
            );
        }
        for e in &s.entries {
            let el = self.render_entry(e, live, cx);
            transcript = transcript.child(div().id(ElementId::Name(format!("e-{}", e.id()).into())).child(el));
        }
        if s.meta.status == SessionStatus::Working {
            let streaming_text = s.entries.last().is_some_and(|e| matches!(e, Entry::Assistant { streaming: true, .. }));
            if !streaming_text {
                transcript = transcript.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(Spinner::new().small().color(agent_color(s.meta.agent)))
                        .child(shimmer_text("Working…", "working-shimmer", cx)),
                );
            }
        }
        let composer = if s.meta.is_subagent() {
            self.render_subagent_footer(&s, cx)
        } else {
            self.render_composer(&s, window, cx).into_any_element()
        };
        let strip = self.render_subagent_strip(cx);
        let parent_crumb = s.meta.parent.and_then(|p| self.store.read(cx).session(p).map(|ps| (p, ps.meta.title.clone())));

        let chat_column = v_flex()
            .size_full()
            .min_w_0()
            .when_some(parent_crumb, |d, (pid, ptitle)| {
                let pal = tokens::p(cx);
                d.child(
                    h_flex()
                        .id("parent-crumb")
                        .px_4()
                        .py(sz(6.))
                        .gap(sz(6.))
                        .items_center()
                        .bg(pal.brand_soft)
                        .border_b_1()
                        .border_color(pal.border)
                        .text_size(sz(tokens::text::XS))
                        .text_color(pal.text_muted)
                        .cursor_pointer()
                        .hover(|d| d.text_color(pal.text))
                        .on_click(cx.listener(move |_, _, _, cx| cx.emit(ChatEvent::Open(pid))))
                        .child(icon("arrow-up-left").size(sz(12.)).text_color(pal.brand))
                        .child("Subagent of")
                        .child(div().font_weight(FontWeight::MEDIUM).text_color(pal.text).truncate().child(ptitle)),
                )
            })
            .child(header)
            .when_some(strip, |d, strip| d.child(strip))
            .child(
                div()
                    .id("transcript")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .on_scroll(cx.listener(|this, ev: &ScrollEvent, window, _| {
                        let dy = ev.delta.pixel_delta(window.line_height()).y;
                        if dy > sz(0.) {
                            this.follow = false;
                        } else {
                            let max = this.scroll.max_offset().y;
                            if -this.scroll.offset().y >= max - sz(40.) {
                                this.follow = true;
                            }
                        }
                    }))
                    .child(transcript),
            )
            .when(!self.follow, |d| {
                d.child(
                    div().relative().child(
                        div().absolute().bottom(sz(8.)).left_0().right_0().flex().justify_center().child(
                            Button::new("jump-bottom")
                                .small()
                                .outline()
                                .icon(icon("arrow-down"))
                                .label("Latest")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.follow = true;
                                    this.scroll.scroll_to_bottom();
                                    cx.notify();
                                })),
                        ),
                    ),
                )
            })
            .child(div().w_full().max_w(sz(860.)).mx_auto().px_6().pb_4().child(composer));

        let _ = window;
        match &self.design {
            Some(design) => h_flex()
                .size_full()
                .child(div().w(relative(0.42)).min_w(sz(380.)).h_full().border_r_1().border_color(theme.border).child(chat_column))
                .child(div().flex_1().min_w_0().h_full().child(design.clone()))
                .into_any_element(),
            None => chat_column.into_any_element(),
        }
    }
}

fn shimmer_text(text: &'static str, id: impl Into<ElementId>, cx: &App) -> impl IntoElement {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text)
        .with_animation(id, Animation::new(Duration::from_millis(1600)).repeat().with_easing(pulsating_between(0.45, 1.0)), |d, t| d.opacity(t))
}

fn file_chip(path: &std::path::Path, cx: &App) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let image = mime_for(path).is_some_and(|m| m.starts_with("image/"));
    h_flex()
        .gap_1()
        .px_2()
        .py(sz(3.))
        .rounded(sz(6.))
        .border_1()
        .border_color(theme.border)
        .text_xs()
        .items_center()
        .child(icon(if image { "image" } else { "file" }).size(sz(12.)).text_color(theme.muted_foreground))
        .child(name)
}

fn mime_for(p: &std::path::Path) -> Option<String> {
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    Some(
        match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "pdf" => "application/pdf",
            _ => "text/plain",
        }
        .into(),
    )
}

fn diff_counts(f: &FileDiff) -> (usize, usize) {
    let new = f.new_text.lines().count();
    let old = f.old_text.as_ref().map(|o| o.lines().count()).unwrap_or(0);
    (new, old)
}

fn render_diff(f: &FileDiff, cx: &App) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let mut lines = v_flex().text_xs().font_family(theme.mono_font_family.clone());
    let mut count = 0;
    if let Some(old) = &f.old_text {
        for l in old.lines() {
            if count > 240 {
                break;
            }
            count += 1;
            lines = lines.child(div().px_3().bg(theme.danger.opacity(0.10)).text_color(theme.danger).child(format!("- {l}")));
        }
    }
    for l in f.new_text.lines() {
        if count > 480 {
            lines = lines.child(div().px_3().text_color(theme.muted_foreground).child("…"));
            break;
        }
        count += 1;
        lines = lines.child(div().px_3().bg(theme.success.opacity(0.10)).text_color(theme.success).child(format!("+ {l}")));
    }
    v_flex()
        .child(
            div()
                .px_3()
                .py_1()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.muted_foreground)
                .border_b_1()
                .border_color(theme.border)
                .child(tildify(&f.path)),
        )
        .child(div().id(ElementId::Name(format!("diff-{}", f.path.display()).into())).max_h(sz(360.)).overflow_y_scroll().py_1().child(lines))
}

#[allow(dead_code)]
fn _n(window: &mut Window, cx: &mut App) {
    window.push_notification(Notification::info(""), cx);
}

/// Markdown styling for agent replies: comfortable rhythm and framed code blocks.
fn markdown_style(cx: &App) -> gpui_kit::component::text::TextViewStyle {
    let pal = tokens::p(cx);
    let code = StyleRefinement::default()
        .bg(pal.code_bg)
        .border_color(pal.border)
        .rounded(sz(tokens::radius::MD));
    gpui_kit::component::text::TextViewStyle {
        paragraph_gap: rems(0.8),
        highlight_theme: cx.theme().highlight_theme.clone(),
        code_block: code,
        ..Default::default()
    }
}

/// Small icon button shown under a message on hover.
fn msg_action(id: ElementId, icon_name: &'static str, tip: &'static str, pal: &tokens::Palette) -> Stateful<Div> {
    let pal = *pal;
    div()
        .id(id)
        .size(sz(28.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(sz(tokens::radius::MD))
        .text_color(pal.text_faint)
        .cursor_pointer()
        .hover(move |d| d.bg(pal.surface_hover).text_color(pal.text))
        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tip).build(window, cx))
        .child(icon(icon_name).size(sz(15.)))
}
