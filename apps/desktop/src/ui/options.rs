//! Reasoning-effort and context-window pickers, shared by the new-chat box and the chat header.

use super::kit::*;
use gpui_kit::component::{
    button::{Button, ButtonVariants as _},
    menu::{DropdownMenu as _, PopupMenuItem},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mothr_core::{ChoiceInfo, ModelInfo, effort_label};

/// What `model` can be set to. With no model chosen (the agent's default) the entry the agent
/// calls "default" is used, else the first model that has the setting.
pub fn settings_for(models: &[ModelInfo], model: Option<&str>) -> (Vec<String>, Vec<ChoiceInfo>) {
    let chosen = model.and_then(|m| models.iter().find(|x| x.id == m));
    let fallback = || models.iter().find(|x| x.id == "default");
    let efforts = chosen
        .or_else(fallback)
        .filter(|m| !m.efforts.is_empty())
        .or_else(|| if model.is_none() { models.iter().find(|m| !m.efforts.is_empty()) } else { None })
        .map(|m| m.efforts.clone())
        .unwrap_or_default();
    let contexts = chosen
        .or_else(fallback)
        .filter(|m| !m.contexts.is_empty())
        .or_else(|| if model.is_none() { models.iter().find(|m| !m.contexts.is_empty()) } else { None })
        .map(|m| m.contexts.clone())
        .unwrap_or_default();
    (efforts, contexts)
}

/// How a picker is laid out: new chats can go back to the agent's default and have room for a
/// label; a running chat can't unset a value and may be short on space.
#[derive(Clone, Copy)]
pub struct Style {
    pub allow_default: bool,
    pub compact: bool,
}

/// "Reasoning" dropdown. `set` receives `None` for the agent's own default.
pub fn effort_menu<T: 'static>(
    id: &'static str,
    efforts: Vec<String>,
    current: Option<String>,
    style: Style,
    weak: WeakEntity<T>,
    set: impl Fn(&mut T, Option<String>, &mut Window, &mut Context<T>) + Clone + 'static,
) -> impl IntoElement {
    let label = current.as_deref().map(effort_label).unwrap_or_else(|| "Reasoning".into());
    Button::new(id)
        .ghost()
        .icon(icon("gauge"))
        .when(!style.compact, |b| b.label(label.clone()).dropdown_caret(true))
        .tooltip(if style.compact { format!("Reasoning: {label}") } else { "Reasoning effort".to_string() })
        .dropdown_menu(move |mut menu, _, _| {
            menu = menu.label("Reasoning");
            let (w, s) = (weak.clone(), set.clone());
            if style.allow_default {
                menu = menu.item(PopupMenuItem::new("Agent default").checked(current.is_none()).on_click(move |_, window, cx| {
                    if let Some(e) = w.upgrade() {
                        e.update(cx, |t, cx| s(t, None, window, cx));
                    }
                }));
            }
            for effort in &efforts {
                let (w, s, value) = (weak.clone(), set.clone(), effort.clone());
                menu = menu.item(PopupMenuItem::new(effort_label(effort)).checked(current.as_deref() == Some(effort.as_str())).on_click(move |_, window, cx| {
                    if let Some(e) = w.upgrade() {
                        e.update(cx, |t, cx| s(t, Some(value.clone()), window, cx));
                    }
                }));
            }
            menu
        })
}

/// "Context window" dropdown (200k, 1M …). `set` receives `None` for the agent's own default.
pub fn context_menu<T: 'static>(
    id: &'static str,
    contexts: Vec<ChoiceInfo>,
    current: Option<String>,
    style: Style,
    weak: WeakEntity<T>,
    set: impl Fn(&mut T, Option<String>, &mut Window, &mut Context<T>) + Clone + 'static,
) -> impl IntoElement {
    let label = current
        .as_deref()
        .and_then(|c| contexts.iter().find(|x| x.id == c))
        .map(|c| c.label.clone())
        .unwrap_or_else(|| "Context".into());
    Button::new(id)
        .ghost()
        .icon(icon("scaling"))
        .when(!style.compact, |b| b.label(label.clone()).dropdown_caret(true))
        .tooltip(if style.compact { format!("Context window: {label}") } else { "Context window".to_string() })
        .dropdown_menu(move |mut menu, _, _| {
            menu = menu.label("Context window");
            let (w, s) = (weak.clone(), set.clone());
            if style.allow_default {
                menu = menu.item(PopupMenuItem::new("Agent default").checked(current.is_none()).on_click(move |_, window, cx| {
                    if let Some(e) = w.upgrade() {
                        e.update(cx, |t, cx| s(t, None, window, cx));
                    }
                }));
            }
            for c in &contexts {
                let (w, s, value) = (weak.clone(), set.clone(), c.id.clone());
                menu = menu.item(PopupMenuItem::new(c.label.clone()).checked(current.as_deref() == Some(c.id.as_str())).on_click(move |_, window, cx| {
                    if let Some(e) = w.upgrade() {
                        e.update(cx, |t, cx| s(t, Some(value.clone()), window, cx));
                    }
                }));
            }
            menu
        })
}
