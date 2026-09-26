//! Hi-Fi — a GPUI browser. `gpui` shell + wry(WKWebView) content +
//! `alacritty_terminal` panes, on the exact stack cosmos runs.

mod assets;
mod command_bar;
mod frost;
mod ipc_server;
mod jsbridge;
mod notes;
mod shell;
mod sidebar;
mod store;
mod surface_chrome;
mod terminal;
mod text_input;
mod theme;
mod views;
mod webview;

use std::borrow::Cow;
use std::sync::mpsc::channel;

use gpui::{
    App, AppContext as _, Bounds, KeyBinding, Menu, MenuItem, TitlebarOptions, WindowAppearance,
    WindowBounds, WindowOptions, actions, px, size,
};
use shell::{IpcJob, Shell};
use store::Store;
use theme::Theme;

actions!(
    hifi,
    [
        NewTab,
        CloseTab,
        CommandBar,
        FocusAddress,
        ReloadPage,
        GoBack,
        GoForward,
        NextTab,
        PrevTab,
        ToggleSidebar,
        ToggleDock,
        FindInPage,
        NewTerminal,
        NewAgent,
        NewDiff,
        NewNotes,
        OpenSettings,
        Quit,
        Hide,
        HideOthers,
        Minimize,
        CloseWindow,
        NewGroup,
        Copy,
        Cut,
        Paste,
        SelectAll,
        Undo,
        Redo,
    ]
);

/// Resolve a typed launcher string to a URL (search-engine aware).
fn resolve_for_open(store: &Store, text: &str) -> Option<String> {
    match hifi_core::route(text) {
        hifi_core::RoutedUrl::Internal(_) | hifi_core::RoutedUrl::Preview(_) => {
            Some(text.to_string())
        }
        hifi_core::RoutedUrl::External(u) => {
            if let Some(q) = u.strip_prefix("search:") {
                Some(hifi_core::schemes::resolve_search(
                    &store.state.settings.search_engine,
                    q,
                ))
            } else {
                Some(u)
            }
        }
    }
}

fn register_fonts(cx: &App) {
    use gpui::AssetSource;
    let fonts: Vec<Cow<'static, [u8]>> = assets::FONTS
        .iter()
        .filter_map(|p| assets::Assets.load(p).ok().flatten())
        .collect();
    let _ = cx.text_system().add_fonts(fonts);
}

