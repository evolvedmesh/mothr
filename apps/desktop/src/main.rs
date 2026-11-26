//! Mothr — a native manager for coding-agent chats.

mod assets;
mod remote;
mod store;
mod ui;

use gpui_kit::component::TitleBar;
use gpui_kit::*;
use mothr_core::Settings;
use std::sync::Arc;

actions!(
    mothr,
    [NewChat, NewDesign, NewGeneralChat, ResumeChat, ToggleTerminal, OpenProjectPalette, OpenProjects, OpenWorkspaces, NextWorkspace, PrevWorkspace, ToggleSidebar, OpenSettings, FocusComposer, GoBack, GoForward, Quit]
);

/// Switch to the workspace at this position (0-based), like a browser's tab shortcuts.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = mothr, no_json)]
pub struct SwitchWorkspace(pub usize);

#[cfg(target_os = "macos")]
macro_rules! k {
    ($k:literal) => {
        concat!("cmd-", $k)
    };
}
#[cfg(not(target_os = "macos"))]
macro_rules! k {
    ($k:literal) => {
        concat!("ctrl-", $k)
    };
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "mothr=info,warn".into()),
        )
        .init();

    mothr_core::config::migrate_legacy_dirs();
    let mut settings = Settings::load();
    if settings.remote.token.is_empty() {
        settings.remote.token = remote::generate_token();
        let _ = settings.save();
    }

    // Agents, networking and file IO run on tokio; GPUI owns the main thread.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_name("mothr-rt")
        .enable_all()
        .build()
        .expect("tokio runtime");
    let hub = mothr_hub::Hub::start(runtime.handle().clone(), settings.clone());
    let rt_handle = runtime.handle().clone();
    // Save chats if the OS asks us to stop (logout, `kill`, Ctrl+C in a terminal).
    {
        let hub = hub.clone();
        runtime.spawn(async move {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{SignalKind, signal};
                let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else { return };
                tokio::select! { _ = term.recv() => {}, _ = int.recv() => {} }
            }
            #[cfg(not(unix))]
            let _ = tokio::signal::ctrl_c().await;
            hub.shutdown();
            std::process::exit(0);
        });
    }

    gpui_kit::application()
        .with_assets(assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            cx.bind_keys([
                KeyBinding::new(k!("n"), NewChat, None),
                KeyBinding::new(k!("shift-n"), NewDesign, None),
                KeyBinding::new(k!("shift-g"), NewGeneralChat, None),
                KeyBinding::new(k!("shift-r"), ResumeChat, None),
                // The terminal, as in VS Code: Ctrl+` (Ctrl+~ too).
                KeyBinding::new("ctrl-`", ToggleTerminal, None),
                KeyBinding::new("ctrl-~", ToggleTerminal, None),
                KeyBinding::new("ctrl-shift-`", ToggleTerminal, None),
                KeyBinding::new(k!("p"), OpenProjectPalette, None),
                KeyBinding::new(k!("k"), OpenProjectPalette, None),
                KeyBinding::new(k!("shift-p"), OpenProjects, None),
                KeyBinding::new(k!("b"), ToggleSidebar, None),
                KeyBinding::new(k!(","), OpenSettings, None),
                KeyBinding::new(k!("l"), FocusComposer, None),
                KeyBinding::new(k!("q"), Quit, None),
                // Workspaces: step through them, or jump straight to one by its number.
                KeyBinding::new(k!("alt-right"), NextWorkspace, None),
                KeyBinding::new(k!("alt-left"), PrevWorkspace, None),
                KeyBinding::new(k!("shift-w"), OpenWorkspaces, None),
                KeyBinding::new(k!("1"), SwitchWorkspace(0), None),
                KeyBinding::new(k!("2"), SwitchWorkspace(1), None),
                KeyBinding::new(k!("3"), SwitchWorkspace(2), None),
                KeyBinding::new(k!("4"), SwitchWorkspace(3), None),
                KeyBinding::new(k!("5"), SwitchWorkspace(4), None),
                KeyBinding::new(k!("6"), SwitchWorkspace(5), None),
                KeyBinding::new(k!("7"), SwitchWorkspace(6), None),
                KeyBinding::new(k!("8"), SwitchWorkspace(7), None),
                KeyBinding::new(k!("9"), SwitchWorkspace(8), None),
                KeyBinding::new(k!("0"), SwitchWorkspace(9), None),
                // Back/forward through the places visited (the mouse's side buttons do the same).
                #[cfg(target_os = "macos")]
                KeyBinding::new("cmd-[", GoBack, None),
                #[cfg(target_os = "macos")]
                KeyBinding::new("cmd-]", GoForward, None),
                #[cfg(not(target_os = "macos"))]
                KeyBinding::new("alt-left", GoBack, None),
                #[cfg(not(target_os = "macos"))]
                KeyBinding::new("alt-right", GoForward, None),
            ]);
            cx.on_action(|_: &Quit, cx| cx.quit());

            ui::tokens::set_scale(settings.ui_scale);
            ui::tokens::set_appearance(settings.appearance.clone());
            ui::theme::load_fonts(cx);
            ui::theme::apply_mode(ui::theme::resolve(settings.theme, None), None, cx);
            ui::init(cx);

            let remote = remote::RemoteController::new(rt_handle.clone(), hub.clone());
            let hub_for_quit = hub.clone();
            cx.on_app_quit(move |_| {
                hub_for_quit.shutdown();
                async {}
            })
            .detach();

            let mut size = size(px(1360.), px(880.));
            if let Some(d) = cx.primary_display() {
                size.width = size.width.min(d.bounds().size.width * 0.9);
                size.height = size.height.min(d.bounds().size.height * 0.9);
            }
            let bounds = Bounds::centered(None, size, cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(gpui_kit::size(px(760.), px(520.))),
                app_id: Some("dev.mothr.Mothr".into()),
                #[cfg(target_os = "linux")]
                window_decorations: Some(WindowDecorations::Client),
                ..TitleBar::window_options()
            };
            let hub2: Arc<mothr_hub::Hub> = hub.clone();
            gpui_kit::open_window(options, cx, move |window, cx| {
                window.set_window_title("Mothr");
                cx.new(|cx| ui::workspace::Workspace::new(hub2, remote, window, cx))
            })
            .expect("failed to open the Mothr window");
            cx.activate(true);
        });
    drop(runtime);
}
