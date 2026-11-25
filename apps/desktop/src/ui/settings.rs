//! Settings: appearance, agents, projects, defaults and Mothr Remote.

use super::home::mode_menu;
use super::kit::*;
use super::tokens;
use super::theme::{apply_preference, card};
use crate::remote::{RemoteController, TunnelAvailability, detect_tunnels, generate_token, qr_matrix};
use crate::store::Store;
use gpui_kit::component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    input::{Input, InputEvent, InputState},
    switch::Switch,
    v_flex,
};
use gpui_kit::*;
use super::tokens::sz;
use gpui_kit::prelude::FluentBuilder as _;
use mothr_core::*;

pub struct SettingsView {
    store: Entity<Store>,
    remote: RemoteController,
    tunnels: TunnelAvailability,
    show_token: bool,
    scroll: ScrollHandle,
    /// Your own accent, as "#RRGGBB".
    accent_input: Entity<InputState>,
    _subs: Vec<Subscription>,
}

impl SettingsView {
    pub fn new(store: Entity<Store>, remote: RemoteController, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let custom = store.read(cx).hub.settings().appearance.custom_accent.unwrap_or_default();
        let accent_input = cx.new(|cx| InputState::new(window, cx).placeholder("#8B7CF6").default_value(custom));
        let subs = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.subscribe(&accent_input, |this, input, ev: &InputEvent, cx| {
                if let InputEvent::Change = ev {
                    let text = input.read(cx).value().trim().to_string();
                    // Applied as soon as it reads as a color.
                    if let Some(hex) = parse_hex_color(&text) {
                        this.edit_appearance(cx, |a| {
                            a.accent = Accent::Custom;
                            a.custom_accent = Some(format!("#{hex:06X}"));
                        });
                    }
                }
            }),
        ];
        Self { store, remote, tunnels: detect_tunnels(), show_token: false, scroll: ScrollHandle::new(), accent_input, _subs: subs }
    }

    fn edit_appearance(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Appearance)) {
        super::theme::edit_appearance(&self.store, f, cx);
        cx.notify();
    }

    /// Accent, tone and which side the sidebar sits on.
    fn render_look(&mut self, s: &Settings, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let pal = tokens::p(cx);
        let a = s.appearance.clone();

        let mut accents = h_flex().gap(sz(10.)).items_center().flex_wrap();
        for preset in Accent::PRESETS {
            let color = hex(preset.hex(pal.dark).unwrap_or(0x8B7CF6));
            let selected = a.accent == preset;
            accents = accents.child(
                div()
                    .id(SharedString::from(format!("accent-{}", preset.label())))
                    .p(sz(2.))
                    .rounded_full()
                    .border_2()
                    .border_color(if selected { color } else { gpui_kit::transparent_black() })
                    .cursor_pointer()
                    .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(preset.label()).build(window, cx))
                    .on_click(cx.listener(move |this, _, _, cx| this.edit_appearance(cx, |a| a.accent = preset)))
                    .child(div().size(sz(22.)).rounded_full().bg(color).border_1().border_color(pal.border_strong)),
            );
        }
        let custom = a.accent == Accent::Custom;
        let custom_color = hex(a.accent_hex(pal.dark));
        accents = accents.child(
            h_flex()
                .gap(sz(8.))
                .items_center()
                .pl(sz(6.))
                .child(
                    div()
                        .id("accent-custom")
                        .p(sz(2.))
                        .rounded_full()
                        .border_2()
                        .border_color(if custom { custom_color } else { gpui_kit::transparent_black() })
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            let ok = this.settings(cx).appearance.custom_accent.as_deref().and_then(parse_hex_color).is_some();
                            if ok {
                                this.edit_appearance(cx, |a| a.accent = Accent::Custom);
                            }
                        }))
                        .child(
                            div()
                                .size(sz(22.))
                                .rounded_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(if custom { custom_color } else { pal.surface_hover })
                                .border_1()
                                .border_color(pal.border_strong)
                                .child(icon("pipette").size(sz(12.)).text_color(if custom { pal.on_brand } else { pal.text_muted })),
                        ),
                )
                .child(div().w(sz(110.)).child(Input::new(&self.accent_input).small())),
        );

        let mut tones = h_flex().gap_3();
        for tone in Tone::ALL {
            let selected = a.tone == tone;
            let mut look = a.clone();
            look.tone = tone;
            let preview = tokens::Palette::themed(pal.dark, &look);
            tones = tones.child(
                v_flex()
                    .id(SharedString::from(format!("tone-{}", tone.label())))
                    .gap(sz(6.))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| this.edit_appearance(cx, |a| a.tone = tone)))
                    .child(
                        div()
                            .p(sz(3.))
                            .rounded(sz(tokens::radius::MD + 3.))
                            .border_2()
                            .border_color(if selected { pal.brand } else { gpui_kit::transparent_black() })
                            .child(
                                h_flex()
                                    .w(sz(84.))
                                    .h(sz(44.))
                                    .rounded(sz(tokens::radius::MD))
                                    .overflow_hidden()
                                    .border_1()
                                    .border_color(preview.border)
                                    .child(div().w(sz(22.)).h_full().bg(preview.chrome).border_r_1().border_color(preview.border))
                                    .child(
                                        v_flex()
                                            .flex_1()
                                            .h_full()
                                            .bg(preview.bg)
                                            .p(sz(6.))
                                            .gap(sz(4.))
                                            .child(div().h(sz(8.)).w(sz(30.)).rounded(sz(3.)).bg(preview.surface_hover))
                                            .child(div().h(sz(8.)).w(sz(20.)).rounded(sz(3.)).bg(preview.brand)),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .text_size(sz(tokens::text::SM))
                            .text_color(if selected { pal.text } else { pal.text_muted })
                            .when(selected, |d| d.font_weight(FontWeight::MEDIUM))
                            .child(tone.label()),
                    ),
            );
        }

        let side = |label: &'static str, active: bool, set: Side, cx: &mut Context<Self>| {
            segment(SharedString::from(format!("side-{label}")), label, active, &pal)
                .on_click(cx.listener(move |this, _, _, cx| this.edit_appearance(cx, |a| a.sidebar_side = set)))
        };
        let sides = segment_track(&pal)
            .child(side("Left", a.sidebar_side == Side::Left, Side::Left, cx))
            .child(side("Right", a.sidebar_side == Side::Right, Side::Right, cx));
        let row = |label: &'static str, body: AnyElement| {
            h_flex()
                .gap_3()
                .items_center()
                .child(div().w(sz(160.)).flex_none().text_size(sz(tokens::text::BASE)).child(label))
                .child(body)
        };
        v_flex()
            .gap_4()
            .child(row("Accent", accents.into_any_element()))
            .child(h_flex().gap_3().items_start().child(div().w(sz(160.)).flex_none().pt(sz(18.)).text_size(sz(tokens::text::BASE)).child("Tone")).child(tones))
            .child(row("Sidebar", sides.into_any_element()))
    }

    pub fn scroll_to_bottom(&self) {
        self.scroll.scroll_to_bottom();
    }

    fn settings(&self, cx: &App) -> Settings {
        self.store.read(cx).hub.settings()
    }

    fn update(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut Settings)) {
        let hub = self.store.read(cx).hub.clone();
        let mut s = hub.settings();
        let before = s.remote.clone();
        f(&mut s);
        let after = s.remote.clone();
        hub.update_settings(s);
        if before != after {
            if after.enabled {
                self.remote.start(after);
            } else if before.enabled {
                self.remote.stop();
            }
        }
        cx.notify();
    }

    fn add_root(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: true, prompt: Some("Add project folder".into()) });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                let _ = this.update(cx, |this, cx| {
                    this.update(cx, |s| {
                        for p in paths {
                            if !s.project_roots.contains(&p) {
                                s.project_roots.push(p);
                            }
                        }
                    })
                });
            }
        })
        .detach();
    }

    fn render_remote(&mut self, s: &Settings, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let status = self.remote.status();
        let busy = self.remote.is_busy();
        let pairing = self.remote.pairing_url();
        let r = s.remote.clone();
        let tunnels = self.tunnels.clone();

        let mut tunnel_row = h_flex().gap_2().flex_wrap();
        for kind in [TunnelKind::None, TunnelKind::Tailscale, TunnelKind::DevTunnel] {
            let available = tunnels.supports(kind);
            let selected = r.tunnel == kind;
            tunnel_row = tunnel_row.child(
                v_flex()
                    .id(ElementId::Name(format!("tunnel-{kind:?}").into()))
                    .w(sz(168.))
                    .p_3()
                    .gap_1()
                    .rounded(sz(10.))
                    .border_1()
                    .border_color(if selected { theme.primary } else { theme.border })
                    .when(selected, |d| d.bg(theme.primary.opacity(0.07)))
                    .when(!available, |d| d.opacity(0.5))
                    .when(available, |d| {
                        d.cursor_pointer().hover(|d| d.bg(theme.list_hover)).on_click(cx.listener(move |this, _, _, cx| {
                            this.update(cx, |s| {
                                s.remote.tunnel = kind;
                                if kind == TunnelKind::None {
                                    s.remote.bind = "0.0.0.0".into();
                                } else {
                                    s.remote.bind = "127.0.0.1".into();
                                }
                            })
                        }))
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(icon(match kind {
                                TunnelKind::None => "wifi",
                                TunnelKind::Tailscale => "network",
                                TunnelKind::DevTunnel => "link",
                            }).size(sz(15.)))
                            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(match kind {
                                TunnelKind::None => "Local network",
                                TunnelKind::Tailscale => "Tailscale",
                                TunnelKind::DevTunnel => "Dev Tunnels",
                            })),
                    )
                    .child(div().text_xs().text_color(theme.muted_foreground).child(match (kind, available) {
                        (_, false) => "Not installed".to_string(),
                        (TunnelKind::None, _) => "Same Wi-Fi / LAN only".into(),
                        (TunnelKind::Tailscale, _) => "Private to your tailnet".into(),
                        (TunnelKind::DevTunnel, _) => "Microsoft dev tunnel".into(),
                    })),
            );
        }

        // With Tailscale or a tunnel the local address is loopback: no use to a phone.
        let public_label = if r.tunnel == TunnelKind::Tailscale { "Tailnet" } else { "Public" };
        let urls = v_flex()
            .gap_2()
            .when_some(status.public_url.clone(), |d, u| d.child(url_row(public_label, u, cx)))
            .when_some(status.local_url.clone().filter(|_| r.tunnel == TunnelKind::None), |d, u| d.child(url_row("Local", u, cx)));

        let qr = pairing.as_ref().map(|p| qr_code(p, 3.4));
        let copy_link = pairing.clone();
        let tailscale_help = r.tunnel == TunnelKind::Tailscale
            && status.public_url.is_some()
            && status.warning.is_none()
            && status.connected_clients == 0;

        card(cx).p_5().child(
            v_flex()
                .gap_4()
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(
                            div()
                                .size(sz(36.))
                                .rounded(sz(10.))
                                .bg(theme.primary.opacity(0.12))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(icon("radio-tower").size(sz(18.)).text_color(theme.primary)),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .child(div().font_weight(FontWeight::SEMIBOLD).child("Mothr Remote"))
                                .child(div().text_sm().text_color(theme.muted_foreground).child(
                                    "Control every chat from your phone or another computer: answer questions, approve tools, start new chats.",
                                )),
                        )
                        .when(busy, |d| d.child(gpui_kit::component::spinner::Spinner::new().small()))
                        .child(Switch::new("remote-enabled").checked(r.enabled).on_change(cx.listener(|this, v: &bool, _, cx| {
                            let v = *v;
                            this.update(cx, |s| {
                                s.remote.enabled = v;
                                // LAN mode must listen on the network to be reachable from a phone.
                                if v && s.remote.tunnel == TunnelKind::None && s.remote.bind == "127.0.0.1" {
                                    s.remote.bind = "0.0.0.0".into();
                                }
                            })
                        }))),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(section_label("CONNECTION", cx))
                        .child(tunnel_row)
                        .when(tunnels.tailscale.is_none(), |d| {
                            d.child(div().text_xs().text_color(theme.muted_foreground).child(
                                "To reach this computer from anywhere, install Tailscale from tailscale.com/download.",
                            ))
                        }),
                )
                .when(status.running || status.error.is_some(), |d| {
                    d.child(
                        h_flex()
                            .gap_6()
                            .items_start()
                            .when_some(qr, |d, q| {
                                d.child(
                                    v_flex()
                                        .items_center()
                                        .gap_2()
                                        .child(q)
                                        .child(div().text_xs().text_color(theme.muted_foreground).child("Scan with the Mothr app"))
                                        .when_some(copy_link, |d, link| {
                                            d.child(Button::new("copy-pairing-link").ghost().small().icon(icon("copy")).label("Copy pairing link").on_click(
                                                move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(link.clone())),
                                            ))
                                        }),
                                )
                            })
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_3()
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .items_center()
                                            .child(div().size(sz(8.)).rounded_full().bg(if status.running { theme.success } else { theme.danger }))
                                            .child(div().text_sm().child(if status.running {
                                                format!(
                                                    "Running · {} device{} connected",
                                                    status.connected_clients,
                                                    if status.connected_clients == 1 { "" } else { "s" }
                                                )
                                            } else {
                                                "Not running".into()
                                            })),
                                    )
                                    .child(urls)
                                    .when_some(status.error.clone(), |d, e| {
                                        d.child(div().w_full().text_xs().text_color(theme.danger).child(e))
                                    })
                                    .when_some(status.warning.clone(), |d, w| {
                                        d.child(div().w_full().text_xs().text_color(theme.warning).child(w))
                                    })
                                    .when(tailscale_help, |d| {
                                        d.child(div().w_full().text_xs().text_color(theme.muted_foreground).child(
                                            "Keep Tailscale on on your phone, signed in to the same account. If it can't connect, check that your firewall allows this port on the Tailscale interface.",
                                        ))
                                    }),
                            ),
                    )
                })
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().text_sm().text_color(theme.muted_foreground).child("Pairing token"))
                        .child(
                            div()
                                .text_xs()
                                .font_family(theme.mono_font_family.clone())
                                .px_2()
                                .py_1()
                                .rounded(sz(6.))
                                .bg(theme.muted)
                                .child(if self.show_token { r.token.clone() } else { "•".repeat(24) }),
                        )
                        .child(Button::new("show-token").ghost().small().icon(icon(if self.show_token { "eye-off" } else { "eye" })).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.show_token = !this.show_token;
                                cx.notify();
                            },
                        )))
                        .child(
                            Button::new("regen-token")
                                .ghost()
                                .small()
                                .icon(icon("refresh-cw"))
                                .label("Regenerate")
                                .tooltip("Disconnects every paired device")
                                .on_click(cx.listener(|this, _, _, cx| this.update(cx, |s| s.remote.token = generate_token()))),
                        ),
                ),
        )
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let s = self.settings(cx);
        let store = self.store.read(cx);
        let agents = store.agents.clone();
        let available = store.available_agents();
        let project_count = store.project_count;
        let weak = cx.entity().downgrade();

        let look = self.render_look(&s, cx);
        // Appearance
        let mut themes = h_flex().gap_3();
        for (pref, label) in [(ThemePreference::System, "System"), (ThemePreference::Light, "Light"), (ThemePreference::Dark, "Dark")] {
            let selected = s.theme == pref;
            let pal_sel = tokens::p(cx);
            themes = themes.child(
                v_flex()
                    .id(label)
                    .gap_2()
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.update(cx, |s| s.theme = pref);
                        apply_preference(pref, window, cx);
                    }))
                    .child(
                        div()
                            .p(sz(3.))
                            .rounded(sz(tokens::radius::LG + 3.))
                            .border_2()
                            .border_color(if selected { pal_sel.brand } else { gpui_kit::transparent_black() })
                            .child(match pref {
                                ThemePreference::System => h_flex()
                                    .rounded(sz(tokens::radius::LG))
                                    .overflow_hidden()
                                    .child(theme_preview(&tokens::Palette::themed(false, &s.appearance), 76.))
                                    .child(theme_preview(&tokens::Palette::themed(true, &s.appearance), 76.))
                                    .into_any_element(),
                                ThemePreference::Light => div().rounded(sz(tokens::radius::LG)).overflow_hidden().child(theme_preview(&tokens::Palette::themed(false, &s.appearance), 152.)).into_any_element(),
                                ThemePreference::Dark => div().rounded(sz(tokens::radius::LG)).overflow_hidden().child(theme_preview(&tokens::Palette::themed(true, &s.appearance), 152.)).into_any_element(),
                            }),
                    )
                    .child(
                        h_flex()
                            .gap(sz(6.))
                            .items_center()
                            .justify_center()
                            .text_size(sz(tokens::text::SM))
                            .text_color(if selected { pal_sel.text } else { pal_sel.text_muted })
                            .when(selected, |d| d.font_weight(FontWeight::MEDIUM))
                            .child(label),
                    ),
            );
        }

        let mut sizes = h_flex()
            .p(sz(3.))
            .gap(sz(2.))
            .rounded(sz(tokens::radius::MD + 2.))
            .bg(tokens::p(cx).surface_hover)
            .border_1()
            .border_color(tokens::p(cx).border);
        for (scale, label) in [(1.0f32, "Compact"), (1.15, "Default"), (1.3, "Large"), (1.45, "Larger")] {
            let selected = (s.ui_scale - scale).abs() < 0.01;
            let pal = tokens::p(cx);
            sizes = sizes.child(
                div()
                    .id(SharedString::from(format!("scale-{label}")))
                    .px(sz(14.))
                    .h(sz(30.))
                    .flex()
                    .items_center()
                    .rounded(sz(tokens::radius::MD))
                    .text_size(sz(tokens::text::BASE))
                    .cursor_pointer()
                    .when(selected, |d| d.bg(pal.surface).text_color(pal.text).font_weight(FontWeight::MEDIUM))
                    .when(!selected, |d| d.text_color(pal.text_muted).hover(|d| d.text_color(pal.text)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.update(cx, |s| s.ui_scale = scale);
                        tokens::set_scale(scale);
                        crate::ui::theme::tune(cx);
                        window.refresh();
                    }))
                    .child(label),
            );
        }

        // Agents
        let mut agent_list = v_flex().gap_2();
        for a in &agents {
            let kind = a.kind;
            let disabled = s.agents.get(&kind).is_some_and(|o| o.disabled);
            agent_list = agent_list.child(
                h_flex()
                    .gap_3()
                    .p_3()
                    .rounded(sz(10.))
                    .border_1()
                    .border_color(theme.border)
                    .items_center()
                    .child(agent_avatar(kind, 34.))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(div().font_weight(FontWeight::SEMIBOLD).child(kind.display_name()))
                                    .when_some(a.version.clone(), |d, v| d.child(div().text_xs().text_color(theme.muted_foreground).child(format!("v{v}"))))
                                    .child(
                                        div()
                                            .text_xs()
                                            .px(sz(6.))
                                            .py(sz(1.))
                                            .rounded(sz(4.))
                                            .bg(if a.available() { theme.success.opacity(0.12) } else { theme.muted })
                                            .text_color(if a.available() { theme.success } else { theme.muted_foreground })
                                            .child(if a.available() { "Ready" } else if disabled { "Disabled" } else { "Not found" }),
                                    ),
                            )
                            .child(div().text_xs().text_color(theme.muted_foreground).truncate().child(match (&a.transport, &a.binary) {
                                (Some(Transport::ClaudeStreamJson { program }), _) => format!("{} · {}", "Claude SDK protocol", program.display()),
                                (Some(Transport::Acp { program, args, .. }), _) => format!("ACP · {} {}", program.display(), args.join(" ")),
                                (None, Some(b)) => b.display().to_string(),
                                (None, None) => "Install the CLI to enable".into(),
                            }))
                            .when_some(a.note.clone(), |d, n| d.child(div().text_xs().text_color(theme.muted_foreground.opacity(0.8)).child(n))),
                    )
                    .child(Switch::new(ElementId::Name(format!("agent-on-{}", kind.id()).into())).checked(!disabled).on_change(cx.listener(
                        move |this, v: &bool, _, cx| {
                            let on = *v;
                            this.update(cx, |s| s.agents.entry(kind).or_default().disabled = !on);
                        },
                    ))),
            );
        }

        // Projects
        let mut roots = v_flex().gap_1();
        for (i, r) in s.project_roots.iter().enumerate() {
            roots = roots.child(
                h_flex()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .rounded(sz(8.))
                    .bg(theme.muted.opacity(0.5))
                    .items_center()
                    .child(icon("folder").size(sz(14.)).text_color(theme.muted_foreground))
                    .child(div().flex_1().text_sm().child(tildify(r)))
                    .child(Button::new(ElementId::Name(format!("rm-root-{i}").into())).ghost().small().icon(icon("x")).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.update(cx, |s| {
                                if i < s.project_roots.len() {
                                    s.project_roots.remove(i);
                                }
                            })
                        },
                    ))),
            );
        }

        let remote = self.render_remote(&s, cx);
        let store_e = self.store.clone();

        div().id("settings-scroll").size_full().overflow_y_scroll().track_scroll(&self.scroll).child(
            v_flex()
                .w_full()
                .max_w(sz(820.))
                .mx_auto()
                .px_8()
                .py_8()
                .gap_8()
                .child(div().text_size(sz(tokens::text::XXL)).font_weight(FontWeight::SEMIBOLD).child("Settings"))
                .child(group(
                    "Appearance",
                    "Theme follows your system unless you pick one.",
                    v_flex()
                        .gap_4()
                        .child(themes)
                        .child(
                            h_flex()
                                .gap_3()
                                .items_center()
                                .child(div().w(sz(160.)).text_size(sz(tokens::text::BASE)).child("Interface size"))
                                .child(sizes),
                        ),
                    cx,
                ))
                .child(group("Colors and layout", "Make Mothr yours: its accent, its tone, and where the sidebar sits.", look, cx))
                .child(group(
                    "Agents",
                    "Mothr talks to each agent over its native protocol — no terminal UI involved.",
                    v_flex().gap_2().child(agent_list).child(
                        h_flex().child(
                            Button::new("rescan-agents")
                                .small()
                                .outline()
                                .icon(icon("refresh-cw"))
                                .label("Rescan")
                                .on_click(move |_, window, cx| {
                                    store_e.update(cx, |s, cx| {
                                        s.dispatch(AppCommand::RescanAgents, window, cx);
                                    });
                                }),
                        ),
                    ),
                    cx,
                ))
                .child(group(
                    "Defaults",
                    "Applied to new chats; change per chat any time.",
                    v_flex()
                        .gap_3()
                        .child(
                            h_flex()
                                .gap_3()
                                .items_center()
                                .child(
                                    v_flex()
                                        .w(sz(220.))
                                        .child(div().text_sm().child("Permission mode"))
                                        .child(div().text_xs().text_color(theme.muted_foreground).child("For every agent; unsupported modes fall back")),
                                )
                                .child(mode_menu("default-mode", None, s.default_mode, None, weak.clone(), |this: &mut SettingsView, m, _, cx| {
                                    this.update(cx, |s| s.default_mode = m)
                                })),
                        )
                        .child(
                            h_flex()
                                .gap_3()
                                .items_center()
                                .child(
                                    v_flex()
                                        .w(sz(220.))
                                        .child(div().text_sm().child("Remember last agent & model"))
                                        .child(div().text_xs().text_color(theme.muted_foreground).child("New chats start with what you used last")),
                                )
                                .child(Switch::new("remember-sel").checked(s.remember_last_selection).on_change(cx.listener(|this, v: &bool, _, cx| {
                                    let v = *v;
                                    this.update(cx, |s| s.remember_last_selection = v)
                                }))),
                        )
                        .when(!s.remember_last_selection, |d| {
                            let mut row = h_flex().gap_2().flex_wrap();
                            for k in available.iter().copied() {
                                let sel = s.default_agent == Some(k);
                                row = row.child(
                                    Button::new(ElementId::Name(format!("def-agent-{}", k.id()).into()))
                                        .small()
                                        .icon(agent_logo(k))
                                        .label(k.short_name())
                                        .map(|b| if sel { b.primary() } else { b.outline() })
                                        .on_click(cx.listener(move |this, _, _, cx| this.update(cx, |s| s.default_agent = Some(k)))),
                                );
                            }
                            d.child(
                                h_flex()
                                    .gap_3()
                                    .items_center()
                                    .child(div().w(sz(220.)).text_sm().child("Default agent"))
                                    .child(row),
                            )
                        })
                        .child(
                            h_flex()
                                .gap_3()
                                .items_center()
                                .child(div().w(sz(220.)).text_sm().child("Notify when a chat needs me"))
                                .child(Switch::new("notify-needs").checked(s.notify_needs_input).on_change(cx.listener(|this, v: &bool, _, cx| {
                                    let v = *v;
                                    this.update(cx, |s| s.notify_needs_input = v)
                                }))),
                        )
                        .child(
                            h_flex()
                                .gap_3()
                                .items_center()
                                .child(div().w(sz(220.)).text_sm().child("Show agent reasoning"))
                                .child(Switch::new("show-thinking").checked(s.show_thinking).on_change(cx.listener(|this, v: &bool, _, cx| {
                                    let v = *v;
                                    this.update(cx, |s| s.show_thinking = v)
                                })))
                                .child(div().text_xs().text_color(tokens::p(cx).text_faint).child("The collapsed “Thought” lines in a chat")),
                        )
                        .child(
                            h_flex()
                                .gap_3()
                                .items_center()
                                .child(div().w(sz(220.)).text_sm().child("Warn at limit usage"))
                                .children([0.7f32, 0.8, 0.9].map(|t| {
                                    let sel = (s.warn_threshold - t).abs() < 0.01;
                                    Button::new(ElementId::Name(format!("warn-{t}").into()))
                                        .small()
                                        .label(format!("{:.0}%", t * 100.0))
                                        .map(|b| if sel { b.primary() } else { b.outline() })
                                        .on_click(cx.listener(move |this, _, _, cx| this.update(cx, |s| s.warn_threshold = t)))
                                })),
                        ),
                    cx,
                ))
                .child(group(
                    "Projects",
                    match project_count {
                        Some(n) => format!("{n} projects found. Mothr scans these folders for repositories and seeds from agent history."),
                        None => "Scanning…".into(),
                    },
                    v_flex().gap_2().child(roots).child(
                        h_flex().gap_2().child(
                            Button::new("add-root")
                                .small()
                                .outline()
                                .icon(icon("plus"))
                                .label("Add folder")
                                .on_click(cx.listener(|this, _, _, cx| this.add_root(cx))),
                        ),
                    ),
                    cx,
                ))
                .child(remote),
        )
    }
}

