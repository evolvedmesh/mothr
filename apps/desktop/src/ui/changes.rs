//! The Changes panel: every file edit in a chat, with accept / reject for those awaiting review.

use super::diff::{parse_section, section_label, stat_label, styled};
use super::kit::*;
use super::tokens::{self, sz};
use crate::store::Store;
use gpui_kit::component::{
    Sizable as _,
    button::{Button, ButtonVariants as _},
    diff::{Diff, DiffFile, DiffMode, DiffState},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mothr_core::*;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::rc::Rc;

pub enum ChangesEvent {
    Close,
}

impl EventEmitter<ChangesEvent> for ChangesPanel {}

/// One file section of the panel.
#[derive(Clone)]
struct Section {
    label: SharedString,
    session: SessionId,
    path: PathBuf,
    stats: DiffStats,
    state: ChangeState,
    accept: Option<(String, String)>,
    reject: Option<(String, String)>,
}

impl Section {
    fn name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.path.display().to_string())
    }

    fn folder(&self, project: &std::path::Path) -> String {
        let parent = self.path.parent().unwrap_or(std::path::Path::new(""));
        match parent.strip_prefix(project) {
            Ok(rel) => rel.display().to_string(),
            Err(_) => tildify(parent),
        }
    }
}

pub struct ChangesPanel {
    id: SessionId,
    store: Entity<Store>,
    diff: Entity<DiffState>,
    sections: Rc<Vec<Section>>,
    fingerprint: Option<u64>,
    /// Developer hook `MOTHR_DEV_REVIEW=accept|reject`: answer the first waiting edits.
    dev_review: Option<bool>,
    _subs: Vec<Subscription>,
}

impl ChangesPanel {
    pub fn new(id: SessionId, store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        let mode = if std::env::var("MOTHR_DIFF_MODE").as_deref() == Ok("split") { DiffMode::Split } else { DiffMode::Unified };
        let diff = cx.new(|cx| DiffState::new(Vec::<DiffFile>::new(), cx).with_mode(mode));
        let subs = vec![cx.observe(&store, |this, _, cx| {
            this.sync(cx);
            cx.notify();
        })];
        let dev_review = std::env::var("MOTHR_DEV_REVIEW").ok().map(|v| v == "accept");
        let mut this = Self { id, store, diff, sections: Rc::new(vec![]), fingerprint: None, dev_review, _subs: subs };
        this.sync(cx);
        this
    }

