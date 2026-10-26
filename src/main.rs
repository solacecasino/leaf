//! Leaf: a bare comic and manga reader. Opens straight back to the page you
//! were on. All state lives in `App` on the UI thread; pages are decoded on
//! one background thread (loader.rs), adapters run as separate programs
//! (adapters.rs), and everything reaches the UI as an [`Event`].

mod adapters;
#[cfg(target_os = "macos")]
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
mod state;

use adapters::{BOOK_CONTROL, BOOK_READ, Host};
use book::Book;
use keys::Keys;
use loader::{Done, Loader, PageInfo, Scaled, Target};
use serde_json::{Value, json};
use softbuffer::{Context, Surface};
use state::{BookState, Fit, Mode, State};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Fullscreen, Window, WindowId};

#[global_allocator]
static ALLOCATOR: platform::Allocator = platform::Allocator;

const BACKGROUND: u32 = 0x1b1b1d;
/// A webtoon page that hasn't arrived yet.
const PLACEHOLDER: u32 = 0x242427;
/// Leaf green: the progress line, the drop outline and the icon.
const ACCENT: u32 = 0x4fbf73;
const BROKEN: u32 = 0x3a2224;
/// Wheel distance, in pixels, that turns a page in fit-page mode.
const WHEEL_PAGE: f64 = 120.0;
/// The widest a webtoon strip is drawn, in points: they're made for phones,
/// and blown up to a whole screen they're blurry and need endless scrolling.
const WEBTOON_WIDTH: f64 = 800.0;
/// Pages looked at to tell manga, comics and webtoons apart (after the cover).
const SAMPLE_PAGES: usize = 4;
/// How long a message from an adapter (or Leaf) stays in the title bar.
const NOTICE: Duration = Duration::from_secs(5);
const HELP_URL: &str = "https://github.com/solacecasino/leaf#readme";

/// Everything that wakes the UI thread.
pub enum Event {
    Page(Done),
    /// Finder asked Leaf to open something (see incoming.rs).
    Incoming,
    Menu(String),
    Adapter(adapters::Message),
}

