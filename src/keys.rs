//! Key bindings: one table from key names to command names. `keys.toml` in
//! the config folder adds to and overrides the defaults.

use std::collections::HashMap;
use std::fs;
use winit::keyboard::{Key, ModifiersState, NamedKey};

const DEFAULTS: &[(&str, &str)] = &[
    ("right", "page-right"),
    ("l", "page-right"),
    ("left", "page-left"),
    ("h", "page-left"),
    ("space", "next"),
    ("down", "next"),
    ("j", "next"),
    ("pagedown", "next"),
    ("shift+space", "prev"),
    ("up", "prev"),
    ("k", "prev"),
    ("pageup", "prev"),
    ("backspace", "prev"),
    ("home", "first"),
    ("end", "last"),
    ("m", "toggle-rtl"),
    ("w", "toggle-fit"),
    ("f", "fullscreen"),
    ("escape", "leave-fullscreen"),
    ("cmd+o", "open-file"),
    ("cmd+shift+o", "open-folder"),
];

pub const COMMANDS: &[&str] = &[
    "next", "prev", "page-right", "page-left", "first", "last", "toggle-rtl", "toggle-fit",
    "fullscreen", "leave-fullscreen", "open-file", "open-folder", "none",
];

const EXAMPLE: &str = r#"# Leaf key bindings. Lines here add to or replace the built-in ones.
# Saved changes apply the next time Leaf starts.
#
# Keys are written like "cmd+shift+o": cmd means Cmd on macOS and Ctrl
# elsewhere. Named keys: right left up down space pageup pagedown home end
# escape backspace enter tab.
#
# Commands:
#   next, prev            the next/previous screen (scrolls first in fit-width)
#   page-right, page-left  turn toward that side (respects right-to-left)
#   first, last
#   toggle-rtl            manga (right-to-left) reading
#   toggle-fit            fit the whole page or fit the width
#   fullscreen, leave-fullscreen
#   open-file, open-folder
#   none                  unbind a built-in key
#
# Examples:
# "d" = "page-right"
# "a" = "page-left"
# "h" = "none"
"#;

pub struct Keys {
    map: HashMap<String, String>,
}

impl Keys {
    /// Built-in bindings plus keys.toml. Returns any problem with the file so
    /// it can be shown; a bad file never stops Leaf.
    pub fn load() -> (Keys, Option<String>) {
        let mut map: HashMap<String, String> = DEFAULTS.iter().map(|(k, c)| (k.to_string(), c.to_string())).collect();
        let path = crate::state::dir().join("keys.toml");
        let Ok(text) = fs::read_to_string(&path) else {
            let _ = fs::create_dir_all(crate::state::dir());
            let _ = fs::write(&path, EXAMPLE);
            return (Keys { map }, None);
        };
        let mut problem = None;
        match toml::from_str::<HashMap<String, String>>(&text) {
            Ok(user) => {
                for (key, cmd) in user {
                    if COMMANDS.contains(&cmd.as_str()) {
                        map.insert(key.to_ascii_lowercase(), cmd);
                    } else {
                        problem = Some(format!("keys.toml: unknown command \"{cmd}\""));
                    }
                }
            }
            Err(e) => problem = Some(format!("keys.toml: {}", e.message())),
        }
        (Keys { map }, problem)
    }

    pub fn command(&self, key: &Key, mods: ModifiersState) -> Option<&str> {
        self.map.get(&name(key, mods)?).map(String::as_str).filter(|c| *c != "none")
    }
}

fn name(key: &Key, mods: ModifiersState) -> Option<String> {
    let base = match key {
        Key::Character(s) => s.to_lowercase(),
        Key::Named(named) => match named {
            NamedKey::ArrowRight => "right",
            NamedKey::ArrowLeft => "left",
            NamedKey::ArrowUp => "up",
            NamedKey::ArrowDown => "down",
            NamedKey::Space => "space",
            NamedKey::PageUp => "pageup",
            NamedKey::PageDown => "pagedown",
            NamedKey::Home => "home",
            NamedKey::End => "end",
            NamedKey::Escape => "escape",
            NamedKey::Backspace => "backspace",
            NamedKey::Enter => "enter",
            NamedKey::Tab => "tab",
            _ => return None,
        }
        .to_string(),
        _ => return None,
    };
    let cmd = if cfg!(target_os = "macos") { mods.super_key() } else { mods.control_key() };
    let mut out = String::new();
    if cmd {
        out.push_str("cmd+");
    }
    if cfg!(target_os = "macos") && mods.control_key() {
        out.push_str("ctrl+");
    }
    if mods.alt_key() {
        out.push_str("alt+");
    }
    if mods.shift_key() {
        out.push_str("shift+");
    }
    out.push_str(&base);
    Some(out)
}