    /// The chat and its subagents: their edits land in the same project.
    fn sessions<'a>(&self, store: &'a Store) -> Vec<&'a SessionState> {
        store.session(self.id).into_iter().chain(store.children(self.id)).collect()
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        let sessions = self.sessions(store);
        let mut h = DefaultHasher::new();
        for s in &sessions {
            s.changes_fingerprint().hash(&mut h);
        }
        let fingerprint = h.finish();
        if self.fingerprint == Some(fingerprint) {
            return;
        }
        let mut changes: Vec<(SessionId, FileChange)> = vec![];
        for s in &sessions {
            changes.extend(s.file_changes().into_iter().map(|c| (s.meta.id, c)));
        }
        // What needs a decision comes first, then the most recent work.
        changes.reverse();
        changes.sort_by_key(|(_, c)| !c.is_pending());
        let mut sections = vec![];
        let mut files = vec![];
        for (session, change) in &changes {
            let option = |o: Option<(&str, &PermissionOption)>| o.map(|(p, o)| (p.to_string(), o.id.clone()));
            for (path, diffs) in change.files() {
                let label = section_label(sections.len(), path);
                let Some(file) = parse_section(&label, &diffs) else { continue };
                let mut stats = DiffStats::default();
                diffs.iter().for_each(|d| stats.add(d.stats()));
                files.push(file);
                sections.push(Section {
                    label: label.into(),
                    session: *session,
                    path: path.to_path_buf(),
                    stats,
                    state: change.state.clone(),
                    accept: option(change.accept_option()),
                    reject: option(change.reject_option()),
                });
            }
        }
        // Edits that never reached the disk stay out of the way until asked for.
        let dropped: Vec<SharedString> =
            sections.iter().filter(|s: &&Section| matches!(s.state, ChangeState::Rejected | ChangeState::Failed)).map(|s| s.label.clone()).collect();
        self.sections = Rc::new(sections);
        self.fingerprint = Some(fingerprint);
        self.diff.update(cx, |d, cx| {
            d.set_files(files, cx);
            for label in &dropped {
                d.set_file_collapsed(label, true, cx);
            }
        });
    }

    fn respond(&self, session: SessionId, choice: &(String, String), window: &mut Window, cx: &mut Context<Self>) {
        let command = SessionCommand::RespondPermission { entry_id: choice.0.clone(), option_id: choice.1.clone() };
        self.store.update(cx, |s, cx| {
            s.dispatch(AppCommand::Session { id: session, command }, window, cx);
        });
    }

    /// Accept or reject everything waiting. One tool call can span several file sections.
    fn respond_all(&self, accept: bool, window: &mut Window, cx: &mut Context<Self>) {
        let mut done: Vec<String> = vec![];
        for s in self.sections.iter() {
            let Some(choice) = (if accept { &s.accept } else { &s.reject }) else { continue };
            if !done.contains(&choice.0) {
                done.push(choice.0.clone());
                self.respond(s.session, choice, window, cx);
            }
        }
    }

    fn set_mode(&self, mode: DiffMode, cx: &mut Context<Self>) {
        self.diff.update(cx, |d, cx| d.set_mode(mode, cx));
        cx.notify();
    }

    fn render_header(&self, summary: ChangeSummary, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let mode = self.diff.read(cx).mode();
        h_flex()
            .flex_none()
            .h(sz(48.))
            .pl(sz(16.))
            .pr(sz(8.))
            .gap(sz(10.))
            .items_center()
            .border_b_1()
            .border_color(pal.border)
            .child(div().text_size(sz(tokens::text::LG)).font_weight(FontWeight::SEMIBOLD).child("Changes"))
            .when(summary.files > 0, |d| {
                d.child(
                    div()
                        .text_size(sz(tokens::text::SM))
                        .text_color(pal.text_faint)
                        .child(format!("{} file{}", summary.files, if summary.files == 1 { "" } else { "s" })),
                )
                .child(stat_label(summary.stats, &pal))
            })
            .child(div().flex_1())
            .child(
                segment_track(&pal)
                    .child(segment("diff-unified", "Unified", mode == DiffMode::Unified, &pal).on_click(cx.listener(|this, _, _, cx| this.set_mode(DiffMode::Unified, cx))))
                    .child(segment("diff-split", "Split", mode == DiffMode::Split, &pal).on_click(cx.listener(|this, _, _, cx| this.set_mode(DiffMode::Split, cx)))),
            )
            .child(
                Button::new("close-changes")
                    .ghost()
                    .small()
                    .icon(icon("x"))
                    .tooltip("Close")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ChangesEvent::Close))),
            )
    }

    /// The decision bar, shown while edits are waiting.
    fn render_review_bar(&self, pending: usize, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        h_flex()
            .flex_none()
            .px(sz(16.))
            .py(sz(9.))
            .gap(sz(8.))
            .items_center()
            .bg(pal.warning.opacity(0.08))
            .border_b_1()
            .border_color(pal.warning.opacity(0.25))
            .child(icon("shield-alert").size(sz(15.)).text_color(pal.warning))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(sz(tokens::text::BASE))
                    .child(if pending == 1 { "1 edit is waiting for you".to_string() } else { format!("{pending} edits are waiting for you") }),
            )
            .when(pending > 1, |d| {
                d.child(
                    Button::new("reject-all")
                        .ghost()
                        .small()
                        .label("Reject all")
                        .on_click(cx.listener(|this, _, window, cx| this.respond_all(false, window, cx))),
                )
                .child(
                    Button::new("accept-all")
                        .primary()
                        .small()
                        .label("Accept all")
                        .on_click(cx.listener(|this, _, window, cx| this.respond_all(true, window, cx))),
                )
            })
    }

    fn render_file_list(&self, project: &std::path::Path, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let mut list = v_flex().p(sz(4.));
        for (i, s) in self.sections.iter().enumerate() {
            let label = s.label.clone();
            let folder = s.folder(project);
            let (ic, color) = state_mark(&s.state, &pal);
            list = list.child(
                h_flex()
                    .id(ElementId::Name(format!("change-file-{i}").into()))
                    .h(sz(28.))
                    .px(sz(10.))
                    .gap(sz(8.))
                    .items_center()
                    .rounded(sz(tokens::radius::SM))
                    .cursor_pointer()
                    .hover(|d| d.bg(pal.surface_hover))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.diff.update(cx, |d, cx| d.scroll_to_file(&label, cx));
                    }))
                    .child(icon(ic).flex_none().size(sz(13.)).text_color(color))
                    .child(div().flex_none().max_w(relative(0.6)).truncate().text_size(sz(tokens::text::SM)).child(s.name()))
                    .child(div().flex_1().min_w_0().truncate().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).child(folder))
                    .child(stat_label(s.stats, &pal)),
            );
        }
        div()
            .id("change-files")
            .flex_none()
            .max_h(sz(148.))
            .overflow_y_scroll()
            .border_b_1()
            .border_color(pal.border)
            .bg(pal.chrome)
            .child(list)
    }

    /// Says what the current permission mode means for edits, with the way to change it.
    fn render_mode_note(&self, mode: PermissionMode, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let applies_directly = matches!(mode, PermissionMode::AcceptEdits | PermissionMode::Auto | PermissionMode::AllowAll);
        let note = match mode {
            PermissionMode::Ask => "Ask mode: you accept or reject each edit before it's written.".to_string(),
            PermissionMode::Plan => "Plan mode: the agent can't change files.".to_string(),
            m => format!("{} writes edits without asking, so there's nothing to accept.", m.label()),
        };
        let id = self.id;
        h_flex()
            .flex_none()
            .min_h(sz(38.))
            .px(sz(16.))
            .py(sz(5.))
            .gap(sz(8.))
            .items_center()
            .border_t_1()
            .border_color(pal.border)
            .bg(pal.chrome)
            .child(icon(super::home::mode_icon(mode)).flex_none().size(sz(13.)).text_color(pal.text_faint))
            .child(div().flex_1().min_w_0().text_size(sz(tokens::text::XS)).text_color(pal.text_muted).child(note))
            .when(applies_directly, |d| {
                d.child(
                    Button::new("review-first")
                        .ghost()
                        .xsmall()
                        .label("Switch to Ask")
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let command = SessionCommand::SetMode { mode: PermissionMode::Ask };
                            this.store.update(cx, |s, cx| {
                                s.dispatch(AppCommand::Session { id, command }, window, cx);
                            });
                        })),
                )
            })
    }
}

