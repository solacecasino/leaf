//! The menu bar: Leaf, File, View, Go, Adapters (only when some are
//! installed), Window (macOS) and Help, at the top of the screen on macOS
//! and of the window on Windows. It's built from the key bindings, so it
//! shows the user's own shortcuts, and every item is a named command the app
//! runs exactly like a key press. It's rebuilt when the book or the adapters
//! change; the View checkmarks are updated in place.
//!
//! Linux gets no menu bar yet (it would need GTK); every command has a key.

use crate::adapters::{Adapter, command_name};
use crate::keys::Keys;
use crate::state::Mode;
use std::path::PathBuf;

/// What the menus show besides the fixed commands.
pub struct Content<'a> {
    pub keys: &'a Keys,
    pub adapters: &'a [Adapter],
    pub recent: &'a [PathBuf],
    pub view: View,
}

/// The open book's settings, for the View menu's checkmarks.
#[derive(Default, Clone, Copy)]
pub struct View {
    pub has_book: bool,
    pub mode: Option<Mode>,
    pub spreads: bool,
    pub pair_cover: bool,
    pub rtl: bool,
    pub fit_width: bool,
    pub magnifier: bool,
    /// Upscaling, when an upscaler adapter is installed: on or off.
    pub upscale: Option<bool>,
}

/// The menu text for a built-in command.
fn label(command: &str) -> &'static str {
    match command {
        "open-file" => "Open…",
        "open-folder" => "Open Folder…",
        "close-book" => "Close Book",
        "mode-comic" => "Comic",
        "mode-manga" => "Manga",
        "mode-webtoon" => "Webtoon",
        "toggle-spreads" => "Two Pages",
        "pair-cover" => "Pair the Cover",
        "toggle-fit" => "Fit Width",
        "toggle-rtl" => "Right to Left",
        "fullscreen" => "Toggle Full Screen",
        "next" => "Next Page",
        "prev" => "Previous Page",
        "page-right" => "Page to the Right",
        "page-left" => "Page to the Left",
        "first" => "First Page",
        "last" => "Last Page",
        "open-config-folder" => "Open Config Folder",
        "edit-keys" => "Edit Key Bindings",
        "edit-settings" => "Edit Settings",
        "toggle-magnifier" => "Magnifier",
        "toggle-upscale" => "Upscale Pages",
        "go-to-page" => "Go to Page…",
        "next-book" => "Next Book in Folder",
        "prev-book" => "Previous Book in Folder",
        "show-adapters-folder" => "Show Adapters Folder",
        "reload-adapters" => "Reload Adapters",
        "show-log" => "Show Log",
        "copy-debug-info" => "Copy Debug Info",
        "help" => "Leaf Help",
        "quit" => "Quit Leaf",
        _ => "",
    }
}

#[cfg(any(target_os = "macos", windows))]
pub use bar::MenuBar;

#[cfg(any(target_os = "macos", windows))]
mod bar {
    use super::{Content, Mode, View, command_name, label};
    use muda::accelerator::Accelerator;
    use muda::{AboutMetadataBuilder, CheckMenuItem, Icon, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
    use std::sync::OnceLock;

    pub struct MenuBar {
        _menu: Menu,
        /// Menu item ids are indexes into this list.
        commands: Vec<String>,
        /// The View menu's checkmarks, by command.
        checks: Vec<(String, CheckMenuItem)>,
    }

    static HANDLER: OnceLock<()> = OnceLock::new();

    struct Builder<'a> {
        content: &'a Content<'a>,
        commands: Vec<String>,
    }

    impl Builder<'_> {
        fn item(&mut self, command: &str, text: &str, enabled: bool) -> MenuItem {
            let accel = self
                .content
                .keys
                .menu_key(command)
                .and_then(crate::keys::accelerator)
                .and_then(|a| a.parse::<Accelerator>().ok());
            let id = self.commands.len().to_string();
            self.commands.push(command.to_owned());
            MenuItem::with_id(id, text, enabled, accel)
        }

        fn builtin(&mut self, command: &str, enabled: bool) -> MenuItem {
            self.item(command, label(command), enabled)
        }

        fn check(&mut self, command: &str, checked: bool, enabled: bool) -> CheckMenuItem {
            let accel = self
                .content
                .keys
                .menu_key(command)
                .and_then(crate::keys::accelerator)
                .and_then(|a| a.parse::<Accelerator>().ok());
            let id = self.commands.len().to_string();
            self.commands.push(command.to_owned());
            CheckMenuItem::with_id(id, label(command), enabled, checked, accel)
        }

