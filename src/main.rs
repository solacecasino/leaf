//! Leaf: a bare comic and manga reader. Opens straight back to the page you
//! were on. All state lives in `App` on the UI thread; pages are decoded on
//! one background thread (loader.rs), adapters run as separate programs
//! (adapters.rs), and everything reaches the UI as an [`Event`].

// No console window behind the reader on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]
// Some code is only used on some systems (Linux has no menu bar, only the
// Mac reads PDFs and text books); the Mac build still checks for unused code.
#![cfg_attr(not(target_os = "macos"), allow(dead_code, unused_imports))]

mod adapters;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod archive;
mod book;
mod epub;
mod incoming;
mod keys;
mod loader;
mod log;
mod menu;
#[cfg(target_os = "macos")]
mod native;
mod platform;
// Windows and Linux read PDFs, text books, archives and more pictures
// themselves; on a Mac this is built only for the tests.
#[cfg(any(not(target_os = "macos"), test))]
mod portable;
mod settings;
mod state;
mod upscale;
mod watch;

use adapters::{BOOK_CONTROL, BOOK_READ, Host};
use book::Book;
use keys::Keys;
use loader::{BAND, Done, Loaded, Loader, PageInfo, Part, Piece, Scaled, Target};
use serde_json::{Value, json};
use softbuffer::{Context, Surface};
use state::{BookState, Fit, Mode, State};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Fullscreen, Window, WindowId};

#[global_allocator]
static ALLOCATOR: platform::Allocator = platform::Allocator;

/// A webtoon page that hasn't arrived yet.
const PLACEHOLDER: u32 = 0x242427;
const BROKEN: u32 = 0x3a2224;
/// Wheel (or sideways swipe) distance, in pixels, that turns a page.
const WHEEL_PAGE: f64 = 120.0;
/// How far a pinch has to go to change the zoom.
const PINCH_STEP: f64 = 0.18;
/// Pages looked at to tell manga, comics and webtoons apart (after the cover).
const SAMPLE_PAGES: usize = 4;
/// How long a message from an adapter (or Leaf) stays in the title bar.
const NOTICE: Duration = Duration::from_secs(5);
/// The most pixels a magnifier picture may have (about 160 MB).
const ZOOM_PIXELS: f64 = 40_000_000.0;
const HELP_URL: &str = "https://github.com/solacecasino/leaf#readme";
/// Files Leaf opens as the next book in a folder, besides folders and
/// files like the one being read.
const BOOK_EXTS: &[&str] = &["cbz", "cbr", "cb7", "cbt", "zip", "rar", "7z", "tar", "pdf", "epub"];

/// Everything that wakes the UI thread.
pub enum Event {
    Loaded(Loaded),
    /// Finder asked Leaf to open something (see incoming.rs).
    Incoming,
    Menu(String),
    Adapter(adapters::Message),
    /// Something changed in the config folder (see watch.rs).
    Config,
    /// The upscaler adapter is ready (its GPU's name), or couldn't start.
    UpscalerReady(Result<String, String>),
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version") => return println!("Leaf {}", env!("CARGO_PKG_VERSION")),
        Some("--check") => std::process::exit(check(args.get(1).map(PathBuf::from))),
        _ => {}
    }
    log::init();
    log::catch_crashes();
    let event_loop = EventLoop::<Event>::with_user_event().build().expect("create the event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let wake = proxy.clone();
    incoming::listen_early(move || drop(wake.send_event(Event::Incoming)));
    let config = proxy.clone();
    watch::config_folder(&state::dir(), move || drop(config.send_event(Event::Config)));
    let mut app = App::new(proxy);
    if let Err(e) = event_loop.run_app(&mut app) {
        log::push("leaf", e.to_string());
    }
}