/// Icon and color for a change's state.
fn state_mark(state: &ChangeState, pal: &tokens::Palette) -> (&'static str, Hsla) {
    match state {
        ChangeState::Pending { .. } => ("circle-dot", pal.warning),
        ChangeState::Running => ("loader-circle", pal.brand),
        ChangeState::Applied => ("circle-check", pal.success),
        ChangeState::Rejected => ("circle-slash", pal.text_faint),
        ChangeState::Failed => ("circle-x", pal.danger),
    }
}

fn state_label(state: &ChangeState) -> &'static str {
    match state {
        ChangeState::Pending { .. } => "Waiting",
        ChangeState::Running => "Applying",
        ChangeState::Applied => "Applied",
        ChangeState::Rejected => "Rejected",
        ChangeState::Failed => "Failed",
    }
}

impl Render for ChangesPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = tokens::p(cx);
        let store = self.store.read(cx);
        let Some(session) = store.session(self.id) else { return div().into_any_element() };
        let project = session.meta.project.clone();
        let mode = session.meta.permission_mode;
        let changes: Vec<FileChange> = self.sessions(store).iter().flat_map(|s| s.file_changes()).collect();
        let summary = ChangeSummary::of(&changes);
        if let Some(accept) = self.dev_review.take_if(|_| summary.pending > 0) {
            cx.defer_in(window, move |this, window, cx| this.respond_all(accept, window, cx));
        }
        let by_label: HashMap<SharedString, Section> = self.sections.iter().map(|s| (s.label.clone(), s.clone())).collect();
        let weak = cx.entity().downgrade();
        let header_project = project.clone();

        let body = if self.sections.is_empty() {
            div().flex_1().flex().items_center().justify_center().child(empty_state("file-diff", "No file changes yet", "Edits the agent makes in this chat will appear here.", cx)).into_any_element()
        } else {
            let diff = styled(Diff::new(&self.diff)).border_0().render_header(move |file: &DiffFile, _, cx: &mut App| {
                let pal = tokens::p(cx);
                let Some(s) = by_label.get(file.path()).cloned() else { return div().into_any_element() };
                let (ic, color) = state_mark(&s.state, &pal);
                let folder = s.folder(&header_project);
                let key = s.label.clone();
                let actions = s.accept.is_some() || s.reject.is_some();
                let session = s.session;
                let (accept, reject) = (s.accept.clone(), s.reject.clone());
                let (w1, w2) = (weak.clone(), weak.clone());
                h_flex()
                    .w_full()
                    .min_w_0()
                    .h(sz(28.))
                    .gap(sz(8.))
                    .items_center()
                    .child(icon(ic).flex_none().size(sz(14.)).text_color(color))
                    .child(div().flex_none().max_w(relative(0.5)).truncate().text_size(sz(tokens::text::BASE)).font_weight(FontWeight::MEDIUM).child(s.name()))
                    .child(div().flex_1().min_w_0().truncate().text_size(sz(tokens::text::XS)).text_color(pal.text_faint).child(folder))
                    .child(stat_label(s.stats, &pal))
                    .when(!actions, |d| d.child(div().flex_none().text_size(sz(tokens::text::XS)).text_color(color).child(state_label(&s.state))))
                    .when_some(reject, |d, choice| {
                        d.child(Button::new(ElementId::Name(format!("reject-{key}").into())).ghost().xsmall().label("Reject").on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            if let Some(p) = w1.upgrade() {
                                p.update(cx, |p, cx| p.respond(session, &choice, window, cx));
                            }
                        }))
                    })
                    .when_some(accept, |d, choice| {
                        d.child(Button::new(ElementId::Name(format!("accept-{key}").into())).primary().xsmall().label("Accept").on_click(move |_, window, cx| {
                            cx.stop_propagation();
                            if let Some(p) = w2.upgrade() {
                                p.update(cx, |p, cx| p.respond(session, &choice, window, cx));
                            }
                        }))
                    })
                    .into_any_element()
            });
            div().flex_1().min_h_0().child(diff).into_any_element()
        };

        v_flex()
            .size_full()
            .bg(pal.bg)
            .child(self.render_header(summary, cx))
            .when(summary.pending > 0, |d| d.child(self.render_review_bar(summary.pending, cx)))
            .when(self.sections.len() > 1, |d| d.child(self.render_file_list(&project, cx)))
            .child(body)
            .child(self.render_mode_note(mode, cx))
            .into_any_element()
    }
}
