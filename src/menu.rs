//! The macOS menu bar: Leaf, File, View, Go, Adapters (only when some are
//! installed), Window and Help. It's built from the key bindings, so it
//! shows the user's own shortcuts, and every item is a named command the app
//! runs exactly like a key press. It's rebuilt when the book or the adapters
//! change; the View checkmarks are updated in place.
//!
//! Other systems get no menu bar yet; every command has a key.

use crate::adapters::{Adapter, command_name};
use crate::keys::Keys;
use std::path::PathBuf;

/// What the menus show besides the fixed commands.
pub struct Content<'a> {
    pub keys: &'a Keys,
    pub adapters: &'a [Adapter],
    pub recent: &'a [PathBuf],
    pub has_book: bool,
    pub rtl: bool,
    pub fit_width: bool,
}

/// The menu text for a built-in command.
fn label(command: &str) -> &'static str {
    match command {
        "open-file" => "Open…",
        "open-folder" => "Open Folder…",
        "close-book" => "Close Book",
        "toggle-fit" => "Fit Width",
        "toggle-rtl" => "Right to Left (Manga)",
        "fullscreen" => "Toggle Full Screen",
        "next" => "Next Page",
        "prev" => "Previous Page",
        "page-right" => "Page to the Right",
        "page-left" => "Page to the Left",
        "first" => "First Page",
        "last" => "Last Page",
        "open-config-folder" => "Open Config Folder",
        "edit-keys" => "Edit Key Bindings",
        "show-adapters-folder" => "Show Adapters Folder",
        "reload-adapters" => "Reload Adapters",
        "show-log" => "Show Log",
        "help" => "Leaf Help",
        "quit" => "Quit Leaf",
        _ => "",
    }
}

#[cfg(target_os = "macos")]
pub use mac::MenuBar;

#[cfg(target_os = "macos")]
mod mac {
    use super::{Content, command_name, label};
    use muda::accelerator::Accelerator;
    use muda::{AboutMetadataBuilder, CheckMenuItem, Icon, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
    use std::sync::OnceLock;

    pub struct MenuBar {
        _menu: Menu,
        /// Menu item ids are indexes into this list.
        commands: Vec<String>,
        fit: Option<CheckMenuItem>,
        rtl: Option<CheckMenuItem>,
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
        pub fn install(content: &Content<'_>, send: impl Fn(String) + Send + Sync + 'static) -> Self {
            let mut b = Builder { content, commands: Vec::new() };
            let book = content.has_book;
            let menu = Menu::new();
            let sep = PredefinedMenuItem::separator;

            let app = Submenu::new("Leaf", true);
            app.append_items(&[
                &PredefinedMenuItem::about(Some("About Leaf"), Some(about())),
                &sep(),
                &b.builtin("open-config-folder", true),
                &b.builtin("edit-keys", true),
                &b.builtin("show-adapters-folder", true),
                &b.builtin("reload-adapters", true),
                &sep(),
                &PredefinedMenuItem::hide(None),
                &PredefinedMenuItem::hide_others(None),
                &PredefinedMenuItem::show_all(None),
                &sep(),
                &b.builtin("quit", true),
            ])
            .expect("menu builds");

            let file = Submenu::new("File", true);
            let recent = Submenu::new("Open Recent", !content.recent.is_empty());
            for (i, path) in content.recent.iter().enumerate() {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                recent.append(&b.item(&format!("open-recent {i}"), &name, true)).expect("menu builds");
            }
            file.append_items(&[&b.builtin("open-file", true), &b.builtin("open-folder", true), &recent, &sep(), &b.builtin("close-book", book)])
                .expect("menu builds");
            b.adapter_items(&file, "file");

            let view = Submenu::new("View", true);
            let fit = b.check("toggle-fit", content.fit_width, book);
            let rtl = b.check("toggle-rtl", content.rtl, book);
            view.append_items(&[&fit, &rtl, &sep(), &b.builtin("fullscreen", true)]).expect("menu builds");
            b.adapter_items(&view, "view");

            let go = Submenu::new("Go", true);
            go.append_items(&[
                &b.builtin("next", book),
                &b.builtin("prev", book),
                &sep(),
                &b.builtin("first", book),
                &b.builtin("last", book),
            ])
            .expect("menu builds");
            b.adapter_items(&go, "go");

            menu.append_items(&[&app, &file, &view, &go]).expect("menu builds");

            // The Adapters menu appears only once something is installed.
            let with_commands: Vec<_> = content.adapters.iter().filter(|a| !a.manifest.commands.is_empty()).collect();
            if !with_commands.is_empty() {
                let adapters = Submenu::new("Adapters", true);
                for a in with_commands {
                    let sub = Submenu::new(&a.manifest.name, true);
                    for c in a.manifest.commands.iter().filter(|c| c.menu.is_none()) {
                        sub.append(&b.item(&command_name(&a.manifest.name, &c.id), &c.label, true)).expect("menu builds");
                    }
                    adapters.append(&sub).expect("menu builds");
                }
                menu.append(&adapters).expect("menu builds");
            }

            let window = Submenu::new("Window", true);
            window
                .append_items(&[&PredefinedMenuItem::minimize(None), &PredefinedMenuItem::maximize(None)])
                .expect("menu builds");
            let help = Submenu::new("Help", true);
            help.append_items(&[&b.builtin("help", true), &b.builtin("show-log", true)]).expect("menu builds");
            menu.append_items(&[&window, &help]).expect("menu builds");

            menu.init_for_nsapp();
            window.set_as_windows_menu_for_nsapp();
            help.set_as_help_menu_for_nsapp();

            // muda allows one handler per process; ids are looked up in
            // whichever menu bar is current when the event arrives.
            HANDLER.get_or_init(|| MenuEvent::set_event_handler(Some(move |e: MenuEvent| send(e.id.0))));

            MenuBar { _menu: menu, commands: b.commands, fit: Some(fit), rtl: Some(rtl) }
        }

        pub fn command(&self, id: &str) -> Option<String> {
            self.commands.get(id.parse::<usize>().ok()?).cloned()
        }

        /// Updates the View checkmarks without rebuilding the menus.
        pub fn sync(&self, has_book: bool, rtl: bool, fit_width: bool) {
            for (item, checked) in [(&self.fit, fit_width), (&self.rtl, rtl)] {
                if let Some(item) = item {
                    item.set_checked(checked);
                    item.set_enabled(has_book);
                }
            }
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

#[cfg(not(target_os = "macos"))]
pub struct MenuBar;

#[cfg(not(target_os = "macos"))]
impl MenuBar {
    pub fn install(_: &Content<'_>, _: impl Fn(String) + Send + Sync + 'static) -> Self {
        MenuBar
    }
    pub fn command(&self, _: &str) -> Option<String> {
        None
    }
    pub fn sync(&self, _: bool, _: bool, _: bool) {}
}