/// `leaf --check <book>`: opens a book without a window and decodes its
/// first page, for testing a build. Exits 0 when it worked.
fn check(path: Option<PathBuf>) -> i32 {
    let Some(path) = path else {
        eprintln!("usage: leaf --check <book>");
        return 2;
    };
    let result = Book::open(&path).and_then(|(book, _)| {
        book.ensure(0);
        let page = book.page(0, Target { w: 800, h: 1000, fit: Fit::Page })?;
        let (w, h) = match page {
            book::Page::Bytes(bytes) => loader::decode(&bytes)?.dimensions(),
            book::Page::Drawn(img) => img.dimensions(),
        };
        Ok(format!("{}: {} pages, the first {w}x{h}", book.name, book.estimate()))
    });
    match result {
        Ok(text) => {
            println!("{text}");
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

struct Open {
    id: u64,
    book: Arc<Book>,
    key: String,
    at: BookState,
    /// Each piece shown or prepared, with the size it was made for.
    cache: HashMap<Piece, (Target, Result<Arc<Scaled>, String>)>,
    /// What was last asked of the loader, and at what size.
    wanted: HashMap<Piece, Target>,
    /// Pieces at their previous size, drawn until the re-scaled ones arrive
    /// so a resize never flashes empty.
    stale: HashMap<Piece, Arc<Scaled>>,
    /// What's known about each page so far.
    info: HashMap<usize, PageInfo>,
}

impl Open {
    fn mode(&self) -> Mode {
        self.at.mode.unwrap_or(Mode::Comic)
    }

    /// The last page. A text book still being laid out goes on at least
    /// one page past what's laid out (or to the page you were on).
    fn last(&self) -> usize {
        let n = self.book.len();
        if self.book.complete() { n.saturating_sub(1) } else { n.max(self.at.page) }
    }

    fn spreads(&self) -> bool {
        !self.book.hints.text && self.mode() != Mode::Webtoon && self.at.spreads.unwrap_or(self.mode() == Mode::Manga)
    }

    fn wide(&self, page: usize) -> bool {
        self.info.get(&page).is_some_and(PageInfo::wide)
    }

    /// The pages shown together with `page`: the cover alone (unless paired),
    /// a wide page alone, otherwise two at a time. Pages not seen yet count
    /// as single pages.
    fn spread(&self, page: usize) -> (usize, Option<usize>) {
        if !self.spreads() {
            return (page, None);
        }
        let n = self.book.len();
        let mut i = 0;
        loop {
            let pair = if (i == 0 && !self.at.pair_cover) || self.wide(i) || i + 1 >= n || self.wide(i + 1) {
                (i, None)
            } else {
                (i, Some(i + 1))
            };
            let end = pair.1.unwrap_or(pair.0);
            if page <= end {
                return pair;
            }
            i = end + 1;
        }
    }

    fn next_spread(&self) -> Option<usize> {
        let (a, b) = self.spread(self.at.page);
        let next = b.unwrap_or(a) + 1;
        (next <= self.last()).then_some(next)
    }

    fn prev_spread(&self) -> Option<usize> {
        let (a, _) = self.spread(self.at.page);
        (a > 0).then(|| self.spread(a - 1).0)
    }

    /// The pages in view, in reading order.
    fn shown(&self) -> Vec<usize> {
        let (a, b) = self.spread(self.at.page);
        std::iter::once(a).chain(b).collect()
    }

    fn image(&self, piece: Piece) -> Option<&Arc<Scaled>> {
        match self.cache.get(&piece) {
            Some((_, Ok(img))) => Some(img),
            _ => self.stale.get(&piece),
        }
    }

    fn broken(&self, piece: Piece) -> bool {
        matches!(self.cache.get(&piece), Some((_, Err(_))))
    }
}

/// When each config file was last changed, to tell what to reload.
#[derive(PartialEq, Clone, Copy, Default)]
struct Stamps {
    keys: Option<SystemTime>,
    settings: Option<SystemTime>,
}

impl Stamps {
    fn now() -> Stamps {
        let changed = |name: &str| std::fs::metadata(state::dir().join(name)).and_then(|m| m.modified()).ok();
        Stamps { keys: changed("keys.toml"), settings: changed("settings.toml") }
    }
}

struct App {
    proxy: EventLoopProxy<Event>,
    state: State,
    keys: Keys,
    loader: Loader,
    host: Host,
    menu: Option<menu::MenuBar>,
    window: Option<Arc<Window>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    open: Option<Open>,
    next_id: u64,
    /// Why the last book didn't open.
    problem: Option<String>,
    /// A mistake in keys.toml or settings.toml.
    config_problem: Option<String>,
    stamps: Stamps,
    notice: Option<(String, Instant)>,
    /// Digits typed after Go to Page, until Return or Escape.
    typing: Option<String>,
    magnifier: bool,
    /// The upscaler adapter, while upscaling is on.
    upscaler: Option<Arc<upscale::Upscaler>>,
    /// A file is being dragged over the window.
    hovering: bool,
    last_drop: Option<Instant>,
    /// Leaf is the active window. macOS also sends scrolling to inactive
    /// windows under the pointer, which mustn't turn your page.
    focused: bool,
    quitting: bool,
    modifiers: ModifiersState,
    cursor: PhysicalPosition<f64>,
    wheel: f64,
    swipe: f64,
    wheel_turned: Option<Instant>,
    /// Leaf just opened or came to the front: scrolling that was already
    /// going was meant for the window that was there before, and mustn't
    /// turn pages. Holds when that scrolling was last seen.
    wheel_settling: Option<Instant>,
    pinch: f64,
    started: Instant,
    /// When the page was last turned, for LEAF_TIMING.
    turned: Option<Instant>,
}

impl App {
    fn new(proxy: EventLoopProxy<Event>) -> Self {
        let settings_problem = settings::load();
        let state = state::load();
        let pages = proxy.clone();
        let calls = proxy.clone();
        let host = Host::new(move |m| drop(calls.send_event(Event::Adapter(m))));
        let (keys, keys_problem) = Keys::load(&adapter_keys(&host));
        // macOS once passed a "-psn_…" argument to apps opened from Finder.
        let arg = std::env::args_os().nth(1).filter(|a| !a.to_string_lossy().starts_with("-psn"));
        let pending = arg.map(PathBuf::from).or_else(|| state.last.clone());
        let mut app = App {
            proxy,
            state,
            keys,
            loader: Loader::new(move |loaded| pages.send_event(Event::Loaded(loaded)).is_ok()),
            host,
            menu: None,
            window: None,
            surface: None,
            open: None,
            next_id: 0,
            problem: None,
            config_problem: keys_problem.or(settings_problem),
            stamps: Stamps::now(),
            notice: None,
            typing: None,
            magnifier: false,
            upscaler: None,
            hovering: false,
            last_drop: None,
            focused: true,
            quitting: false,
            modifiers: ModifiersState::empty(),
            cursor: PhysicalPosition::default(),
            wheel: 0.0,
            swipe: 0.0,
            wheel_turned: None,
            wheel_settling: Some(Instant::now()),
            pinch: 0.0,
            started: Instant::now(),
            turned: None,
        };
        if let Some(crash) = log::last_crash() {
            log::push("leaf", format!("Leaf ran into a bug last time: {crash}"));
            app.notice = Some(("Leaf closed unexpectedly last time; Help → Show Log has the details".into(), Instant::now() + NOTICE * 2));
        }
        // Open the book before the window exists, so the page is decoded and
        // scaled while the system is still creating the window.
        if let Some(path) = pending {
            app.open_path(path);
        }
        // The upscaler starts alongside; pages don't wait for it.
        app.sync_upscaler();
        app
    }

    fn open_path(&mut self, path: PathBuf) -> bool {
        if adapters::is_package(&path) {
            self.install_adapter(&path);
            return false;
        }
        // An adapter that says it opens this kind of file was installed on
        // purpose, so it comes first.
        let opened = match self.host.reader_for(&path) {
            Some(i) if path.is_file() => self.host.start(i).and_then(|link| Book::through_adapter(&path, link)).map(|b| (b, None)),
            _ => Book::open(&path),
        };
        match opened {
            Ok((book, start)) => {
                self.remember();
                let key = book.path.to_string_lossy().into_owned();
                let mut at = self.state.books.get(&key).cloned().unwrap_or_default();
                if let Some(start) = start {
                    at.page = start;
                    at.scroll = 0.0;
                }
                // A text book is still being laid out, and keeps its page.
                if book.complete() {
                    at.page = at.page.min(book.len().saturating_sub(1));
                }
                self.state.opened(&book.path);
                self.next_id += 1;
                self.host.emit("opened", json!({ "path": book.path, "name": book.name, "pages": book.estimate() }));
                self.open = Some(Open {
                    id: self.next_id,
                    book: Arc::new(book),
                    key,
                    at,
                    cache: HashMap::new(),
                    wanted: HashMap::new(),
                    stale: HashMap::new(),
                    info: HashMap::new(),
                });
                self.problem = None;
                self.detect_from_file();
                self.moved();
                self.rebuild_menu();
                true
            }
            Err(e) => {
                log::push("leaf", &e);
                self.problem = Some(e);
                self.update_title();
                self.redraw();
                false
            }
        }
    }

    /// Decides how to read a new book from what the file says, if it says:
    /// text is read a page at a time; ComicInfo.xml (or an EPUB) can say
    /// manga, comic or webtoon; so can a name. Otherwise Leaf waits to look
    /// at a few pages, unless settings.toml says how to read new books.
    fn detect_from_file(&mut self) {
        let Some(o) = &self.open else { return };
        if o.at.mode.is_some() {
            return;
        }
        let name = o.book.path.to_string_lossy().to_lowercase();
        let file = o.book.path.file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default();
        let japanese = name.chars().any(|c| matches!(c, '\u{3040}'..='\u{30ff}'));
        // "Saga #054": an issue number, as American comics are numbered.
        let issue = file.match_indices('#').any(|(i, _)| file[i + 1..].starts_with(|c: char| c.is_ascii_digit()));
        let mode = if o.book.hints.text {
            Some(Mode::Comic)
        } else if let Some(mode) = o.book.hints.mode {
            Some(mode)
        } else if ["webtoon", "manhwa", "manhua"].iter().any(|w| name.contains(w)) {
            Some(Mode::Webtoon)
        } else if name.contains("manga") || japanese {
            Some(Mode::Manga)
        } else if issue || o.book.hints.pdf {
            Some(Mode::Comic)
        } else {
            match settings::get().default_mode.as_str() {
                "comic" => Some(Mode::Comic),
                "manga" => Some(Mode::Manga),
                "webtoon" => Some(Mode::Webtoon),
                _ => None,
            }
        };
        if let Some(mode) = mode {
            self.set_mode(mode, false);
        }
    }

    /// Decides from the first pages, once they've been looked at: mostly
    /// tall strips is a webtoon, mostly black and white is manga, otherwise
    /// a comic.
    fn detect_from_pages(&mut self) {
        let Some(o) = &self.open else { return };
        if o.at.mode.is_some() {
            return;
        }
        let sample = sample_pages(o.book.len());
        let seen: Vec<&PageInfo> = sample.iter().filter_map(|p| o.info.get(p)).collect();
        if seen.len() < sample.len() {
            return;
        }
        let half = seen.len().div_ceil(2);
        let mode = if seen.iter().filter(|i| i.tall()).count() >= half {
            Mode::Webtoon
        } else if seen.iter().filter(|i| i.gray).count() >= half {
            Mode::Manga
        } else {
            Mode::Comic
        };
        self.set_mode(mode, false);
        self.say(match mode {
            Mode::Manga => "Reading as manga: two pages, right to left (View → Comic changes it)",
            Mode::Webtoon => "Reading as a webtoon: scroll down (View → Comic changes it)",
            Mode::Comic => "Reading as a comic: one page, left to right (View → Manga changes it)",
        });
    }

    /// Switches the book to `mode`, with that mode's way of reading.
    fn set_mode(&mut self, mode: Mode, chosen: bool) {
        let Some(o) = &mut self.open else { return };
        o.at.mode = Some(mode);
        o.at.chosen = chosen;
        o.at.spreads = None;
        o.at.rtl = mode == Mode::Manga;
        o.at.scroll = 0.0;
        o.at.page = o.spread(o.at.page).0;
        self.moved();
    }

    /// Opens the book after (or before) this one in its folder: the next
    /// volume or chapter. Going back opens it at its last page.
    fn continue_book(&mut self, forward: bool) {
        let Some(o) = &self.open else { return };
        if !settings::get().continue_to_next {
            return;
        }
        let from = o.book.path.clone();
        for path in neighbours(&from, forward).into_iter().take(5) {
            if self.open_path(path) {
                if !forward {
                    self.run("last");
                }
                let name = self.open.as_ref().map(|o| o.book.name.clone()).unwrap_or_default();
                self.say(if forward { format!("On to {name}, the next book in the folder") } else { format!("Back to {name}") });
                return;
            }
        }
        self.problem = None;
        self.say(if forward { "That's the last page, and the last book in this folder" } else { "That's the first page" });
    }

    fn install_adapter(&mut self, path: &Path) {
        let manifest = match adapters::package_manifest(path) {
            Ok(m) => m,
            Err(e) => return self.say(e),
        };
        // A folder can be checked before asking; a zip is checked as it's unpacked.
        if path.is_dir()
            && let Err(e) = adapters::check_platform(path, &manifest)
        {
            return self.say(e);
        }
        let text = format!("Install {}?\n\n{}", manifest.name, adapters::summary(&manifest));
        if !platform::confirm("Install adapter?", &text, "Install", "Cancel") {
            return;
        }
        match adapters::install(path) {
            Ok(m) => {
                adapters::remember_approval(&m);
                self.reload_adapters();
                self.say(format!("Installed {} {}", m.name, m.version));
            }
            Err(e) => self.say(e),
        }
    }

    fn remove_adapter(&mut self, name: &str) {
        let Some(i) = self.host.index(name) else { return };
        let text = format!("Remove {name}? It goes to {}, so you can still get it back.", platform::TRASH);
        if !platform::confirm("Remove adapter?", &text, "Remove", "Cancel") {
            return;
        }
        let dir = self.host.all[i].dir.clone();
        self.host.stop_all();
        match platform::move_to_trash(&dir) {
            Ok(()) => {
                adapters::forget_approval(name);
                self.say(format!("Moved {name} to {}", platform::TRASH));
            }
            Err(e) => self.say(e),
        }
        self.reload_adapters();
    }

    /// Starts or stops the upscaler adapter to match View → Upscale Pages,
    /// and tells the page loader how to upscale this book.
    fn sync_upscaler(&mut self) {
        let wanted = self.state.upscale != Some(false);
        match self.host.upscaler() {
            Some(i) if wanted => {
                if self.upscaler.is_none() {
                    match self.host.start(i) {
                        Ok(link) => {
                            let up = upscale::Upscaler::new(link);
                            let proxy = self.proxy.clone();
                            up.warm_up(move |r| drop(proxy.send_event(Event::UpscalerReady(r))));
                            self.upscaler = Some(up);
                        }
                        Err(e) => {
                            self.state.upscale = Some(false);
                            self.say(e);
                        }
                    }
                }
            }
            _ => self.upscaler = None,
        }
        self.tell_loader_upscaler();
    }

    /// The upscaler and style for the open book: line art for manga and
    /// webtoons, soft for comics, unless settings.toml says otherwise.
    fn tell_loader_upscaler(&self) {
        let settings = settings::get();
        let style = match (settings.upscale_style.as_str(), self.open.as_ref().map(Open::mode)) {
            ("lines", _) => upscale::Style::Lines,
            ("soft", _) | (_, Some(Mode::Comic)) => upscale::Style::Soft,
            _ => upscale::Style::Lines,
        };
        let strength = upscale::Strength::from_setting(&settings.upscale_strength);
        self.loader.set_upscaler(self.upscaler.clone().map(|u| (u, style, strength)));
    }

    /// Pages are made again (the old ones showing until then), after the
    /// way they're made changed.
    fn remake_pages(&mut self) {
        if let Some(o) = &mut self.open {
            for (piece, (_, img)) in o.cache.drain() {
                if let Ok(img) = img {
                    o.stale.insert(piece, img);
                }
            }
        }
        self.request_pages();
        self.redraw();
    }

    fn reload_adapters(&mut self) {
        // A book read through an adapter is reopened on the restarted one.
        let reopen = self.open.as_ref().filter(|o| o.book.adapter().is_some()).map(|o| o.book.path.clone());
        self.remember();
        self.upscaler = None;
        self.loader.set_upscaler(None);
        self.host.reload();
        self.reload_keys();
        self.host.start_launch_adapters();
        self.sync_upscaler();
        match reopen {
            Some(path) => drop(self.open_path(path)),
            None => self.rebuild_menu(),
        }
    }

    fn reload_keys(&mut self) {
        let (keys, problem) = Keys::load(&adapter_keys(&self.host));
        self.keys = keys;
        self.config_problem = problem;
    }

    /// Reloads keys.toml and settings.toml if either changed since Leaf
    /// last read them.
    fn reload_config(&mut self) {
        let now = Stamps::now();
        if now == self.stamps {
            return;
        }
        let before = std::mem::replace(&mut self.stamps, now);
        if now.keys != before.keys {
            self.reload_keys();
            self.rebuild_menu();
            if self.config_problem.is_none() {
                self.say("Key bindings reloaded");
            }
        }
        if now.settings != before.settings {
            let old_text = settings::get().text.clone();
            let problem = settings::load();
            if problem.is_some() || now.keys == before.keys {
                self.config_problem = problem;
            }
            if self.config_problem.is_none() {
                self.say("Settings reloaded");
            }
            // A text book is laid out again in the new face, on the same page.
            let text_book = self.open.as_ref().filter(|o| o.book.hints.text).map(|o| o.book.path.clone());
            if let Some(path) = text_book
                && settings::get().text != old_text
            {
                self.open_path(path);
            }
            if let Some(o) = &mut self.open {
                o.cache.clear();
            }
            self.request_pages();
            self.redraw();
        }
        self.update_title();
    }

    /// Shows a short message in the title bar, and logs it.
    fn say(&mut self, text: impl Into<String>) {
        let text = text.into();
        log::push("leaf", &text);
        // A message still showing stays, so one doesn't hide another.
        let text = match self.notice.take() {
            Some((old, until)) if until > Instant::now() => format!("{old}  ·  {text}"),
            _ => text,
        };
        self.notice = Some((text, Instant::now() + NOTICE));
        self.update_title();
    }

    /// After any change of page or setting: remember it, ask for pages, redraw.
    fn moved(&mut self) {
        self.remember();
        state::save(&self.state);
        self.update_title();
        self.request_pages();
        self.redraw();
        if let Some(o) = &self.open {
            self.host.emit("page", json!({ "page": o.at.page, "pages": o.book.estimate() }));
        }
        self.sync_menu();
    }

    fn remember(&mut self) {
        if let Some(o) = &self.open {
            self.state.books.insert(o.key.clone(), o.at.clone());
        }
    }

    fn rebuild_menu(&mut self) {
        let Some(window) = self.window.clone() else { return };
        let proxy = self.proxy.clone();
        // The old menu goes first: on Windows, dropping one takes it off the window.
        self.menu = None;
        let content = menu::Content { keys: &self.keys, adapters: &self.host.all, recent: &self.state.recent, view: self.view() };
        self.menu = Some(menu::MenuBar::install(&content, &window, move |id| drop(proxy.send_event(Event::Menu(id)))));
    }

    fn view(&self) -> menu::View {
        match &self.open {
            Some(o) => menu::View {
                has_book: true,
                mode: Some(o.mode()),
                spreads: o.spreads(),
                pair_cover: o.at.pair_cover,
                rtl: o.at.rtl,
                fit_width: o.at.fit == Fit::Width,
                magnifier: self.magnifier,
                upscale: self.host.upscaler().map(|_| self.state.upscale != Some(false)),
            },
            None => menu::View { upscale: self.host.upscaler().map(|_| self.state.upscale != Some(false)), ..menu::View::default() },
        }
    }

    fn sync_menu(&self) {
        if let Some(m) = &self.menu {
            m.sync(&self.view());
        }
    }

    fn update_title(&self) {
        let Some(window) = &self.window else { return };
        let mut title = match &self.open {
            Some(o) => {
                let shown = o.shown();
                let pages = match shown.as_slice() {
                    [a, b] => format!("{}–{}", a + 1, b + 1),
                    _ => (o.at.page + 1).to_string(),
                };
                let of = if o.book.complete() { o.book.len().to_string() } else { format!("~{}", o.book.estimate()) };
                let mut t = format!("{} — {pages} / {of}", o.book.name);
                match (o.mode(), o.at.rtl) {
                    (Mode::Manga, true) => t.push_str("  ·  manga"),
                    (Mode::Manga, false) => t.push_str("  ·  manga, left to right"),
                    (Mode::Webtoon, _) => t.push_str("  ·  webtoon"),
                    (Mode::Comic, true) => t.push_str("  ·  right to left"),
                    (Mode::Comic, false) => {}
                }
                for p in shown {
                    if let Some((_, Err(e))) = o.cache.get(&(p, Part::Whole)) {
                        t.push_str(&format!("  ·  {e}"));
                    }
                }
                t
            }
            None if cfg!(target_os = "macos") => "Leaf — drop a book here, or press ⌘O".into(),
            None => "Leaf — drop a book here, or press Ctrl+O".into(),
        };
        if let Some(typed) = &self.typing {
            title = format!("Go to page: {typed}▏  (Return to go, Esc to cancel)  ·  {title}");
        }
        for problem in [&self.problem, &self.config_problem].into_iter().flatten() {
            title.push_str(&format!("  ·  {problem}"));
        }
        if let Some((notice, _)) = &self.notice {
            title.push_str(&format!("  ·  {notice}"));
        }
        window.set_title(&title);
    }

    /// The window's size in pixels. Before the window exists, the size it
    /// will open at, from the saved size and display scale.
    fn window_px(&self) -> (u32, u32) {
        match &self.window {
            Some(w) => w.inner_size().into(),
            None => LogicalSize::<f64>::from(self.window_size()).to_physical::<u32>(self.state.scale.unwrap_or(1.0)).into(),
        }
    }

    fn scale_factor(&self) -> f64 {
        self.window.as_ref().map(|w| w.scale_factor()).or(self.state.scale).unwrap_or(1.0)
    }

    fn window_size(&self) -> (f64, f64) {
        self.state.window.unwrap_or((820.0, 1100.0))
    }

    /// The gap between two pages side by side, in pixels.
    fn gap(&self) -> u32 {
        (settings::get().page_gap * self.scale_factor()).round() as u32
    }

    /// The size page `page` is prepared at in the current layout.
    fn target_for(&self, o: &Open, page: usize) -> Target {
        let (w, h) = self.window_px();
        match o.mode() {
            Mode::Webtoon => {
                let max = (settings::get().webtoon_width * self.scale_factor()) as u32;
                Target { w: w.min(max), h, fit: Fit::Width }
            }
            _ if o.spread(page).1.is_some() || (o.spreads() && o.spread(page.saturating_sub(1)).1 == Some(page)) => {
                Target { w: w.saturating_sub(self.gap()) / 2, h, fit: Fit::Page }
            }
            _ => Target { w, h, fit: if o.spreads() { Fit::Page } else { o.at.fit } },
        }
    }

    /// A webtoon page's height as drawn, in pixels: exact once its size is
    /// known, otherwise a guess.
    fn strip_height(&self, o: &Open, page: usize) -> i64 {
        let t = self.target_for(o, page);
        match o.info.get(&page) {
            Some(i) => loader::scaled_size(i.w, i.h, t).1 as i64,
            None => (t.w as f64 * 1.5) as i64,
        }
        .max(1)
    }

    /// The bands of the webtoon strip between `from` and `to` (in window
    /// pixels; 0 is the window's top), each with its top and height.
    fn strip_bands(&self, o: &Open, from: i64, to: i64) -> Vec<(Piece, i64, i64)> {
        let mut page = o.at.page;
        let mut top = -(o.at.scroll as f64 * self.strip_height(o, page) as f64).round() as i64;
        while top > from && page > 0 {
            page -= 1;
            top -= self.strip_height(o, page);
        }
        let mut out = Vec::new();
        while top < to && page <= o.last() {
            let h = self.strip_height(o, page);
            for i in 0..(h as u32).div_ceil(BAND) {
                let y = top + (i * BAND) as i64;
                let bh = (h - (i * BAND) as i64).min(BAND as i64);
                if y + bh > from && y < to {
                    out.push(((page, Part::Band(i)), y, bh));
                }
            }
            top += h;
            page += 1;
        }
        out
    }

    /// Asks for the pages in view first, then the ones a key press away, then
    /// (for a book not yet read) a few to look at.
    fn request_pages(&mut self) {
        let (w, h) = self.window_px();
        if self.open.is_none() || w == 0 || h == 0 {
            return;
        }
        self.tell_loader_upscaler();
        let Some(o) = &self.open else { return };
        let mut wanted: Vec<(Piece, Target)> = Vec::new();
        if o.mode() == Mode::Webtoon {
            // The bands in view, then one more window below, then half a
            // window above: strips are tall, so whole pages would hold far
            // too much.
            let h = h as i64;
            for (from, to) in [(0, h), (h, 2 * h), (-h / 2, 0)] {
                wanted.extend(self.strip_bands(o, from, to).into_iter().map(|(piece, ..)| (piece, self.target_for(o, piece.0))));
            }
        } else {
            let mut pages = o.shown();
            if let Some(next) = o.next_spread() {
                let (a, b) = o.spread(next);
                pages.extend(std::iter::once(a).chain(b));
            }
            if let Some(prev) = o.prev_spread() {
                let (a, b) = o.spread(prev);
                pages.extend(std::iter::once(a).chain(b));
            }
            wanted.extend(pages.into_iter().map(|p| ((p, Part::Whole), self.target_for(o, p))));
            // The magnifier's enlarged pictures, after the pages themselves.
            if self.magnifier {
                for p in o.shown() {
                    let t = self.target_for(o, p);
                    let z = settings::get().magnifier_zoom.min((ZOOM_PIXELS / (t.w as f64 * t.h as f64).max(1.0)).sqrt());
                    wanted.push(((p, Part::Zoom), Target { w: (t.w as f64 * z) as u32, h: (t.h as f64 * z) as u32, fit: t.fit }));
                }
            }
        }
        if o.at.mode.is_none() {
            // Only their size and colour matter, so they're made small.
            let small = Target { w: 64, h: 64, fit: Fit::Page };
            wanted.extend(sample_pages(o.book.len()).into_iter().filter(|p| !o.info.contains_key(p)).map(|p| ((p, Part::Whole), small)));
        }
        let mut seen = std::collections::HashSet::new();
        wanted.retain(|(piece, _)| seen.insert(*piece));

        let Some(o) = &mut self.open else { return };
        // Pieces now wanted at another size keep their old picture until the new one comes.
        for (piece, t) in &wanted {
            if let Some((old, Ok(img))) = o.cache.get(piece)
                && old != t
            {
                o.stale.insert(*piece, img.clone());
            }
        }
        o.wanted = wanted.iter().copied().collect();
        o.cache.retain(|piece, (t, _)| o.wanted.get(piece) == Some(t));
        o.stale.retain(|piece, _| o.wanted.contains_key(piece));
        let missing = wanted.into_iter().filter(|(piece, _)| !o.cache.contains_key(piece)).collect();
        self.loader.want(o.id, &o.book, missing);
    }

    fn redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    /// How many pixels the current page can scroll, in fit-width mode.
    fn scroll_range(&self) -> f64 {
        let (_, h) = self.window_px();
        let Some(o) = &self.open else { return 0.0 };
        o.image((o.at.page, Part::Whole)).map(|img| (img.h as f64 - h as f64).max(0.0)).unwrap_or(0.0)
    }

    fn go(&mut self, page: usize, scroll: f32) {
        let Some(o) = &mut self.open else { return };
        // A text book is laid out as far as the page asked for first.
        if !o.book.complete() {
            o.book.ensure(page);
        }
        let page = o.spread(page.min(o.last())).0;
        if page == o.at.page && scroll == o.at.scroll {
            return;
        }
        o.at.page = page;
        o.at.scroll = scroll;
        self.moved();
        if let Some(o) = &self.open
            && std::env::var_os("LEAF_TIMING").is_some()
        {
            let ready = o.shown().iter().all(|&p| o.cache.contains_key(&(p, Part::Whole)));
            eprintln!("leaf: turned to page {}, {}", page + 1, if ready { "ready" } else { "not ready yet" });
            self.turned = Some(Instant::now());
        }
    }

    /// Scrolls by `dy` pixels; returns false when already at that end.
    fn scroll_by(&mut self, dy: f64) -> bool {
        if self.open.as_ref().is_some_and(|o| o.mode() == Mode::Webtoon) {
            return self.scroll_strip(dy);
        }
        let range = self.scroll_range();
        let Some(o) = &mut self.open else { return false };
        if o.at.fit != Fit::Width || o.spreads() || range <= 0.0 {
            return false;
        }
        let new = ((o.at.scroll as f64 * range + dy) / range).clamp(0.0, 1.0) as f32;
        if new == o.at.scroll {
            return false;
        }
        o.at.scroll = new;
        self.redraw();
        true
    }

    /// Moves down (or up) the webtoon strip, page by page.
    fn scroll_strip(&mut self, dy: f64) -> bool {
        let (_, screen) = self.window_px();
        let Some(o) = &self.open else { return false };
        let last = o.last();
        let height = |page: usize| self.strip_height(o, page) as f64;
        let mut page = o.at.page;
        let mut offset = o.at.scroll as f64 * height(page) + dy;
        loop {
            let h = height(page);
            if offset < 0.0 && page > 0 {
                page -= 1;
                offset += height(page);
            } else if offset >= h && page < last {
                offset -= h;
                page += 1;
            } else {
                break;
            }
        }
        let h = height(page);
        // At the end, stop with the last page's bottom at the window's bottom.
        let end = if page == last { (h - screen as f64).max(0.0) } else { h };
        let offset = offset.clamp(0.0, end);
        let scroll = (offset / h) as f32;
        let (old_page, old_scroll) = (o.at.page, o.at.scroll);
        if (page, scroll) == (old_page, old_scroll) {
            return false;
        }
        let o = self.open.as_mut().expect("checked");
        o.at.page = page;
        o.at.scroll = scroll;
        if page != old_page {
            self.moved();
        } else {
            self.request_pages();
            self.redraw();
        }
        true
    }

    fn step(&mut self, forward: bool) {
        let screen = self.window_px().1 as f64 * 0.85;
        if self.scroll_by(if forward { screen } else { -screen }) {
            return;
        }
        let Some(o) = &self.open else { return };
        let to = match (o.mode(), forward) {
            (Mode::Webtoon, _) => None,
            (_, true) => o.next_spread().map(|p| (p, 0.0)),
            (_, false) => o.prev_spread().map(|p| (p, 1.0)),
        };
        match to {
            Some((page, scroll)) => self.go(page, scroll),
            None => self.continue_book(forward),
        }
    }

    /// Runs a named command, from a key, a menu or an adapter.
    fn run(&mut self, command: &str) {
        if let Some((name, id)) = adapters::parse_command(command) {
            if let Err(e) = self.host.run_command(&name, &id) {
                self.say(e);
            }
            return;
        }
        if let Some(name) = command.strip_prefix("remove-adapter ") {
            let name = name.to_owned();
            return self.remove_adapter(&name);
        }
        if let Some(i) = command.strip_prefix("open-recent ").and_then(|i| i.parse::<usize>().ok()) {
            if let Some(path) = self.state.recent.get(i).cloned() {
                self.open_path(path);
            }
            return;
        }
        let rtl = self.open.as_ref().is_some_and(|o| o.at.rtl);
        let webtoon = self.open.as_ref().is_some_and(|o| o.mode() == Mode::Webtoon);
        match command {
            "next" => self.step(true),
            "prev" => self.step(false),
            "page-right" | "page-left" if webtoon => self.step(command == "page-right"),
            "page-right" | "page-left" => {
                let forward = (command == "page-right") != rtl;
                let Some(o) = &self.open else { return };
                match if forward { o.next_spread() } else { o.prev_spread() } {
                    Some(page) => self.go(page, 0.0),
                    None => self.continue_book(forward),
                }
            }
            "first" => self.go(0, 0.0),
            "last" => self.go(usize::MAX, 0.0),
            "next-book" => self.continue_book(true),
            "prev-book" => self.continue_book(false),
            "go-to-page" if self.open.is_some() => {
                self.typing = Some(String::new());
                self.update_title();
            }
            "mode-comic" => self.set_mode(Mode::Comic, true),
            "mode-manga" => self.set_mode(Mode::Manga, true),
            "mode-webtoon" => self.set_mode(Mode::Webtoon, true),
            "toggle-spreads" => {
                if let Some(o) = &mut self.open {
                    o.at.spreads = Some(!o.spreads());
                    o.at.page = o.spread(o.at.page).0;
                    self.moved();
                }
            }
            "pair-cover" => {
                if let Some(o) = &mut self.open {
                    o.at.pair_cover = !o.at.pair_cover;
                    o.at.page = o.spread(o.at.page).0;
                    self.moved();
                }
            }
            "toggle-rtl" => {
                if let Some(o) = &mut self.open {
                    o.at.rtl = !o.at.rtl;
                    self.moved();
                }
            }
            "toggle-fit" => {
                if let Some(o) = &mut self.open {
                    o.at.fit = if o.at.fit == Fit::Page { Fit::Width } else { Fit::Page };
                    o.at.scroll = 0.0;
                    self.moved();
                }
            }
            "toggle-upscale" => {
                if self.host.upscaler().is_none() {
                    return self.say("Upscaling needs an upscaler adapter (see the adapters catalog)");
                }
                let on = self.state.upscale == Some(false);
                self.state.upscale = Some(on);
                state::save(&self.state);
                self.sync_upscaler();
                self.remake_pages();
                self.sync_menu();
                if !on {
                    self.say("Upscaling off");
                }
            }
            "toggle-magnifier" => {
                self.magnifier = !self.magnifier && self.open.is_some();
                // The enlarged pictures go as soon as it's put away.
                self.request_pages();
                self.redraw();
                self.sync_menu();
            }
            "fullscreen" => {
                if let Some(w) = &self.window {
                    w.set_fullscreen(if w.fullscreen().is_some() { None } else { Some(Fullscreen::Borderless(None)) });
                }
            }
            "leave-fullscreen" => {
                if let Some(w) = &self.window {
                    w.set_fullscreen(None);
                }
            }
            "open-file" => {
                if let Some(path) = rfd::FileDialog::new().pick_file() {
                    self.open_path(path);
                }
            }
            "open-folder" => {
                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                    self.open_path(path);
                }
            }
            "close-book" => {
                self.remember();
                self.open = None;
                self.magnifier = false;
                self.state.last = None;
                state::save(&self.state);
                self.update_title();
                self.redraw();
                self.rebuild_menu();
            }
            "quit" => self.quitting = true,
            "open-config-folder" => platform::open(&state::dir(), false),
            "show-adapters-folder" => {
                let dir = adapters::adapters_dir();
                let _ = std::fs::create_dir_all(&dir);
                platform::open(&dir, false);
            }
            "reload-adapters" => {
                self.reload_adapters();
                let n = self.host.all.len();
                self.say(format!("{n} adapter{}", if n == 1 { "" } else { "s" }));
            }
            "show-log" => platform::open(&log::path(), true),
            "copy-debug-info" => {
                platform::copy(&self.debug_info());
                self.say("Copied debug info (no book names or paths)");
            }
            "edit-keys" => platform::open(&state::dir().join("keys.toml"), true),
            "edit-settings" => platform::open(&state::dir().join("settings.toml"), true),
            "help" => platform::open_url(HELP_URL),
            _ => {}
        }
    }

    /// A key pressed while typing a page number after Go to Page.
    fn type_page(&mut self, key: &Key) {
        let Some(typed) = &mut self.typing else { return };
        match key {
            Key::Character(c) if c.chars().all(|c| c.is_ascii_digit()) && typed.len() < 7 => typed.push_str(c),
            Key::Named(NamedKey::Backspace) => drop(typed.pop()),
            Key::Named(NamedKey::Enter) => {
                let page = typed.parse::<usize>().ok().filter(|&p| p > 0);
                self.typing = None;
                if let Some(page) = page {
                    self.go(page - 1, 0.0);
                }
            }
            Key::Named(NamedKey::Escape) => self.typing = None,
            _ => {}
        }
        self.update_title();
    }

    /// What a bug report needs to know about this Leaf. Never what you're
    /// reading: no book names, and paths show ~ for the home folder.
    fn debug_info(&self) -> String {
        let mb = |b: u64| format!("{:.0} MB", b as f64 / 1_048_576.0);
        let mut out = vec![
            format!("Leaf {}", env!("CARGO_PKG_VERSION")),
            format!("System: {} {}", std::env::consts::OS, std::env::consts::ARCH),
            format!("Config folder: {}", state::dir().display()),
        ];
        if let Some((now, peak)) = platform::memory_use() {
            out.push(format!("Memory: {} (most so far: {})", mb(now), mb(peak)));
        }
        let (w, h) = self.window_px();
        out.push(format!("Window: {w}x{h} pixels at {}x", self.scale_factor()));
        if let Some(o) = &self.open {
            let kind = o.book.path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "folder".into());
            out.push(format!(
                "Book: a {kind} of {}{} pages, {:?}{}{}",
                if o.book.complete() { "" } else { "~" },
                o.book.estimate(),
                o.mode(),
                if o.spreads() { ", two pages" } else { "" },
                if o.at.rtl { ", right to left" } else { "" }
            ));
        }
        let adapters: Vec<String> = self.host.all.iter().map(|a| format!("{} {}", a.manifest.name, a.manifest.version)).collect();
        out.push(format!("Adapters: {}", if adapters.is_empty() { "none".into() } else { adapters.join(", ") }));
        if let Some(u) = &self.upscaler {
            out.push(format!("Upscaling: {} ({})", u.name, if u.usable() { "working" } else { "not working; Leaf scales" }));
        }
        let changed = |name: &str| {
            std::fs::read_to_string(state::dir().join(name))
                .is_ok_and(|t| t.lines().any(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#')))
        };
        out.push(format!(
            "Changed by you: keys.toml {}, settings.toml {}",
            if changed("keys.toml") { "yes" } else { "no" },
            if changed("settings.toml") { "yes" } else { "no" }
        ));
        for p in [&self.problem, &self.config_problem].into_iter().flatten() {
            out.push(format!("Showing: {p}"));
        }
        let problems = log::recent_problems(20);
        out.push(if problems.is_empty() { "Recent problems: none".into() } else { format!("Recent problems:\n{}", problems.join("\n")) });
        platform::hide_home(&out.join("\n"))
    }

    /// A message from an adapter. Requests are answered; a refused or
    /// unknown one gets an error back instead of being ignored.
    fn adapter_call(&mut self, adapter: String, id: Option<Value>, method: String, params: Value) {
        let allowed = |permission: &str| {
            if self.host.has(&adapter, permission) {
                Ok(())
            } else {
                Err(format!("{method} needs the {permission} permission"))
            }
        };
        let result: Result<Value, String> = match method.as_str() {
            "book.get" => allowed(BOOK_READ).map(|()| match &self.open {
                Some(o) => json!({
                    "path": o.book.path, "name": o.book.name, "page": o.at.page,
                    "pages": o.book.estimate(), "rtl": o.at.rtl,
                    "fit": if o.at.fit == Fit::Width { "width" } else { "page" },
                    "mode": format!("{:?}", o.mode()).to_lowercase(),
                }),
                None => Value::Null,
            }),
            "book.goto" => allowed(BOOK_CONTROL).and_then(|()| {
                let page = params["page"].as_u64().ok_or("book.goto needs a `page` number")?;
                self.go(page as usize, 0.0);
                Ok(Value::Null)
            }),
            "book.open" => allowed(BOOK_CONTROL).and_then(|()| {
                let path = params["path"].as_str().ok_or("book.open needs a `path`")?;
                self.open_path(path.into());
                Ok(Value::Null)
            }),
            "notify" => {
                let text: String = params["text"].as_str().unwrap_or_default().chars().take(200).collect();
                self.say(format!("{adapter}: {text}"));
                Ok(Value::Null)
            }
            "log" => {
                log::push(&adapter, params["message"].as_str().unwrap_or_default());
                Ok(Value::Null)
            }
            "ui.confirm" => {
                let title = params["title"].as_str().unwrap_or(&adapter);
                Ok(json!(platform::confirm(title, params["message"].as_str().unwrap_or_default(), "Yes", "No")))
            }
            other => Err(format!("Leaf doesn't know the method {other}")),
        };
        match (id, result) {
            (Some(id), result) => self.host.reply(&adapter, id, result),
            (None, Err(e)) => log::push(&adapter, e),
            (None, Ok(_)) => {}
        }
    }

    /// Where each picture in view goes: (piece, x, y, width, height) in
    /// window pixels. A webtoon band not here yet still gets a place.
    fn layout(&self) -> Vec<(Piece, i64, i64, i64, i64)> {
        let (w, h) = self.window_px();
        let (w, h) = (w as i64, h as i64);
        let Some(o) = &self.open else { return Vec::new() };
        let size = |p: usize| o.image((p, Part::Whole)).map(|i| (i.w as i64, i.h as i64));
        match o.mode() {
            Mode::Webtoon => self
                .strip_bands(o, 0, h)
                .into_iter()
                .map(|(piece, y, bh)| {
                    let pw = o.image(piece).map_or(self.target_for(o, piece.0).w as i64, |i| i.w as i64);
                    (piece, (w - pw) / 2, y, pw, bh)
                })
                .collect(),
            _ => match o.spread(o.at.page) {
                (a, Some(b)) => {
                    // Two pages meeting in the middle, the first on the right
                    // when reading right to left.
                    let (left, right) = if o.at.rtl { (b, a) } else { (a, b) };
                    let gap = self.gap() as i64;
                    let mut out = Vec::new();
                    if let Some((pw, ph)) = size(left) {
                        out.push(((left, Part::Whole), w / 2 - gap / 2 - pw, (h - ph) / 2, pw, ph));
                    }
                    if let Some((pw, ph)) = size(right) {
                        out.push(((right, Part::Whole), w / 2 + (gap - gap / 2), (h - ph) / 2, pw, ph));
                    }
                    out
                }
                (a, None) => {
                    let Some((pw, ph)) = size(a) else { return Vec::new() };
                    let y = if o.at.fit == Fit::Width && !o.spreads() && ph > h {
                        -(o.at.scroll as f64 * self.scroll_range()).round() as i64
                    } else {
                        (h - ph) / 2
                    };
                    vec![((a, Part::Whole), (w - pw) / 2, y, pw, ph)]
                }
            },
        }
    }

    fn draw(&mut self) {
        let Some(window) = self.window.clone() else { return };
        let size = window.inner_size();
        let (Some(nw), Some(nh)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        let placed = self.layout();
        let settings = settings::get();
        let (background, accent) = (settings.background.0, settings.accent.0);
        let hovering = self.hovering;
        let loupe = self.magnifier.then(|| (self.cursor.x as i64, self.cursor.y as i64, settings.magnifier_zoom));
        let (w, h) = (size.width as i64, size.height as i64);
        let thick = (window.scale_factor() * 2.0).round().max(1.0) as i64;
        let Some(surface) = &mut self.surface else { return };
        if surface.resize(nw, nh).is_err() {
            return;
        }
        let Ok(mut buf) = surface.buffer_mut() else { return };
        buf.fill(background);
        let open = self.open.as_ref();

        for &(piece, x0, y0, pw, ph) in &placed {
            let Some(o) = open else { break };
            let (cx0, cx1) = (x0.max(0), (x0 + pw).min(w));
            let (cy0, cy1) = (y0.max(0), (y0 + ph).min(h));
            if cx0 >= cx1 || cy0 >= cy1 {
                continue;
            }
            match o.image(piece) {
                Some(img) => {
                    let iw = img.w as i64;
                    let len = (cx1 - cx0).min(iw - (cx0 - x0)).max(0) as usize;
                    for y in cy0..cy1.min(y0 + img.h as i64) {
                        let src = ((y - y0) * iw + (cx0 - x0)) as usize;
                        let dst = (y * w + cx0) as usize;
                        buf[dst..dst + len].copy_from_slice(&img.px[src..src + len]);
                    }
                }
                None => {
                    let colour = if o.broken(piece) { BROKEN } else { PLACEHOLDER };
                    for y in cy0..cy1 {
                        buf[(y * w + cx0) as usize..(y * w + cx1) as usize].fill(colour);
                    }
                }
            }
        }
        // A page that couldn't be decoded, outside the webtoon strip: a dim card.
        if let Some(o) = open
            && o.mode() != Mode::Webtoon
            && placed.is_empty()
            && o.broken((o.at.page, Part::Whole))
        {
            let (bw, bh) = (w * 2 / 3, h * 2 / 3);
            for y in (h - bh) / 2..(h + bh) / 2 {
                let row = (y * w + (w - bw) / 2) as usize;
                buf[row..row + bw as usize].fill(BROKEN);
            }
        }

        // The magnifier: a square around the pointer showing what's under
        // it enlarged, from the enlarged picture once it has arrived.
        if let (Some((cx, cy, z)), Some(o)) = (loupe, open) {
            let side = (w.min(h) as f64 * 0.42) as i64;
            let (lx0, ly0) = (cx - side / 2, cy - side / 2);
            let (x_lo, x_hi, y_lo, y_hi) = (lx0.max(0), (lx0 + side).min(w), ly0.max(0), (ly0 + side).min(h));
            for y in y_lo..y_hi {
                let sy = cy as f64 + (y - cy) as f64 / z;
                for x in x_lo..x_hi {
                    let sx = cx as f64 + (x - cx) as f64 / z;
                    let mut colour = background;
                    for &(piece, x0, y0, pw, ph) in &placed {
                        let (u, v) = (sx - x0 as f64, sy - y0 as f64);
                        if u < 0.0 || v < 0.0 || u >= pw as f64 || v >= ph as f64 {
                            continue;
                        }
                        let zoom = if piece.1 == Part::Whole { o.image((piece.0, Part::Zoom)) } else { None };
                        colour = match zoom.or_else(|| o.image(piece)) {
                            Some(img) => {
                                let ix = ((u / pw as f64 * img.w as f64) as i64).clamp(0, img.w as i64 - 1);
                                let iy = (v / ph as f64 * if zoom.is_some() { img.h as f64 } else { ph as f64 }) as i64;
                                if iy < img.h as i64 { img.px[(iy * img.w as i64 + ix) as usize] } else { PLACEHOLDER }
                            }
                            None => PLACEHOLDER,
                        };
                        break;
                    }
                    let edge = x - lx0 < thick || lx0 + side - 1 - x < thick || y - ly0 < thick || ly0 + side - 1 - y < thick;
                    buf[(y * w + x) as usize] = if edge { accent } else { colour };
                }
            }
        }

        // Reading progress: a thin line along the bottom, from the side you start on.
        if let Some(o) = open
            && settings.progress
        {
            let n = o.book.estimate().max(1) as f64;
            let done = match o.mode() {
                Mode::Webtoon => (o.at.page as f64 + o.at.scroll as f64) / n,
                _ => (o.shown().last().copied().unwrap_or(0) + 1) as f64 / n,
            };
            let filled = ((w as f64 * done).round() as i64).clamp(0, w);
            for y in (h - thick).max(0)..h {
                let row = (y * w) as usize;
                let (a, b) = if o.at.rtl { (w - filled, w) } else { (0, filled) };
                buf[row + a as usize..row + b as usize].fill(accent);
            }
        }

        // While something is dragged over the window: an outline, so you can
        // see it'll be taken.
        if hovering {
            let edge = thick * 2;
            for y in 0..h {
                let row = (y * w) as usize;
                if y < edge || y >= h - edge {
                    buf[row..row + w as usize].fill(accent);
                } else {
                    buf[row..row + edge as usize].fill(accent);
                    buf[row + (w - edge) as usize..row + w as usize].fill(accent);
                }
            }
        }
        window.pre_present_notify();
        let _ = buf.present();
    }

    /// Notes the window's size and display scale, so the next launch can
    /// prepare the page at exactly that size. Returns whether they changed.
    fn note_window(&mut self) -> bool {
        let Some(window) = &self.window else { return false };
        if window.fullscreen().is_some() {
            return false;
        }
        let size = window.inner_size().to_logical::<f64>(window.scale_factor());
        let (old_size, old_scale) = (self.state.window, self.state.scale);
        self.state.window = Some((size.width, size.height));
        self.state.scale = Some(window.scale_factor());
        (old_size, old_scale) != (self.state.window, self.state.scale)
    }

    fn quit(&mut self, event_loop: &ActiveEventLoop) {
        self.remember();
        self.note_window();
        state::save(&self.state);
        self.host.stop_all();
        event_loop.exit();
    }

    fn open_incoming(&mut self) {
        for path in incoming::take() {
            self.open_path(path);
        }
    }

    fn page_done(&mut self, done: Done) {
        let Some(o) = &mut self.open else { return };
        if done.book != o.id {
            return;
        }
        let page = done.piece.0;
        // A page's proportions can change the layout: a wide page stands alone.
        let new_info = done.info.filter(|i| o.info.get(&page) != Some(i));
        if let Some(info) = new_info {
            o.info.insert(page, info);
        }
        if o.wanted.get(&done.piece) == Some(&done.target) {
            let shown = o.mode() == Mode::Webtoon || o.shown().contains(&page);
            if shown && std::env::var_os("LEAF_TIMING").is_some() {
                match self.turned {
                    Some(t) => eprintln!("leaf: page {} on screen {:?} after the turn", page + 1, t.elapsed()),
                    None => eprintln!("leaf: page {} on screen {:?} after start", page + 1, self.started.elapsed()),
                }
            }
            if done.image.is_ok() {
                o.stale.remove(&done.piece);
            }
            o.cache.insert(done.piece, (done.target, done.image));
        }
        if new_info.is_some() {
            let o = self.open.as_mut().expect("checked");
            o.at.page = o.spread(o.at.page).0;
            self.detect_from_pages();
            self.request_pages();
        }
        self.update_title();
        self.redraw();
    }

    /// More of a text book is laid out: its length, and the title, change.
    fn book_grew(&mut self, id: u64) {
        let Some(o) = &mut self.open else { return };
        if o.id != id {
            return;
        }
        if o.book.complete() && o.at.page > o.last() {
            o.at.page = o.last();
            self.moved();
        }
        self.update_title();
        self.redraw();
    }

    /// A sideways swipe (or tilt of the wheel) turns the page that way.
    fn swipe_by(&mut self, dx: f64) {
        if self.open.as_ref().is_none_or(|o| o.mode() == Mode::Webtoon) {
            return;
        }
        self.swipe += dx;
        if self.swipe.abs() >= WHEEL_PAGE {
            // Fingers moving left bring in the page on the right.
            let command = if self.swipe < 0.0 { "page-right" } else { "page-left" };
            self.swipe = 0.0;
            self.wheel_turned = Some(Instant::now());
            self.run(command);
        }
    }

    /// Pinching out zooms in: a single page fits its width, else the
    /// magnifier comes out. Pinching in goes back.
    fn pinch_by(&mut self, delta: f64) {
        self.pinch += delta;
        if self.pinch.abs() < PINCH_STEP {
            return;
        }
        let zoom_in = self.pinch > 0.0;
        self.pinch = 0.0;
        let Some(o) = &self.open else { return };
        let single = !o.spreads() && o.mode() != Mode::Webtoon;
        let fit_width = o.at.fit == Fit::Width;
        match (zoom_in, single, fit_width, self.magnifier) {
            (true, true, false, _) => self.run("toggle-fit"),
            (true, _, _, false) => self.run("toggle-magnifier"),
            (false, _, _, true) => self.run("toggle-magnifier"),
            (false, true, true, _) => self.run("toggle-fit"),
            _ => {}
        }
    }
}

/// The pages looked at to decide how to read a book: a few after the cover.
fn sample_pages(n: usize) -> Vec<usize> {
    if n <= 1 { vec![0] } else { (1..n).take(SAMPLE_PAGES).collect() }
}

/// The books after (or before) `path` in its folder, nearest first: other
/// folders, and files Leaf reads as books or of the same kind as `path`,
/// in natural name order.
fn neighbours(path: &Path, forward: bool) -> Vec<PathBuf> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else { return Vec::new() };
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let hidden = p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
            let e = p.extension().map(|e| e.to_string_lossy().to_lowercase());
            !hidden && (p.is_dir() || e.as_deref().is_some_and(|e| BOOK_EXTS.contains(&e) || Some(e) == ext.as_deref()))
        })
        .map(|p| (p.file_name().unwrap_or_default().to_string_lossy().into_owned(), p))
        .collect();
    names.sort_by(|a, b| book::natural_cmp(&a.0, &b.0));
    let name = name.to_string_lossy();
    let Some(i) = names.iter().position(|(n, _)| *n == name) else { return Vec::new() };
    let paths = names.into_iter().map(|(_, p)| p);
    if forward { paths.skip(i + 1).collect() } else { paths.take(i).rev().collect() }
}

/// Adapters' default keys, for the key map.
fn adapter_keys(host: &Host) -> Vec<(String, String)> {
    host.all
        .iter()
        .flat_map(|a| {
            a.manifest
                .commands
                .iter()
                .filter_map(|c| Some((c.keys.clone()?, adapters::command_name(&a.manifest.name, &c.id))))
        })
        .collect()
}

impl ApplicationHandler<Event> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes().with_title("Leaf").with_inner_size(LogicalSize::<f64>::from(self.window_size()));
        // On Linux the desktop matches the window to leaf.desktop (its menu
        // entry and icon) by this name.
        #[cfg(target_os = "linux")]
        let attrs = {
            use winit::platform::{wayland::WindowAttributesExtWayland, x11::WindowAttributesExtX11};
            let attrs = WindowAttributesExtWayland::with_name(attrs, "leaf", "leaf");
            WindowAttributesExtX11::with_name(attrs, "leaf", "leaf")
        };
        let window = Arc::new(event_loop.create_window(attrs).expect("create the window"));
        // X11 and Windows show the window's own icon in the taskbar and the
        // window switcher (Wayland takes it from leaf.desktop).
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        if let Ok(icon) = image::load_from_memory(include_bytes!("../assets/icon-256.png")).map(|i| i.into_rgba8()) {
            let (w, h) = icon.dimensions();
            let icon = winit::window::Icon::from_rgba(icon.into_raw(), w, h).ok();
            #[cfg(target_os = "windows")]
            winit::platform::windows::WindowExtWindows::set_taskbar_icon(&*window, icon.clone());
            window.set_window_icon(icon);
        }
        let context = Context::new(window.clone()).expect("connect to the display");
        self.surface = Some(Surface::new(&context, window.clone()).expect("create the drawing surface"));
        self.window = Some(window);
        if self.note_window() {
            state::save(&self.state);
        }
        incoming::listen();
        self.rebuild_menu();
        self.update_title();
        self.request_pages();
        self.redraw();
        self.host.start_launch_adapters();
        self.open_incoming();
    }

    fn user_event(&mut self, _: &ActiveEventLoop, event: Event) {
        match event {
            Event::Loaded(Loaded::Page(done)) => self.page_done(done),
            Event::Loaded(Loaded::Grew(id)) => self.book_grew(id),
            Event::Incoming => self.open_incoming(),
            Event::Config => self.reload_config(),
            Event::UpscalerReady(result) => match (result, &self.upscaler) {
                (Ok(device), Some(up)) => {
                    let text = format!("Upscaling with {} on {device}", up.name);
                    self.tell_loader_upscaler();
                    self.remake_pages();
                    self.say(text);
                }
                (Err(e), Some(_)) => {
                    self.upscaler = None;
                    self.tell_loader_upscaler();
                    self.say(format!("Upscaling isn't available: {e}"));
                }
                _ => {}
            },
            Event::Menu(id) => {
                if let Some(command) = self.menu.as_ref().and_then(|m| m.command(&id)) {
                    self.run(&command);
                }
            }
            Event::Adapter(adapters::Message::Call { adapter, id, method, params }) => {
                self.adapter_call(adapter, id, method, params)
            }
            Event::Adapter(adapters::Message::Stopped { adapter, generation }) => {
                let crashed = self.host.stopped(&adapter, generation);
                if crashed && self.open.as_ref().is_some_and(|o| o.book.adapter() == Some(adapter.as_str())) {
                    self.problem = Some(format!("{adapter} stopped; open the book again to keep reading"));
                    self.update_title();
                }
            }
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.quitting {
            return self.quit(event_loop);
        }
        // Clear an expired title-bar message, and otherwise sleep until it expires.
        match self.notice.as_ref().map(|(_, until)| *until) {
            Some(until) if until <= Instant::now() => {
                self.notice = None;
                self.update_title();
                event_loop.set_control_flow(ControlFlow::Wait);
            }
            Some(until) => event_loop.set_control_flow(ControlFlow::WaitUntil(until)),
            None => event_loop.set_control_flow(ControlFlow::Wait),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.quit(event_loop),
            WindowEvent::Resized(_) => {
                self.note_window();
                self.request_pages();
                self.redraw();
            }
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::HoveredFile(_) => {
                self.hovering = true;
                self.redraw();
            }
            WindowEvent::HoveredFileCancelled => {
                self.hovering = false;
                self.redraw();
            }
            WindowEvent::DroppedFile(path) => {
                self.hovering = false;
                // Several files dropped at once arrive one by one: open the first.
                if self.last_drop.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
                    return;
                }
                self.last_drop = Some(Instant::now());
                self.open_path(path);
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                if self.typing.is_some() {
                    return self.type_page(&event.logical_key);
                }
                let mods = self.modifiers;
                if let Some(cmd) = self.keys.command(&event.logical_key, mods).map(str::to_owned) {
                    self.run(&cmd);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = position;
                if self.magnifier {
                    self.redraw();
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                // Click the left or right third of the window to turn that way.
                let w = self.window_px().0 as f64;
                if self.cursor.x < w / 3.0 {
                    self.run("page-left");
                } else if self.cursor.x > w * 2.0 / 3.0 {
                    self.run("page-right");
                }
            }
            WindowEvent::MouseWheel { delta, phase, .. } => {
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (x as f64 * 60.0, y as f64 * 60.0),
                    MouseScrollDelta::PixelDelta(p) => (p.x, p.y),
                };
                if phase == TouchPhase::Started {
                    self.swipe = 0.0;
                }
                if settling(&mut self.wheel_settling, phase, Instant::now()) {
                    return;
                }
                let scrolls = self.open.as_ref().is_some_and(|o| o.mode() == Mode::Webtoon || (o.at.fit == Fit::Width && !o.spreads()));
                // Scrolling a page works without focus, as on any Mac
                // window; turning pages needs Leaf to be the active window.
                if scrolls && dy.abs() >= dx.abs() {
                    self.scroll_by(-dy);
                    return;
                }
                // After a turn, trackpad momentum mustn't flick through
                // several pages.
                if !self.focused || self.wheel_turned.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
                    return;
                }
                if dx.abs() > dy.abs() {
                    return self.swipe_by(dx);
                }
                // Fit-page: the wheel turns pages.
                self.wheel += dy;
                if self.wheel.abs() >= WHEEL_PAGE {
                    let forward = self.wheel < 0.0;
                    self.wheel = 0.0;
                    self.wheel_turned = Some(Instant::now());
                    self.step(forward);
                }
            }
            WindowEvent::PinchGesture { delta, phase, .. } => {
                if phase == TouchPhase::Started {
                    self.pinch = 0.0;
                }
                if delta.is_finite() {
                    self.pinch_by(delta);
                }
            }
            WindowEvent::DoubleTapGesture { .. } => self.run("toggle-magnifier"),
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                self.wheel = 0.0;
                self.swipe = 0.0;
                if focused {
                    self.wheel_settling = Some(Instant::now());
                    // Edited keys.toml or settings.toml and came back: use them.
                    self.reload_config();
                } else {
                    self.remember();
                    state::save(&self.state);
                }
            }
            _ => {}
        }
    }
}

