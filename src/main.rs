//! Leaf: a bare comic and manga reader. Opens straight back to the page you
//! were on. All state lives in `App` on the UI thread; pages are decoded on
//! one background thread (loader.rs), adapters run as separate programs
//! (adapters.rs), and everything reaches the UI as an [`Event`].

mod adapters;
mod book;
mod incoming;
mod keys;
mod loader;
mod log;
mod menu;
mod platform;
mod state;

use adapters::{BOOK_CONTROL, BOOK_READ, Host};
use book::Book;
use keys::Keys;
use loader::{Done, Loader, Scaled, Target};
use serde_json::{Value, json};
use softbuffer::{Context, Surface};
use state::{BookState, Fit, State};
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
/// Leaf green: the progress line, the drop outline and the icon.
const ACCENT: u32 = 0x4fbf73;
const BROKEN: u32 = 0x3a2224;
/// The pages one key press away, decoded ahead so they turn instantly.
const PREFETCH: [isize; 3] = [0, 1, -1];
/// Wheel distance, in pixels, that turns a page in fit-page mode.
const WHEEL_PAGE: f64 = 120.0;
/// How long a message from an adapter (or Leaf) stays in the title bar.
const NOTICE: Duration = Duration::from_secs(4);
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
    target: Target,
    cache: HashMap<usize, Result<Arc<Scaled>, String>>,
    /// The current page at the previous window size, drawn until the
    /// re-scaled one arrives so a resize never flashes empty.
    stale: Option<(usize, Arc<Scaled>)>,
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
        let opened = match self.host.reader_for(&path) {
            Some(i) if Book::needs_adapter(&path) => {
                self.host.start(i).and_then(|link| Book::through_adapter(&path, link)).map(|b| (b, None))
            }
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
                    target: Target::default(),
                    cache: HashMap::new(),
                    stale: None,
                });
                self.problem = None;
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

    fn install_adapter(&mut self, path: &Path) {
        let manifest = match adapters::package_manifest(path) {
            Ok(m) => m,
            Err(e) => return self.say(e),
        };
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
            if let Some(m) = &self.menu {
                m.sync(true, o.at.rtl, o.at.fit == Fit::Width);
            }
        }
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
        let content = menu::Content {
            keys: &self.keys,
            adapters: &self.host.all,
            recent: &self.state.recent,
            has_book: self.open.is_some(),
            rtl: self.open.as_ref().is_some_and(|o| o.at.rtl),
            fit_width: self.open.as_ref().is_some_and(|o| o.at.fit == Fit::Width),
        };
        self.menu = Some(menu::MenuBar::install(&content, move |id| drop(proxy.send_event(Event::Menu(id)))));
    }

    fn update_title(&self) {
        let Some(window) = &self.window else { return };
        let mut title = match &self.open {
            Some(o) => {
                let mut t = format!("{} — {} / {}", o.book.name, o.at.page + 1, o.book.pages.len());
                if o.at.rtl {
                    t.push_str("  ·  right to left");
                }
                if let Some(Err(e)) = o.cache.get(&o.at.page) {
                    t.push_str(&format!("  ·  {e}"));
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
    fn target(&self) -> Target {
        let fit = self.open.as_ref().map(|o| o.at.fit).unwrap_or_default();
        let size = match &self.window {
            Some(w) => w.inner_size(),
            None => LogicalSize::<f64>::from(self.window_size()).to_physical(self.state.scale.unwrap_or(1.0)),
        };
        Target { w: size.width, h: size.height, fit }
    }

    fn window_size(&self) -> (f64, f64) {
        self.state.window.unwrap_or((820.0, 1100.0))
    }

    fn request_pages(&mut self) {
        let target = self.target();
        let Some(o) = &mut self.open else { return };
        if target.w == 0 || target.h == 0 {
            return;
        }
        if o.target != target {
            if let Some(Ok(img)) = o.cache.remove(&o.at.page) {
                o.stale = Some((o.at.page, img));
            }
            o.cache.clear();
            o.target = target;
        }
        let page = o.at.page;
        let n = o.book.pages.len() as isize;
        let wanted = PREFETCH
            .iter()
            .map(|d| page as isize + d)
            .filter(|&p| (0..n).contains(&p) && !o.cache.contains_key(&(p as usize)))
            .map(|p| p as usize)
            .collect();
        o.cache.retain(|&p, _| p.abs_diff(page) <= 1);
        self.loader.want(o.id, &o.book, target, wanted);
    }

    fn redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn current(&self) -> Option<&Arc<Scaled>> {
        let o = self.open.as_ref()?;
        match o.cache.get(&o.at.page) {
            Some(Ok(img)) => Some(img),
            _ => o.stale.as_ref().filter(|(p, _)| *p == o.at.page).map(|(_, img)| img),
        }
    }

    /// How many pixels the current page can scroll, in fit-width mode.
    fn scroll_range(&self) -> f64 {
        let h = self.target().h as f64;
        self.current().map(|img| (img.h as f64 - h).max(0.0)).unwrap_or(0.0)
    }

    fn go(&mut self, page: usize, scroll: f32) {
        let Some(o) = &mut self.open else { return };
        let page = page.min(o.book.pages.len() - 1);
        if page == o.at.page && scroll == o.at.scroll {
            return;
        }
        o.at.page = page;
        o.at.scroll = scroll;
        self.moved();
    }

    /// Scrolls by `dy` pixels; returns false when already at that end.
    fn scroll_by(&mut self, dy: f64) -> bool {
        let range = self.scroll_range();
        let Some(o) = &mut self.open else { return false };
        if o.at.fit != Fit::Width || range <= 0.0 {
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

    fn step(&mut self, forward: bool) {
        let Some(o) = &self.open else { return };
        let (page, last) = (o.at.page, o.book.pages.len() - 1);
        let screen = self.target().h as f64 * 0.85;
        if self.scroll_by(if forward { screen } else { -screen }) {
            return;
        }
        if forward && page < last {
            self.go(page + 1, 0.0);
        } else if !forward && page > 0 {
            self.go(page - 1, 1.0);
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
        if let Some(i) = command.strip_prefix("open-recent ").and_then(|i| i.parse::<usize>().ok()) {
            if let Some(path) = self.state.recent.get(i).cloned() {
                self.open_path(path);
            }
            return;
        }
        let rtl = self.open.as_ref().is_some_and(|o| o.at.rtl);
        match command {
            "next" => self.step(true),
            "prev" => self.step(false),
            "page-right" | "page-left" => {
                let forward = (command == "page-right") != rtl;
                let Some(o) = &self.open else { return };
                let page = o.at.page;
                if forward {
                    self.go(page + 1, 0.0);
                } else if page > 0 {
                    self.go(page - 1, 0.0);
                }
            }
            "first" => self.go(0, 0.0),
            "last" => self.go(usize::MAX, 0.0),
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
                let picked = rfd::FileDialog::new()
                    .add_filter("Books and images", &["cbz", "zip", "epub", "jpg", "jpeg", "png", "webp", "gif", "bmp"])
                    .add_filter("Any file", &["*"])
                    .pick_file();
                if let Some(path) = picked {
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
            "edit-keys" => platform::open(&state::dir().join("keys.toml"), true),
            "help" => platform::open_url(HELP_URL),
            _ => {}
        }
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
                self.say(format!("{adapter}: {}", params["text"].as_str().unwrap_or_default()));
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

    fn draw(&mut self) {
        let Some(window) = self.window.clone() else { return };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        let scroll_range = self.scroll_range();
        let current = self.current().cloned();
        let hovering = self.hovering;
        let open = self.open.as_ref();
        let Some(surface) = &mut self.surface else { return };
        if surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buf) = surface.buffer_mut() else { return };
        let (w, h) = (size.width as i64, size.height as i64);
        let thick = (window.scale_factor() * 2.0).round().max(1.0) as i64;
        buf.fill(BACKGROUND);

        if let Some(img) = &current {
            let (iw, ih) = (img.w as i64, img.h as i64);
            let x0 = (w - iw) / 2;
            let y0 = match open.map(|o| o.at.fit) {
                Some(Fit::Width) if ih > h => -(open.unwrap().at.scroll as f64 * scroll_range).round() as i64,
                _ => (h - ih) / 2,
            };
            let (cx0, cx1) = (x0.max(0), (x0 + iw).min(w));
            for y in y0.max(0)..(y0 + ih).min(h) {
                let src = ((y - y0) * iw + (cx0 - x0)) as usize;
                let dst = (y * w + cx0) as usize;
                let len = (cx1 - cx0) as usize;
                buf[dst..dst + len].copy_from_slice(&img.px[src..src + len]);
            }
        } else if let Some(Some(Err(_))) = open.map(|o| o.cache.get(&o.at.page)) {
            // A page that couldn't be decoded: a dim card where it would be.
            let (bw, bh) = (w * 2 / 3, h * 2 / 3);
            for y in (h - bh) / 2..(h + bh) / 2 {
                let row = (y * w + (w - bw) / 2) as usize;
                buf[row..row + bw as usize].fill(BROKEN);
            }
        }

        // Reading progress: a thin line along the bottom, from the side you start on.
        if let Some(o) = open {
            let n = o.book.pages.len() as i64;
            let filled = w * (o.at.page as i64 + 1) / n;
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
            Event::Page(done) => {
                let Some(o) = &mut self.open else { return };
                if done.book != o.id || done.target != o.target {
                    return;
                }
                let is_current = done.page == o.at.page;
                if is_current && std::env::var_os("LEAF_TIMING").is_some() {
                    eprintln!("leaf: page {} on screen {:?} after start", done.page + 1, self.started.elapsed());
                }
                if is_current && done.image.is_ok() {
                    o.stale = None;
                }
                o.cache.insert(done.page, done.image);
                if is_current {
                    self.update_title();
                    self.redraw();
                }
            }
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
                let w = self.target().w as f64;
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
                if self.open.as_ref().is_some_and(|o| o.at.fit == Fit::Width) {
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