fn group(title: &'static str, desc: impl Into<SharedString>, body: impl IntoElement, cx: &App) -> impl IntoElement {
    v_flex()
        .gap_3()
        .child(
            v_flex()
                .child(div().text_size(sz(tokens::text::LG)).font_weight(FontWeight::SEMIBOLD).child(title))
                .child(div().text_size(sz(tokens::text::SM)).text_color(cx.theme().muted_foreground).child(desc.into())),
        )
        .child(body)
}

fn url_row(label: &'static str, url: String, cx: &App) -> impl IntoElement + use<> {
    let theme = cx.theme();
    let u2 = url.clone();
    let u3 = url.clone();
    h_flex()
        .gap_2()
        .items_center()
        .child(div().w(sz(56.)).text_xs().text_color(theme.muted_foreground).child(label))
        .child(div().flex_1().text_sm().font_family(theme.mono_font_family.clone()).truncate().child(url))
        .child(Button::new(ElementId::Name(format!("copy-{label}").into())).ghost().small().icon(icon("copy")).tooltip("Copy").on_click(
            move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(u2.clone())),
        ))
        .child(Button::new(ElementId::Name(format!("open-{label}").into())).ghost().small().icon(icon("external-link")).on_click(
            move |_, _, cx| cx.open_url(&u3),
        ))
}

