//! Leaf: a bare comic and manga reader. Opens straight back to the page you
//! were on. All state lives in `App` on the UI thread; pages are decoded on
//! one background thread (loader.rs).

mod book;
mod keys;
mod loader;
mod platform;
mod state;

use book::Book;
use keys::Keys;
use loader::{Done, Loader, Scaled, Target};
use softbuffer::{Context, Surface};
use state::{BookState, Fit, State};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::path::PathBuf;
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
const PROGRESS: u32 = 0x5b6cff;
const BROKEN: u32 = 0x3a2224;
/// The pages one key press away, decoded ahead so they turn instantly.
const PREFETCH: [isize; 3] = [0, 1, -1];
/// Wheel distance, in pixels, that turns a page in fit-page mode.
const WHEEL_PAGE: f64 = 120.0;

fn main() {
    let event_loop = EventLoop::<Done>::with_user_event().build().expect("create the event loop");
    event_loop.set_control_flow(ControlFlow::Wait);
    let mut app = App::new(event_loop.create_proxy());
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("leaf: {e}");
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
    state: State,
    keys: Keys,
    loader: Loader,
    window: Option<Arc<Window>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    open: Option<Open>,
    next_id: u64,
    problem: Option<String>,
    modifiers: ModifiersState,
    cursor: PhysicalPosition<f64>,
    wheel: f64,
    wheel_turned: Option<Instant>,
    started: Instant,
}

impl App {
    fn new(proxy: EventLoopProxy<Done>) -> Self {
        let state = state::load();
        let (keys, problem) = Keys::load();
        let pending = std::env::args_os().nth(1).map(PathBuf::from).or_else(|| state.last.clone());
        let mut app = App {
            state,
            keys,
            loader: Loader::new(proxy),
            window: None,
            surface: None,
            open: None,
            next_id: 0,
            problem,
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
        match Book::open(&path) {
            Ok((book, start)) => {
                let key = book.path.to_string_lossy().into_owned();
                let mut at = self.state.books.get(&key).cloned().unwrap_or_default();
                if let Some(start) = start {
                    at.page = start;
                    at.scroll = 0.0;
                }
                at.page = at.page.min(book.pages.len() - 1);
                self.state.last = Some(book.path.clone());
                self.next_id += 1;
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
            }
            Err(e) => {
                self.problem = Some(e);
                self.update_title();
                self.redraw();
            }
        }
    }

    /// After any change of page or setting: remember it, ask for pages, redraw.
    fn moved(&mut self) {
        self.remember();
        state::save(&self.state);
        self.update_title();
        self.request_pages();
        self.redraw();
    }

    fn remember(&mut self) {
        if let Some(o) = &self.open {
            self.state.books.insert(o.key.clone(), o.at.clone());
        }
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
            None => "Leaf — drop a book here, or press ⌘O".replace('⌘', if cfg!(target_os = "macos") { "⌘" } else { "Ctrl+" }),
        };
        if let Some(problem) = &self.problem {
            title.push_str(&format!("  ·  {problem}"));
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

    fn run(&mut self, command: &str) {
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
            _ => {}
        }
    }

    fn draw(&mut self) {
        let Some(window) = self.window.clone() else { return };
        let size = window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        let scroll_range = self.scroll_range();
        let current = self.current().cloned();
        let open = self.open.as_ref();
        let Some(surface) = &mut self.surface else { return };
        if surface.resize(w, h).is_err() {
            return;
        }
        let Ok(mut buf) = surface.buffer_mut() else { return };
        let (w, h) = (size.width as i64, size.height as i64);
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
            let thick = (window.scale_factor() * 2.0).round().max(1.0) as i64;
            for y in (h - thick).max(0)..h {
                let row = (y * w) as usize;
                let (a, b) = if o.at.rtl { (w - filled, w) } else { (0, filled) };
                buf[row + a as usize..row + b as usize].fill(PROGRESS);
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
        event_loop.exit();
    }
}

impl ApplicationHandler<Done> for App {
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
        self.update_title();
        self.request_pages();
        self.redraw();
    }

    fn user_event(&mut self, _: &ActiveEventLoop, done: Done) {
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

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.quit(event_loop),
            WindowEvent::Resized(_) => {
                self.note_window();
                self.request_pages();
                self.redraw();
            }
            WindowEvent::RedrawRequested => self.draw(),
            WindowEvent::DroppedFile(path) => self.open_path(path),
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
            WindowEvent::Focused(false) => {
                self.remember();
                state::save(&self.state);
            }
            _ => {}
        }
    }
}
