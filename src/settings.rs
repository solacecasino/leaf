//! `settings.toml`: the things that would otherwise be fixed in the code.
//! leaf9000 writes it on first run with every setting commented out, so it
//! starts bare; a line you uncomment changes only that. Saved changes apply
//! straight away. A mistake shows in the title bar and the rest still works.

use serde::Deserialize;
use std::sync::{Arc, RwLock};

const EXAMPLE: &str = r##"# leaf9000 settings. Every line is commented out, so it uses its own choice;
# remove the # in front of one to change it. Saved changes apply straight
# away (the text book settings, the next time a text book opens).

# background = "#17181b"     # behind the pages: the 9000 series' graphite
# accent = "#ffc23d"         # the progress line, the drop outline, the magnifier's edge
# progress = true            # the thin progress line along the bottom
# page_gap = 0               # points between two pages side by side
# webtoon_width = 800        # the widest a webtoon strip is drawn, in points
# default_mode = "auto"      # how a new book is read: "auto", "comic", "manga" or "webtoon"
# color_threshold = 3        # percent of a page that must be in colour for it to count as colour
# continue_to_next = true    # turning past the last page opens the next book in the folder
# magnifier_zoom = 2.0       # how much the magnifier (Z) enlarges
# upscale_style = "auto"     # with an upscaler adapter: "auto" (line art for manga and
#                            # webtoons, soft for comics), "lines" or "soft"
# upscale_strength = "normal" # how hard it works the lines: "gentle", "normal" or
#                            # "strong" (crisper, with slight stair steps on curves)

# [text]                     # books of writing: .txt, .rtf, Word, text EPUBs
# font = "Iowan Old Style"
# size = 17
# paper = "#faf7f0"
# ink = "#211f1c"

# [apps]                     # the other 9000 apps, if leaf9000 doesn't find them itself
# grace9000 = "/Applications/grace9000.app"
"##;

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    pub background: Color,
    pub accent: Color,
    pub progress: bool,
    pub page_gap: f64,
    pub webtoon_width: f64,
    pub default_mode: String,
    pub color_threshold: f64,
    pub continue_to_next: bool,
    pub magnifier_zoom: f64,
    pub upscale_style: String,
    pub upscale_strength: String,
    pub text: TextSettings,
    /// Where the sister apps are, by name (`[apps]`).
    pub apps: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct TextSettings {
    pub font: String,
    pub size: f64,
    pub paper: Color,
    pub ink: Color,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            background: Color(crate::series::GRAPHITE),
            accent: Color(crate::series::GOLD),
            progress: true,
            page_gap: 0.0,
            webtoon_width: 800.0,
            default_mode: "auto".into(),
            color_threshold: 3.0,
            continue_to_next: true,
            magnifier_zoom: 2.0,
            upscale_style: "auto".into(),
            upscale_strength: "normal".into(),
            text: TextSettings::default(),
            apps: Default::default(),
        }
    }
}

impl Default for TextSettings {
    fn default() -> Self {
        TextSettings { font: "Iowan Old Style".into(), size: 17.0, paper: Color(0xfaf7f0), ink: Color(0x211f1c) }
    }
}

/// A colour written "#rrggbb", kept as 0RGB.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(pub u32);

impl Color {
    /// Red, green and blue from 0 to 1, for Core Graphics.
    pub fn unit(self) -> [f64; 3] {
        [(self.0 >> 16 & 0xff) as f64 / 255.0, (self.0 >> 8 & 0xff) as f64 / 255.0, (self.0 & 0xff) as f64 / 255.0]
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let hex = s.trim().trim_start_matches('#');
        match u32::from_str_radix(hex, 16) {
            Ok(v) if hex.len() == 6 => Ok(Color(v)),
            _ => Err(serde::de::Error::custom(format!("\"{s}\" isn't a colour like \"#1b1b1d\""))),
        }
    }
}

const KNOWN: &[&str] = &[
    "background", "accent", "progress", "page_gap", "webtoon_width", "default_mode", "color_threshold",
    "continue_to_next", "magnifier_zoom", "upscale_style", "upscale_strength", "text", "apps",
];
const KNOWN_TEXT: &[&str] = &["font", "size", "paper", "ink"];

static CURRENT: RwLock<Option<Arc<Settings>>> = RwLock::new(None);

/// The settings in force.
pub fn get() -> Arc<Settings> {
    CURRENT.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

/// Reads settings.toml (writing the commented example the first time).
/// Returns any problem with it so it can be shown; a bad file never stops
/// leaf9000, it just keeps the built-in choices.
pub fn load() -> Option<String> {
    let path = crate::state::dir().join("settings.toml");
    let text = std::fs::read_to_string(&path).ok();
    if text.is_none() {
        let _ = std::fs::create_dir_all(crate::state::dir());
        let _ = std::fs::write(&path, EXAMPLE);
    }
    let (settings, problem) = parse(text.as_deref().unwrap_or(""));
    *CURRENT.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(settings));
    problem
}