fn install_menus(cx: &mut App) {
    cx.set_menus(vec![
        Menu::new("Hi-Fi").items([
            MenuItem::separator(),
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::separator(),
            MenuItem::action("Hide Hi-Fi", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::separator(),
            MenuItem::action("Quit Hi-Fi", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Tab", NewTab),
            MenuItem::action("New Terminal", NewTerminal),
            MenuItem::action("New Agent", NewAgent),
            MenuItem::separator(),
            MenuItem::action("Close Tab", CloseTab),
        ]),
        Menu::new("Edit").items([
            MenuItem::action("Undo", Undo),
            MenuItem::action("Redo", Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", Cut, gpui::OsAction::Cut),
            MenuItem::os_action("Copy", Copy, gpui::OsAction::Copy),
            MenuItem::os_action("Paste", Paste, gpui::OsAction::Paste),
            MenuItem::os_action("Select All", SelectAll, gpui::OsAction::SelectAll),
        ]),
        Menu::new("View").items([
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
            MenuItem::action("Toggle Dock", ToggleDock),
            MenuItem::action("New Page", NewNotes),
            MenuItem::action("Reload", ReloadPage),
            MenuItem::action("Command Bar", CommandBar),
        ]),
        Menu::new("History").items([
            MenuItem::action("Back", GoBack),
            MenuItem::action("Forward", GoForward),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", Minimize),
            MenuItem::action("Close Window", CloseWindow),
        ]),
    ]);
}

fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-t", CommandBar, None),
        KeyBinding::new("secondary-l", FocusAddress, None),
        KeyBinding::new("secondary-w", CloseTab, None),
        KeyBinding::new("secondary-r", ReloadPage, None),
        KeyBinding::new("secondary-shift-r", ReloadPage, None),
        KeyBinding::new("secondary-left", GoBack, None),
        KeyBinding::new("secondary-right", GoForward, None),
        KeyBinding::new("secondary-[", GoBack, None),
        KeyBinding::new("secondary-]", GoForward, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("secondary-shift-]", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PrevTab, None),
        KeyBinding::new("secondary-shift-[", PrevTab, None),
        KeyBinding::new("secondary-b", ToggleSidebar, None),
        KeyBinding::new("alt-secondary-b", ToggleDock, None),
        KeyBinding::new("secondary-shift-n", NewNotes, None),
        KeyBinding::new("secondary-f", FindInPage, None),
        KeyBinding::new("secondary-,", OpenSettings, None),
        KeyBinding::new("secondary-shift-\\", NewGroup, None),
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
        // TextField editing keys, scoped to focused text fields.
        KeyBinding::new("enter", text_input::Submit, Some("TextField")),
        KeyBinding::new("escape", text_input::Escape, Some("TextField")),
        KeyBinding::new("backspace", text_input::Backspace, Some("TextField")),
        KeyBinding::new("delete", text_input::Delete, Some("TextField")),
        KeyBinding::new("left", text_input::Left, Some("TextField")),
        KeyBinding::new("right", text_input::Right, Some("TextField")),
        KeyBinding::new("shift-left", text_input::SelectLeft, Some("TextField")),
        KeyBinding::new("shift-right", text_input::SelectRight, Some("TextField")),
        KeyBinding::new("secondary-a", text_input::SelectAll, Some("TextField")),
        KeyBinding::new("secondary-x", text_input::Cut, Some("TextField")),
        KeyBinding::new("secondary-c", text_input::Copy, Some("TextField")),
        KeyBinding::new("secondary-v", text_input::Paste, Some("TextField")),
        KeyBinding::new("home", text_input::Home, Some("TextField")),
        KeyBinding::new("end", text_input::End, Some("TextField")),
    ]);
}

fn main() {
    let app = gpui_platform::application().with_assets(assets::Assets);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("tokio runtime");
    let handle = runtime.handle().clone();

    let (ipc_tx, ipc_rx) = channel::<IpcJob>();

    app.run(move |cx| {
        gpui_tokio::init_from_handle(cx, handle.clone());
        register_fonts(cx);

        let store = Store::load(cx);
        let dark = matches!(
            cx.window_appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        Theme::install(&store.read(cx).state.settings.clone(), dark, cx);
        let mut applied = {
            let s = &store.read(cx).state.settings;
            (s.appearance, s.accent.clone())
        };
        cx.observe(&store, move |store, cx| {
            let settings = store.read(cx).state.settings.clone();
            let next = (settings.appearance, settings.accent.clone());
            if next == applied {
                return;
            }
            applied = next;
            let dark = matches!(
                cx.window_appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            );
            Theme::install(&settings, dark, cx);
            cx.refresh_windows();
        })
        .detach();

        bind_keys(cx);
        install_menus(cx);

        // Global app actions.
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &Hide, cx| cx.hide());
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        cx.on_action(|_: &Minimize, cx| {
            if let Some(w) = cx.active_window() {
                let _ = w.update(cx, |_, window, _| window.minimize_window());
            }
        });
        cx.on_action(|_: &CloseWindow, cx| {
            if let Some(w) = cx.active_window() {
                let _ = w.update(cx, |_, window, _| window.remove_window());
            }
        });

        // IPC socket for the `hifi` CLI.
        let paths = store.read(cx).paths.clone();
        ipc_server::serve(paths.socket_file(), ipc_tx.clone());

        let bounds = Bounds::centered(None, size(px(1280.), px(840.)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(720.), px(480.))),
                    titlebar: Some(TitlebarOptions {
                        title: cfg!(target_os = "windows").then(|| "Hi-Fi".into()),
                        appears_transparent: true,
                        traffic_light_position: Some(gpui::point(px(14.), px(14.))),
                    }),
                    app_owns_titlebar_drag: true,
                    // Frosted shell — sidebar/new-tab translucency reads the
                    // wallpaper behind the window like cosmos.
                    window_background: gpui::WindowBackgroundAppearance::Blurred,
                    app_id: Some("hifi".into()),
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| Shell::new(store.clone(), ipc_rx, window, cx)),
            )
            .expect("open window");
        let _ = window;

        // First-run content: a new-tab page if nothing was restored.
        store.update(cx, |s, cx| {
            if s.state.tabs.is_empty() {
                s.open_tab("hifi://newtab", None, None, cx);
            }
        });
    });
}