/// Whether a scroll event belongs to scrolling that was already going when
/// Leaf opened or came to the front (see `wheel_settling`). Such scrolling
/// is ignored until it pauses; fingers put down afresh count at once.
fn settling(since: &mut Option<Instant>, phase: TouchPhase, now: Instant) -> bool {
    const PAUSE: Duration = Duration::from_millis(250);
    let Some(last) = *since else { return false };
    if phase == TouchPhase::Started || now.saturating_duration_since(last) >= PAUSE {
        *since = None;
        return false;
    }
    *since = Some(now);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrolling_from_before_must_pause() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        // A stream every 16 ms that began before Leaf opened stays ignored...
        let mut since = Some(t0);
        for n in (16..2000).step_by(16) {
            assert!(settling(&mut since, TouchPhase::Moved, ms(n)));
        }
        // ...until it pauses, and then scrolling works.
        assert!(!settling(&mut since, TouchPhase::Moved, ms(2300)));
        assert!(!settling(&mut since, TouchPhase::Moved, ms(2310)));
        // Fingers put down on the trackpad start afresh at once.
        let mut since = Some(t0);
        assert!(settling(&mut since, TouchPhase::Moved, ms(100)));
        assert!(!settling(&mut since, TouchPhase::Started, ms(116)));
        assert!(!settling(&mut since, TouchPhase::Moved, ms(132)));
        // A first scroll a moment after opening isn't held back.
        let mut since = Some(t0);
        assert!(!settling(&mut since, TouchPhase::Moved, ms(400)));
    }

    fn book_of(n: usize) -> Open {
        let dir = std::env::temp_dir().join(format!("leaf-layout-{n}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..n {
            std::fs::write(dir.join(format!("{i:03}.png")), b"x").unwrap();
        }
        let (book, _) = Book::open(&dir).unwrap();
        Open {
            id: 1,
            book: Arc::new(book),
            key: String::new(),
            at: BookState { mode: Some(Mode::Manga), ..BookState::default() },
            cache: HashMap::new(),
            wanted: HashMap::new(),
            stale: HashMap::new(),
            info: HashMap::new(),
        }
    }

    #[test]
    fn spreads_pair_pages_after_the_cover() {
        let mut o = book_of(6);
        assert_eq!(o.spread(0), (0, None));
        assert_eq!(o.spread(2), (1, Some(2)));
        assert_eq!(o.spread(5), (5, None));
        o.at.page = 1;
        assert_eq!(o.next_spread(), Some(3));
        assert_eq!(o.prev_spread(), Some(0));

        // A wide page (a spread printed as one image) stands alone and the
        // pairing starts again after it.
        o.info.insert(2, PageInfo { w: 2000, h: 1400, gray: true });
        assert_eq!(o.spread(1), (1, None));
        assert_eq!(o.spread(2), (2, None));
        assert_eq!(o.spread(4), (3, Some(4)));

        o.at.pair_cover = true;
        o.info.clear();
        assert_eq!(o.spread(1), (0, Some(1)));

        o.at.mode = Some(Mode::Comic);
        assert_eq!(o.spread(2), (2, None));
        o.at.spreads = Some(true);
        assert_eq!(o.spread(3), (2, Some(3)));
    }

    #[test]
    fn samples_skip_the_cover() {
        assert_eq!(sample_pages(1), [0]);
        assert_eq!(sample_pages(3), [1, 2]);
        assert_eq!(sample_pages(50), [1, 2, 3, 4]);
    }

    #[test]
    fn the_next_book_is_the_next_volume() {
        let dir = std::env::temp_dir().join(format!("leaf-series-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("Extras")).unwrap();
        for f in ["Vol 10.cbz", "Vol 2.cbz", "Vol 1.cbz", "notes.txt", ".hidden.cbz", "cover.jpg"] {
            std::fs::write(dir.join(f), b"x").unwrap();
        }
        let names = |v: Vec<PathBuf>| v.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect::<Vec<_>>();
        assert_eq!(names(neighbours(&dir.join("Vol 2.cbz"), true)), ["Vol 10.cbz"]);
        assert_eq!(names(neighbours(&dir.join("Vol 2.cbz"), false)), ["Vol 1.cbz", "Extras"]);
        // A text book's neighbours include other text books.
        assert_eq!(names(neighbours(&dir.join("notes.txt"), true)), ["Vol 1.cbz", "Vol 2.cbz", "Vol 10.cbz"]);
        assert!(neighbours(&dir.join("Vol 10.cbz"), true).is_empty());
    }
}