fn main() {
    log::init();
    log::catch_crashes();
    let event_loop = EventLoop::<Event>::with_user_event().build().expect("create the event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let wake = proxy.clone();
    incoming::listen_early(move || drop(wake.send_event(Event::Incoming)));
    let mut app = App::new(proxy);
    if let Err(e) = event_loop.run_app(&mut app) {
        log::push("leaf", e.to_string());
    }
}

struct Open {
    id: u64,
    book: Arc<Book>,
    key: String,
    at: BookState,
    /// Each page shown or prepared, with the size it was made for.
    cache: HashMap<usize, (Target, Result<Arc<Scaled>, String>)>,
    /// Pages at their previous size, drawn until the re-scaled ones arrive
    /// so a resize never flashes empty.
    stale: HashMap<usize, Arc<Scaled>>,
    /// What's known about each page so far.
    info: HashMap<usize, PageInfo>,
}

impl Open {
    fn mode(&self) -> Mode {
        self.at.mode.unwrap_or(Mode::Comic)
    }

    fn last(&self) -> usize {
        self.book.pages.len() - 1
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
        let n = self.book.pages.len();
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

    fn image(&self, page: usize) -> Option<&Arc<Scaled>> {
        match self.cache.get(&page) {
            Some((_, Ok(img))) => Some(img),
            _ => self.stale.get(&page),
        }
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
    problem: Option<String>,
    notice: Option<(String, Instant)>,
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
    wheel_turned: Option<Instant>,
    started: Instant,
}

impl App {
    fn new(proxy: EventLoopProxy<Event>) -> Self {
        let state = state::load();
        let pages = proxy.clone();
        let calls = proxy.clone();
        let host = Host::new(move |m| drop(calls.send_event(Event::Adapter(m))));
        let (keys, problem) = Keys::load(&adapter_keys(&host));
        // macOS once passed a "-psn_…" argument to apps opened from Finder.
        let arg = std::env::args_os().nth(1).filter(|a| !a.to_string_lossy().starts_with("-psn"));
        let pending = arg.map(PathBuf::from).or_else(|| state.last.clone());
        let mut app = App {
            proxy,
            state,
            keys,
            loader: Loader::new(move |done| pages.send_event(Event::Page(done)).is_ok()),
            host,
            menu: None,
            window: None,
            surface: None,
            open: None,
            next_id: 0,
            problem,
            notice: None,
            hovering: false,
            last_drop: None,
            focused: true,
            quitting: false,
            modifiers: ModifiersState::empty(),
            cursor: PhysicalPosition::default(),
            wheel: 0.0,
            wheel_turned: None,
            started: Instant::now(),
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
        app
    }

    fn open_path(&mut self, path: PathBuf) {
        if adapters::is_package(&path) {
            self.install_adapter(&path);
            return;
        }
        // An adapter that says it opens this kind of file was installed on
        // purpose, so it comes first.
        let opened = match self.host.reader_for(&path) {
            Some(i) if path.is_file() => self.host.start(i).and_then(|link| Book::through_adapter(&path, link)).map(|b| (b, None)),
            _ => Book::open(&path),
        };
        match opened {
            Ok((book, start)) => {
                let key = book.path.to_string_lossy().into_owned();
                let mut at = self.state.books.get(&key).cloned().unwrap_or_default();
                if let Some(start) = start {
                    at.page = start;
                    at.scroll = 0.0;
                }
                at.page = at.page.min(book.pages.len() - 1);
                self.state.opened(&book.path);
                self.next_id += 1;
                self.host.emit("opened", json!({ "path": book.path, "name": book.name, "pages": book.pages.len() }));
                self.open = Some(Open {
                    id: self.next_id,
                    book: Arc::new(book),
                    key,
                    at,
                    cache: HashMap::new(),
                    stale: HashMap::new(),
                    info: HashMap::new(),
                });
                self.problem = None;
                self.detect_from_file();
                self.moved();
                self.rebuild_menu();
            }
            Err(e) => {
                log::push("leaf", &e);
                self.problem = Some(e);
                self.update_title();
                self.redraw();
            }
        }
    }

    /// Decides how to read a new book from what the file says, if it says:
    /// text is read a page at a time, ComicInfo.xml can say manga, and so
    /// can a name. Otherwise Leaf waits to look at a few pages.
    fn detect_from_file(&mut self) {
        let Some(o) = &self.open else { return };
        if o.at.mode.is_some() {
            return;
        }
        let name = o.book.path.to_string_lossy().to_lowercase();
        let japanese = name.chars().any(|c| matches!(c, '\u{3040}'..='\u{30ff}'));
        let mode = if o.book.hints.text {
            Some(Mode::Comic)
        } else if ["webtoon", "manhwa", "manhua"].iter().any(|w| name.contains(w)) {
            Some(Mode::Webtoon)
        } else if o.book.hints.manga || name.contains("manga") || japanese {
            Some(Mode::Manga)
        } else if o.book.hints.pdf {
            Some(Mode::Comic)
        } else {
            None
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
        let sample = sample_pages(o.book.pages.len());
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
        let text = format!("Remove {name}? It goes to the Trash, so you can still get it back.");
        if !platform::confirm("Remove adapter?", &text, "Remove", "Cancel") {
            return;
        }
        let dir = self.host.all[i].dir.clone();
        self.host.stop_all();
        match platform::move_to_trash(&dir) {
            Ok(()) => {
                adapters::forget_approval(name);
                self.say(format!("Moved {name} to the Trash"));
            }
            Err(e) => self.say(e),
        }
        self.reload_adapters();
    }

    fn reload_adapters(&mut self) {
        // A book read through an adapter is reopened on the restarted one.
        let reopen = self.open.as_ref().filter(|o| o.book.adapter().is_some()).map(|o| o.book.path.clone());
        self.remember();
        self.host.reload();
        let (keys, problem) = Keys::load(&adapter_keys(&self.host));
        self.keys = keys;
        self.problem = problem;
        self.host.start_launch_adapters();
        match reopen {
            Some(path) => self.open_path(path),
            None => self.rebuild_menu(),
        }
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
            self.host.emit("page", json!({ "page": o.at.page, "pages": o.book.pages.len() }));
        }
        self.sync_menu();
    }

    fn remember(&mut self) {
        if let Some(o) = &self.open {
            self.state.books.insert(o.key.clone(), o.at.clone());
        }
    }

    fn rebuild_menu(&mut self) {
        if self.window.is_none() {
            return;
        }
        let proxy = self.proxy.clone();
        let content = menu::Content { keys: &self.keys, adapters: &self.host.all, recent: &self.state.recent, view: self.view() };
        self.menu = Some(menu::MenuBar::install(&content, move |id| drop(proxy.send_event(Event::Menu(id)))));
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
            },
            None => menu::View::default(),
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
                let mut t = format!("{} — {pages} / {}", o.book.name, o.book.pages.len());
                match (o.mode(), o.at.rtl) {
                    (Mode::Manga, true) => t.push_str("  ·  manga"),
                    (Mode::Manga, false) => t.push_str("  ·  manga, left to right"),
                    (Mode::Webtoon, _) => t.push_str("  ·  webtoon"),
                    (Mode::Comic, true) => t.push_str("  ·  right to left"),
                    (Mode::Comic, false) => {}
                }
                for p in shown {
                    if let Some((_, Err(e))) = o.cache.get(&p) {
                        t.push_str(&format!("  ·  {e}"));
                    }
                }
                t
            }
            None if cfg!(target_os = "macos") => "Leaf — drop a book here, or press ⌘O".into(),
            None => "Leaf — drop a book here, or press Ctrl+O".into(),
        };
        if let Some(problem) = &self.problem {
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

    /// The size page `page` is prepared at in the current layout.
    fn target_for(&self, o: &Open, page: usize) -> Target {
        let (w, h) = self.window_px();
        match o.mode() {
            Mode::Webtoon => {
                let max = (WEBTOON_WIDTH * self.scale_factor()) as u32;
                Target { w: w.min(max), h, fit: Fit::Width }
            }
            _ if o.spread(page).1.is_some() || (o.spreads() && o.spread(page.saturating_sub(1)).1 == Some(page)) => {
                Target { w: w / 2, h, fit: Fit::Page }
            }
            _ => Target { w, h, fit: if o.spreads() { Fit::Page } else { o.at.fit } },
        }
    }

    /// Asks for the pages in view first, then the ones a key press away, then
    /// (for a book not yet read) a few to look at.
    fn request_pages(&mut self) {
        let (w, h) = self.window_px();
        let Some(o) = &self.open else { return };
        if w == 0 || h == 0 {
            return;
        }
        let last = o.last();
        let mut pages: Vec<usize> = Vec::new();
        if o.mode() == Mode::Webtoon {
            // What covers the window and one more window below it; the page
            // above only while you're near the top of this one. Strips are
            // tall, so a fixed number of pages would hold far too much.
            let p = o.at.page;
            let offset = o.at.scroll as f64 * self.strip_height(o, p);
            let mut covered = -offset;
            let mut q = p;
            while q <= last && covered < 2.0 * h as f64 {
                pages.push(q);
                covered += self.strip_height(o, q);
                q += 1;
            }
            if offset < h as f64 / 2.0 {
                pages.extend(p.checked_sub(1));
            }
        } else {
            pages.extend(o.shown());
            if let Some(next) = o.next_spread() {
                let (a, b) = o.spread(next);
                pages.extend(std::iter::once(a).chain(b));
            }
            if let Some(prev) = o.prev_spread() {
                let (a, b) = o.spread(prev);
                pages.extend(std::iter::once(a).chain(b));
            }
        }
        if o.at.mode.is_none() {
            pages.extend(sample_pages(o.book.pages.len()).into_iter().filter(|p| !o.info.contains_key(p)));
        }
        let mut seen = std::collections::HashSet::new();
        pages.retain(|p| seen.insert(*p));

        let wanted: Vec<(usize, Target)> = pages.iter().map(|&p| (p, self.target_for(o, p))).collect();
        let Some(o) = &mut self.open else { return };
        // Pages now wanted at another size keep their old picture until the new one comes.
        for &(p, t) in &wanted {
            if let Some((old, Ok(img))) = o.cache.get(&p)
                && *old != t
            {
                o.stale.insert(p, img.clone());
            }
        }
        o.cache.retain(|p, (t, _)| wanted.contains(&(*p, *t)));
        o.stale.retain(|p, _| pages.contains(p));
        let missing = wanted.into_iter().filter(|(p, _)| !o.cache.contains_key(p)).collect();
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
        o.image(o.at.page).map(|img| (img.h as f64 - h as f64).max(0.0)).unwrap_or(0.0)
    }

    fn go(&mut self, page: usize, scroll: f32) {
        let Some(o) = &mut self.open else { return };
        let page = o.spread(page.min(o.last())).0;
        if page == o.at.page && scroll == o.at.scroll {
            return;
        }
        o.at.page = page;
        o.at.scroll = scroll;
        self.moved();
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

    /// A webtoon page's height as drawn: exact once it's arrived, otherwise
    /// from its proportions, otherwise a guess.
    fn strip_height(&self, o: &Open, page: usize) -> f64 {
        if let Some(img) = o.image(page) {
            return img.h as f64;
        }
        let w = self.target_for(o, page).w as f64;
        o.info.get(&page).map_or(w * 1.5, |i| w * i.h as f64 / i.w as f64).max(1.0)
    }

    /// Moves down (or up) the webtoon strip, page by page.
    fn scroll_strip(&mut self, dy: f64) -> bool {
        let (_, screen) = self.window_px();
        let Some(o) = &self.open else { return false };
        let last = o.last();
        let mut page = o.at.page;
        let mut offset = o.at.scroll as f64 * self.strip_height(o, page) + dy;
        loop {
            let h = self.strip_height(o, page);
            if offset < 0.0 && page > 0 {
                page -= 1;
                offset += self.strip_height(o, page);
            } else if offset >= h && page < last {
                offset -= h;
                page += 1;
            } else {
                break;
            }
        }
        let h = self.strip_height(o, page);
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
        if o.mode() == Mode::Webtoon {
            return;
        }
        if forward {
            if let Some(next) = o.next_spread() {
                self.go(next, 0.0);
            }
        } else if let Some(prev) = o.prev_spread() {
            self.go(prev, 1.0);
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
                let to = if forward { o.next_spread() } else { o.prev_spread() };
                if let Some(page) = to {
                    self.go(page, 0.0);
                }
            }
            "first" => self.go(0, 0.0),
            "last" => self.go(usize::MAX, 0.0),
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
            "help" => platform::open_url(HELP_URL),
            _ => {}
        }
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
                "Book: a {kind} of {} pages, {:?}{}{}",
                o.book.pages.len(),
                o.mode(),
                if o.spreads() { ", two pages" } else { "" },
                if o.at.rtl { ", right to left" } else { "" }
            ));
        }
        let adapters: Vec<String> = self.host.all.iter().map(|a| format!("{} {}", a.manifest.name, a.manifest.version)).collect();
        out.push(format!("Adapters: {}", if adapters.is_empty() { "none".into() } else { adapters.join(", ") }));
        if let Some(p) = &self.problem {
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
                    "pages": o.book.pages.len(), "rtl": o.at.rtl,
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

    /// Where each picture in view goes: (page, x, y, width, height) in
    /// window pixels. A webtoon page not here yet still gets a place.
    fn layout(&self) -> Vec<(usize, i64, i64, i64, i64)> {
        let (w, h) = self.window_px();
        let (w, h) = (w as i64, h as i64);
        let Some(o) = &self.open else { return Vec::new() };
        let size = |p: usize| o.image(p).map(|i| (i.w as i64, i.h as i64));
        match o.mode() {
            Mode::Webtoon => {
                let mut out = Vec::new();
                let mut page = o.at.page;
                let mut y = -(o.at.scroll as f64 * self.strip_height(o, page)).round() as i64;
                while y < h && page <= o.last() {
                    let ph = self.strip_height(o, page).round() as i64;
                    let pw = size(page).map_or(self.target_for(o, page).w as i64, |s| s.0);
                    out.push((page, (w - pw) / 2, y, pw, ph));
                    y += ph;
                    page += 1;
                }
                out
            }
            _ => match o.spread(o.at.page) {
                (a, Some(b)) => {
                    // Two pages meeting in the middle, the first on the right
                    // when reading right to left.
                    let (left, right) = if o.at.rtl { (b, a) } else { (a, b) };
                    let mut out = Vec::new();
                    if let Some((pw, ph)) = size(left) {
                        out.push((left, w / 2 - pw, (h - ph) / 2, pw, ph));
                    }
                    if let Some((pw, ph)) = size(right) {
                        out.push((right, w / 2, (h - ph) / 2, pw, ph));
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
                    vec![(a, (w - pw) / 2, y, pw, ph)]
                }
            },
        }
    }

    fn draw(&mut self) {
        let Some(window) = self.window.clone() else { return };
        let size = window.inner_size();
        let (Some(nw), Some(nh)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        let placed = self.layout();
        let hovering = self.hovering;
        let (w, h) = (size.width as i64, size.height as i64);
        let thick = (window.scale_factor() * 2.0).round().max(1.0) as i64;
        let Some(surface) = &mut self.surface else { return };
        if surface.resize(nw, nh).is_err() {
            return;
        }
        let Ok(mut buf) = surface.buffer_mut() else { return };
        buf.fill(BACKGROUND);
        let open = self.open.as_ref();

        for &(page, x0, y0, pw, ph) in &placed {
            let Some(o) = open else { break };
            let (cx0, cx1) = (x0.max(0), (x0 + pw).min(w));
            let (cy0, cy1) = (y0.max(0), (y0 + ph).min(h));
            if cx0 >= cx1 || cy0 >= cy1 {
                continue;
            }
            match o.image(page) {
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
                    let colour = if matches!(o.cache.get(&page), Some((_, Err(_)))) { BROKEN } else { PLACEHOLDER };
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
            && matches!(o.cache.get(&o.at.page), Some((_, Err(_))))
        {
            let (bw, bh) = (w * 2 / 3, h * 2 / 3);
            for y in (h - bh) / 2..(h + bh) / 2 {
                let row = (y * w + (w - bw) / 2) as usize;
                buf[row..row + bw as usize].fill(BROKEN);
            }
        }

        // Reading progress: a thin line along the bottom, from the side you start on.
        if let Some(o) = open {
            let n = o.book.pages.len() as f64;
            let done = match o.mode() {
                Mode::Webtoon => (o.at.page as f64 + o.at.scroll as f64) / n,
                _ => (o.shown().last().copied().unwrap_or(0) + 1) as f64 / n,
            };
            let filled = ((w as f64 * done).round() as i64).clamp(0, w);
            for y in (h - thick).max(0)..h {
                let row = (y * w) as usize;
                let (a, b) = if o.at.rtl { (w - filled, w) } else { (0, filled) };
                buf[row + a as usize..row + b as usize].fill(ACCENT);
            }
        }

        // While something is dragged over the window: an outline, so you can
        // see it'll be taken.
        if hovering {
            let edge = thick * 2;
            for y in 0..h {
                let row = (y * w) as usize;
                if y < edge || y >= h - edge {
                    buf[row..row + w as usize].fill(ACCENT);
                } else {
                    buf[row..row + edge as usize].fill(ACCENT);
                    buf[row + (w - edge) as usize..row + w as usize].fill(ACCENT);
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
        let Some(o) = &self.open else { return };
        if done.book != o.id {
            return;
        }
        let wanted = self.target_for(o, done.page);
        let o = self.open.as_mut().expect("checked");
        // A page's proportions can change the layout: a wide page stands alone.
        let new_info = done.info.filter(|i| o.info.get(&done.page) != Some(i));
        if let Some(info) = new_info {
            o.info.insert(done.page, info);
        }
        if done.target == wanted {
            let shown = o.mode() == Mode::Webtoon || o.shown().contains(&done.page);
            if shown && std::env::var_os("LEAF_TIMING").is_some() {
                eprintln!("leaf: page {} on screen {:?} after start", done.page + 1, self.started.elapsed());
            }
            if done.image.is_ok() {
                o.stale.remove(&done.page);
            }
            o.cache.insert(done.page, (done.target, done.image));
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
}

/// The pages looked at to decide how to read a book: a few after the cover.
fn sample_pages(n: usize) -> Vec<usize> {
    if n <= 1 { vec![0] } else { (1..n).take(SAMPLE_PAGES).collect() }
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
        let window = Arc::new(event_loop.create_window(attrs).expect("create the window"));
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
            Event::Page(done) => self.page_done(done),
            Event::Incoming => self.open_incoming(),
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
                let mods = self.modifiers;
                if let Some(cmd) = self.keys.command(&event.logical_key, mods).map(str::to_owned) {
                    self.run(&cmd);
                }
            }
            WindowEvent::CursorMoved { position, .. } => self.cursor = position,
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                // Click the left or right third of the window to turn that way.
                let w = self.window_px().0 as f64;
                if self.cursor.x < w / 3.0 {
                    self.run("page-left");
                } else if self.cursor.x > w * 2.0 / 3.0 {
                    self.run("page-right");
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y as f64 * 60.0,
                    MouseScrollDelta::PixelDelta(p) => p.y,
                };
                let scrolls = self.open.as_ref().is_some_and(|o| o.mode() == Mode::Webtoon || (o.at.fit == Fit::Width && !o.spreads()));
                if scrolls {
                    self.scroll_by(-dy);
                    return;
                }
                // Fit-page: wheel turns pages, with a pause so trackpad
                // momentum doesn't flick through several at once.
                if !self.focused {
                    return;
                }
                if self.wheel_turned.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
                    return;
                }
                self.wheel += dy;
                if self.wheel.abs() >= WHEEL_PAGE {
                    let forward = self.wheel < 0.0;
                    self.wheel = 0.0;
                    self.wheel_turned = Some(Instant::now());
                    self.step(forward);
                }
            }
            WindowEvent::Focused(focused) => {
                self.focused = focused;
                self.wheel = 0.0;
                if !focused {
                    self.remember();
                    state::save(&self.state);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