        /// Adapter commands that asked for a place in `menu`.
        fn adapter_items(&mut self, menu: &Submenu, which: &str) {
            let mut first = true;
            for a in self.content.adapters {
                for c in a.manifest.commands.iter().filter(|c| c.menu.as_deref() == Some(which)) {
                    if first {
                        menu.append(&PredefinedMenuItem::separator()).expect("menu builds");
                        first = false;
                    }
                    let item = self.item(&command_name(&a.manifest.name, &c.id), &c.label, true);
                    menu.append(&item).expect("menu builds");
                }
            }
        }
    }

    impl MenuBar {
        /// Builds the menus and makes them the app's menu bar. `send` gets the
        /// command of each item chosen; it's called on the main thread.
        pub fn install(content: &Content<'_>, window: &winit::window::Window, send: impl Fn(String) + Send + Sync + 'static) -> Self {
            let mut b = Builder { content, commands: Vec::new() };
            let book = content.view.has_book;
            let menu = Menu::new();
            let sep = PredefinedMenuItem::separator;

            let app = Submenu::new("Leaf", true);
            let config = [
                b.builtin("open-config-folder", true),
                b.builtin("edit-keys", true),
                b.builtin("edit-settings", true),
                b.builtin("show-adapters-folder", true),
                b.builtin("reload-adapters", true),
            ];
            #[cfg(target_os = "macos")]
            {
                app.append_items(&[&PredefinedMenuItem::about(Some("About Leaf"), Some(about())), &sep()]).expect("menu builds");
                for item in &config {
                    app.append(item).expect("menu builds");
                }
                app.append_items(&[
                    &sep(),
                    &PredefinedMenuItem::hide(None),
                    &PredefinedMenuItem::hide_others(None),
                    &PredefinedMenuItem::show_all(None),
                    &sep(),
                    &b.builtin("quit", true),
                ])
                .expect("menu builds");
            }
            #[cfg(windows)]
            for item in &config {
                app.append(item).expect("menu builds");
            }

            let file = Submenu::new("File", true);
            let recent = Submenu::new("Open Recent", !content.recent.is_empty());
            for (i, path) in content.recent.iter().enumerate() {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                recent.append(&b.item(&format!("open-recent {i}"), &name, true)).expect("menu builds");
            }
            file.append_items(&[&b.builtin("open-file", true), &b.builtin("open-folder", true), &recent, &sep(), &b.builtin("close-book", book)])
                .expect("menu builds");
            b.adapter_items(&file, "file");
            // Windows keeps Exit at the bottom of File.
            #[cfg(windows)]
            file.append_items(&[&sep(), &b.item("quit", "Exit", true)]).expect("menu builds");

            let view = Submenu::new("View", true);
            let checks: Vec<(String, CheckMenuItem)> =
                CHECKS.iter().map(|c| (c.to_string(), b.check(c, checked(&content.view, c), enabled(&content.view, c)))).collect();
            let item = |c: &str| &checks.iter().find(|(name, _)| name == c).expect("listed").1;
            view.append_items(&[
                item("mode-comic"),
                item("mode-manga"),
                item("mode-webtoon"),
                &sep(),
                item("toggle-spreads"),
                item("pair-cover"),
                item("toggle-rtl"),
                item("toggle-fit"),
                item("toggle-magnifier"),
                item("toggle-upscale"),
                &sep(),
                &b.builtin("fullscreen", true),
            ])
            .expect("menu builds");
            b.adapter_items(&view, "view");

            let go = Submenu::new("Go", true);
            go.append_items(&[
                &b.builtin("next", book),
                &b.builtin("prev", book),
                &sep(),
                &b.builtin("first", book),
                &b.builtin("last", book),
                &b.builtin("go-to-page", book),
                &sep(),
                &b.builtin("next-book", book),
                &b.builtin("prev-book", book),
            ])
            .expect("menu builds");
            b.adapter_items(&go, "go");

            menu.append_items(&[&app, &file, &view, &go]).expect("menu builds");

            // The Adapters menu appears only once something is installed.
            let with_commands: Vec<_> = content.adapters.iter().collect();
            if !with_commands.is_empty() {
                let adapters = Submenu::new("Adapters", true);
                for a in with_commands {
                    let sub = Submenu::new(&a.manifest.name, true);
                    for c in a.manifest.commands.iter().filter(|c| c.menu.is_none()) {
                        sub.append(&b.item(&command_name(&a.manifest.name, &c.id), &c.label, true)).expect("menu builds");
                    }
                    let remove = format!("Remove {}…", a.manifest.name);
                    sub.append_items(&[&sep(), &b.item(&format!("remove-adapter {}", a.manifest.name), &remove, true)]).expect("menu builds");
                    adapters.append(&sub).expect("menu builds");
                }
                menu.append(&adapters).expect("menu builds");
            }

            let help = Submenu::new("Help", true);
            help.append_items(&[&b.builtin("help", true), &b.builtin("show-log", true), &b.builtin("copy-debug-info", true)]).expect("menu builds");

            #[cfg(target_os = "macos")]
            {
                let _ = window;
                let windows = Submenu::new("Window", true);
                windows
                    .append_items(&[&PredefinedMenuItem::minimize(None), &PredefinedMenuItem::maximize(None)])
                    .expect("menu builds");
                menu.append_items(&[&windows, &help]).expect("menu builds");
                menu.init_for_nsapp();
                windows.set_as_windows_menu_for_nsapp();
                help.set_as_help_menu_for_nsapp();
            }
            #[cfg(windows)]
            {
                use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
                help.append_items(&[&PredefinedMenuItem::separator(), &PredefinedMenuItem::about(Some("About Leaf"), Some(about()))])
                    .expect("menu builds");
                menu.append(&help).expect("menu builds");
                if let Ok(RawWindowHandle::Win32(h)) = window.window_handle().map(|h| h.as_raw()) {
                    // SAFETY: the window's own handle, which outlives the menu.
                    if let Err(e) = unsafe { menu.init_for_hwnd(h.hwnd.get()) } {
                        crate::log::push("leaf", format!("couldn't add the menu bar: {e}"));
                    }
                }
            }

            // muda allows one handler per process; ids are looked up in
            // whichever menu bar is current when the event arrives.
            HANDLER.get_or_init(|| MenuEvent::set_event_handler(Some(move |e: MenuEvent| send(e.id.0))));

            MenuBar { _menu: menu, commands: b.commands, checks }
        }

        pub fn command(&self, id: &str) -> Option<String> {
            self.commands.get(id.parse::<usize>().ok()?).cloned()
        }

        /// Updates the View checkmarks without rebuilding the menus.
        pub fn sync(&self, view: &View) {
            for (command, item) in &self.checks {
                item.set_checked(checked(view, command));
                item.set_enabled(enabled(view, command));
            }
        }
    }

    const CHECKS: [&str; 9] = [
        "mode-comic", "mode-manga", "mode-webtoon", "toggle-spreads", "pair-cover", "toggle-rtl", "toggle-fit", "toggle-magnifier",
        "toggle-upscale",
    ];

    /// Upscaling needs an upscaler adapter; the rest, a book.
    fn enabled(view: &View, command: &str) -> bool {
        if command == "toggle-upscale" { view.upscale.is_some() } else { view.has_book }
    }

    fn checked(view: &View, command: &str) -> bool {
        match command {
            "mode-comic" => view.mode == Some(Mode::Comic),
            "mode-manga" => view.mode == Some(Mode::Manga),
            "mode-webtoon" => view.mode == Some(Mode::Webtoon),
            "toggle-spreads" => view.spreads,
            "pair-cover" => view.pair_cover,
            "toggle-rtl" => view.rtl,
            "toggle-fit" => view.fit_width,
            "toggle-magnifier" => view.magnifier,
            "toggle-upscale" => view.upscale == Some(true),
            _ => false,
        }
    }

    fn about() -> muda::AboutMetadata {
        let icon = image::load_from_memory(include_bytes!("../assets/icon-256.png")).ok().map(|i| i.into_rgba8());
        AboutMetadataBuilder::new()
            .name(Some("Leaf"))
            .version(Some(env!("CARGO_PKG_VERSION")))
            .comments(Some("A bare comic and manga reader that opens where you left off."))
            .copyright(Some("© 2026 Vishal Mohan · MIT License"))
            .icon(icon.and_then(|i| Icon::from_rgba(i.to_vec(), i.width(), i.height()).ok()))
            .build()
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
pub struct MenuBar;

#[cfg(not(any(target_os = "macos", windows)))]
impl MenuBar {
    pub fn install(_: &Content<'_>, _: &winit::window::Window, _: impl Fn(String) + Send + Sync + 'static) -> Self {
        MenuBar
    }
    pub fn command(&self, _: &str) -> Option<String> {
        None
    }
    pub fn sync(&self, _: &View) {}
}