/// Draw a QR code natively from its module matrix.
fn qr_code(data: &str, module: f32) -> impl IntoElement + use<> {
    let m = qr_matrix(data);
    let mut col = v_flex();
    for row in &m {
        let mut r = h_flex();
        for &dark in row {
            r = r.child(div().size(sz(module)).bg(if dark { gpui_kit::black() } else { gpui_kit::white() }));
        }
        col = col.child(r);
    }
    div().p_3().rounded(sz(10.)).bg(gpui_kit::white()).child(col)
}

/// A miniature of the workspace in a given palette (sidebar, bubble, composer).
fn theme_preview(pal: &tokens::Palette, w: f32) -> impl IntoElement {
    let h = 96.;
    h_flex()
        .w(sz(w))
        .h(sz(h))
        .bg(pal.bg)
        .overflow_hidden()
        .child(
            v_flex()
                .w(sz(38.))
                .flex_none()
                .h_full()
                .bg(pal.chrome)
                .border_r_1()
                .border_color(pal.border)
                .p(sz(6.))
                .gap(sz(5.))
                .child(div().h(sz(8.)).rounded(sz(3.)).bg(pal.brand))
                .child(div().h(sz(5.)).w(sz(22.)).rounded(sz(2.)).bg(pal.surface_hover))
                .child(div().h(sz(5.)).w(sz(18.)).rounded(sz(2.)).bg(pal.surface_hover))
                .child(div().h(sz(5.)).w(sz(24.)).rounded(sz(2.)).bg(pal.surface_hover)),
        )
        .child(
            v_flex()
                .flex_1()
                .p(sz(8.))
                .gap(sz(6.))
                .child(h_flex().justify_end().child(div().h(sz(10.)).w(sz(48.)).rounded(sz(5.)).bg(pal.surface_hover)))
                .child(div().h(sz(4.)).w(relative(0.9)).rounded(sz(2.)).bg(pal.text_faint))
                .child(div().h(sz(4.)).w(relative(0.7)).rounded(sz(2.)).bg(pal.text_faint))
                .child(div().flex_1())
                .child(
                    h_flex()
                        .h(sz(18.))
                        .px(sz(5.))
                        .rounded(sz(5.))
                        .border_1()
                        .border_color(pal.border_strong)
                        .bg(pal.surface)
                        .justify_end()
                        .items_center()
                        .child(div().size(sz(9.)).rounded(sz(3.)).bg(pal.brand)),
                ),
        )
}