fn parse(text: &str) -> (Settings, Option<String>) {
    let table: toml::Table = match toml::from_str(text) {
        Ok(t) => t,
        Err(e) => return (Settings::default(), Some(format!("settings.toml: {}", e.message()))),
    };
    let mut problem = None;
    for key in table.keys().filter(|k| !KNOWN.contains(&k.as_str())) {
        problem = Some(format!("settings.toml: unknown setting \"{key}\""));
    }
    if let Some(text) = table.get("text").and_then(|t| t.as_table()) {
        for key in text.keys().filter(|k| !KNOWN_TEXT.contains(&k.as_str())) {
            problem = Some(format!("settings.toml: unknown setting \"text.{key}\""));
        }
    }
    if let Some(apps) = table.get("apps").and_then(|t| t.as_table()) {
        for key in apps.keys().filter(|k| crate::series::sibling(k).is_none()) {
            problem = Some(format!("settings.toml: [apps] has \"{key}\"; it knows text9000, splice9000, wave9000 and grace9000"));
        }
    }
    let mut settings: Settings = match table.try_into() {
        Ok(s) => s,
        Err(e) => return (Settings::default(), Some(format!("settings.toml: {}", e.message()))),
    };
    // Out-of-range numbers are held to something that still works.
    let d = Settings::default();
    let mut check = |ok: bool, name: &str, fix: &mut dyn FnMut()| {
        if !ok {
            problem = Some(format!("settings.toml: {name} is out of range"));
            fix();
        }
    };
    let s = &mut settings;
    check((0.0..=200.0).contains(&s.page_gap), "page_gap", &mut || s.page_gap = d.page_gap);
    check((200.0..=4000.0).contains(&s.webtoon_width), "webtoon_width", &mut || s.webtoon_width = d.webtoon_width);
    check((0.0..=100.0).contains(&s.color_threshold), "color_threshold", &mut || s.color_threshold = d.color_threshold);
    check((1.2..=6.0).contains(&s.magnifier_zoom), "magnifier_zoom", &mut || s.magnifier_zoom = d.magnifier_zoom);
    check((6.0..=72.0).contains(&s.text.size), "text.size", &mut || s.text.size = d.text.size);
    check(
        ["auto", "lines", "soft"].contains(&s.upscale_style.as_str()),
        "upscale_style (use auto, lines or soft)",
        &mut || s.upscale_style = d.upscale_style.clone(),
    );
    check(
        ["gentle", "normal", "strong"].contains(&s.upscale_strength.as_str()),
        "upscale_strength (use gentle, normal or strong)",
        &mut || s.upscale_strength = d.upscale_strength.clone(),
    );
    check(
        ["auto", "comic", "manga", "webtoon"].contains(&s.default_mode.as_str()),
        "default_mode (use auto, comic, manga or webtoon)",
        &mut || s.default_mode = d.default_mode.clone(),
    );
    (settings, problem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_example_is_all_defaults() {
        let (s, problem) = parse(EXAMPLE);
        assert!(problem.is_none());
        assert_eq!(s.background, Settings::default().background);
        assert_eq!(s.text, TextSettings::default());
    }

    #[test]
    fn reads_what_is_set_and_reports_mistakes() {
        let (s, problem) = parse("background = \"#000000\"\npage_gap = 8\n[text]\nsize = 20\n");
        assert!(problem.is_none());
        assert_eq!((s.background.0, s.page_gap, s.text.size, s.text.font.as_str()), (0, 8.0, 20.0, "Iowan Old Style"));

        let (s, problem) = parse("backgrond = \"#000000\"\nwebtoon_width = 5\n");
        assert_eq!(s.webtoon_width, 800.0);
        assert!(problem.unwrap().contains("webtoon_width"));
        assert!(parse("backgrond = 1").1.unwrap().contains("unknown setting \"backgrond\""));

        let (s, problem) = parse("upscale_strength = \"strong\"");
        assert!(problem.is_none());
        assert_eq!(s.upscale_strength, "strong");
        let (s, problem) = parse("upscale_strength = \"max\"");
        assert_eq!(s.upscale_strength, "normal");
        assert!(problem.unwrap().contains("use gentle, normal or strong"));

        let (s, problem) = parse("[apps]\ngrace9000 = \"/x/grace9000.app\"\n");
        assert!(problem.is_none());
        assert_eq!(s.apps.get("grace9000").map(String::as_str), Some("/x/grace9000.app"));
        assert!(parse("[apps]\ngrace = \"/x\"\n").1.unwrap().contains("[apps] has \"grace\""));

        let (s, problem) = parse("accent = \"green\"");
        assert_eq!(s.accent, Settings::default().accent);
        assert!(problem.unwrap().contains("isn't a colour"));
    }
}
