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
    ("cmd+w", "close-book"),
    ("cmd+q", "quit"),
    ("cmd+ctrl+f", "fullscreen"),
    ("cmd+up", "first"),
    ("cmd+down", "last"),
    ("cmd+,", "open-config-folder"),
];

pub const COMMANDS: &[&str] = &[
    "next", "prev", "page-right", "page-left", "first", "last", "toggle-rtl", "toggle-fit",
    "fullscreen", "leave-fullscreen", "open-file", "open-folder", "close-book", "quit",
    "open-config-folder", "show-adapters-folder", "reload-adapters", "show-log", "edit-keys", "help", "none",
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
#   open-file, open-folder, close-book, quit
#   open-config-folder, show-adapters-folder, reload-adapters, show-log, edit-keys, help
#   adapter "Name" id     an adapter's command
#   none                  unbind a built-in key
#
# Examples:
# "d" = "page-right"
# "a" = "page-left"
# "h" = "none"
# "cmd+k" = 'adapter "Some Adapter" its-command'
"#;

pub struct Keys {
    map: HashMap<String, String>,
}

impl Keys {
    /// Built-in bindings, then adapters' default keys, then keys.toml. Returns
    /// any problem with the file so it can be shown; a bad file never stops Leaf.
    pub fn load(adapter_keys: &[(String, String)]) -> (Keys, Option<String>) {
        let path = crate::state::dir().join("keys.toml");
        let text = fs::read_to_string(&path).ok();
        if text.is_none() {
            let _ = fs::create_dir_all(crate::state::dir());
            let _ = fs::write(&path, EXAMPLE);
        }
        Self::build(adapter_keys, text.as_deref())
    }

    fn build(adapter_keys: &[(String, String)], user: Option<&str>) -> (Keys, Option<String>) {
        let mut map: HashMap<String, String> = DEFAULTS.iter().map(|(k, c)| (k.to_string(), c.to_string())).collect();
        for (key, command) in adapter_keys {
            map.entry(key.to_ascii_lowercase()).or_insert_with(|| command.clone());
        }
        let Some(text) = user else { return (Keys { map }, None) };
        let mut problem = None;
        match toml::from_str::<HashMap<String, String>>(text) {
            Ok(user) => {
                for (key, cmd) in user {
                    if COMMANDS.contains(&cmd.as_str()) || crate::adapters::parse_command(&cmd).is_some() {
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

    /// The shortcut a menu shows for `command`: one with Cmd (or Ctrl), since
    /// a plain key in the menu bar would make the menu flash on every page
    /// turn. Plain keys still work; they go straight to the window.
    pub fn menu_key(&self, command: &str) -> Option<&str> {
        let mut keys: Vec<&str> = self
            .map
            .iter()
            .filter(|(k, c)| *c == command && (k.starts_with("cmd+") || k.starts_with("ctrl+")))
            .map(|(k, _)| k.as_str())
            .collect();
        keys.sort_by_key(|k| (k.len(), *k));
        keys.first().copied()
    }
}

/// A key name as the menu library writes it ("cmd+shift+o" → "CmdOrCtrl+Shift+O").
pub fn accelerator(key: &str) -> Option<String> {
    let mut parts: Vec<&str> = key.split('+').collect();
    // "cmd++" binds the plus key.
    if key.ends_with("++") {
        parts.pop();
        if let Some(last) = parts.last_mut() {
            *last = "+";
        }
    }
    let (base, mods) = parts.split_last()?;
    let mut out: Vec<String> = mods
        .iter()
        .map(|m| match *m {
            "cmd" => "CmdOrCtrl",
            "ctrl" => "Ctrl",
            "alt" => "Alt",
            "shift" => "Shift",
            other => other,
        })
        .map(String::from)
        .collect();
    let base = match *base {
        "right" => "ArrowRight".into(),
        "left" => "ArrowLeft".into(),
        "up" => "ArrowUp".into(),
        "down" => "ArrowDown".into(),
        "space" => "Space".into(),
        "pageup" => "PageUp".into(),
        "pagedown" => "PageDown".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "escape" => "Escape".into(),
        "backspace" => "Backspace".into(),
        "enter" => "Enter".into(),
        "tab" => "Tab".into(),
        "," => "Comma".into(),
        "." => "Period".into(),
        "/" => "Slash".into(),
        "+" => "Equal".into(),
        "-" => "Minus".into(),
        other if other.chars().count() == 1 => other.to_uppercase(),
        _ => return None,
    };
    out.push(base);
    Some(out.join("+"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accelerators() {
        assert_eq!(accelerator("cmd+shift+o").as_deref(), Some("CmdOrCtrl+Shift+O"));
        assert_eq!(accelerator("cmd+,").as_deref(), Some("CmdOrCtrl+Comma"));
        assert_eq!(accelerator("cmd+ctrl+f").as_deref(), Some("CmdOrCtrl+Ctrl+F"));
        assert_eq!(accelerator("cmd+up").as_deref(), Some("CmdOrCtrl+ArrowUp"));
    }

    #[test]
    fn menus_show_cmd_keys_only() {
        let (keys, problem) = Keys::build(&[("cmd+alt+k".into(), r#"adapter "A" k"#.into())], Some("\"cmd+o\" = \"open-folder\"\n\"x\" = \"fly\""));
        assert_eq!(problem.as_deref(), Some("keys.toml: unknown command \"fly\""));
        assert_eq!(keys.menu_key("open-folder"), Some("cmd+o"));
        assert_eq!(keys.menu_key("open-file"), None);
        assert_eq!(keys.menu_key("next"), None);
        assert_eq!(keys.menu_key(r#"adapter "A" k"#), Some("cmd+alt+k"));
    }
}
