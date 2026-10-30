//! Adapters: optional add-ons, each a separate program in any language that
//! talks to Leaf with JSON messages, one per line, over stdin and stdout.
//! Leaf ships with none. They live in the `adapters` folder of the config
//! folder, are approved by the user before they first run, and can crash
//! without taking Leaf down. The protocol is in docs/adapters.md.
//!
//! Two things an adapter can do:
//! - add **commands** (to the Adapters menu, or File, View and Go, and to
//!   `keys.toml`), and talk back to Leaf while they run;
//! - **open books** Leaf can't read itself (`opens = ["cbr", "pdf"]`): Leaf
//!   asks it for the page list and then for each page's image.

use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The protocol version sent in `initialize`. New versions only add things.
pub const PROTOCOL: u32 = 3;

pub const BOOK_READ: &str = "book.read";
pub const BOOK_CONTROL: &str = "book.control";
/// Sees the pages' pixels (for filters such as upscaling).
pub const BOOK_PAGES: &str = "book.pages";
pub const NETWORK: &str = "network";
const KNOWN_PERMISSIONS: [&str; 4] = [BOOK_READ, BOOK_CONTROL, BOOK_PAGES, NETWORK];

/// How long Leaf waits for a book-opening adapter to answer.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Deserialize, Clone, Debug)]
pub struct Manifest {
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub homepage: String,
    /// The program and its arguments, run from the adapter's folder.
    pub run: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    /// The 9000 sisters it works in; left out, any that installs it.
    #[serde(default)]
    pub apps: Vec<String>,
    #[serde(default)]
    pub start: Start,
    /// Events to receive while running: "opened", "page".
    #[serde(default)]
    pub events: Vec<String>,
    #[serde(default)]
    pub commands: Vec<CommandSpec>,
    /// File extensions this adapter can open as books.
    #[serde(default)]
    pub opens: Vec<String>,
    /// What it does to pages: "upscale" (protocol 3). Needs `book.pages`.
    #[serde(default)]
    pub filters: Vec<String>,
}

#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Start {
    /// When one of its commands is first chosen, or a book needs it.
    #[default]
    Command,
    /// When Leaf opens.
    Launch,
}

#[derive(Deserialize, Clone, Debug)]
pub struct CommandSpec {
    pub id: String,
    pub label: String,
    /// A default shortcut, such as "cmd+alt+k". keys.toml can override it.
    #[serde(default)]
    pub keys: Option<String>,
    /// "file", "view" or "go" to show it in that menu instead of Adapters.
    #[serde(default)]
    pub menu: Option<String>,
}

/// What the reader threads tell the UI.
pub enum Message {
    /// A notification or request from an adapter. Requests have an `id` and
    /// must be answered with [`Host::reply`].
    Call { adapter: String, id: Option<Value>, method: String, params: Value },
    Stopped { adapter: String, generation: u64 },
}

/// A running adapter's pipe. Shared with the page loader, which asks
/// book-opening adapters for pages from its own thread.
pub struct Link {
    name: String,
    stdin: Mutex<ChildStdin>,
    pending: Mutex<HashMap<u64, Sender<Result<Value, String>>>>,
    next_id: AtomicU64,
    alive: AtomicBool,
}

impl Link {
    pub fn name(&self) -> &str {
        &self.name
    }

    fn write(&self, message: Value) {
        let mut stdin = self.stdin.lock().unwrap_or_else(|e| e.into_inner());
        // An adapter that has quit just misses the message.
        let _ = writeln!(stdin, "{message}").and_then(|()| stdin.flush());
    }

    pub fn notify(&self, method: &str, params: Value) {
        self.write(json!({ "method": method, "params": params }));
    }

    /// Sends a request and waits for the answer.
    pub fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(format!("{} has stopped", self.name));
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let (tx, rx) = channel();
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).insert(id, tx);
        self.write(json!({ "id": id, "method": method, "params": params }));
        rx.recv_timeout(timeout).unwrap_or_else(|_| {
            self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id);
            Err(format!("{} didn't answer {method} in time", self.name))
        })
    }

    fn answered(&self, id: u64, result: Result<Value, String>) {
        if let Some(tx) = self.pending.lock().unwrap_or_else(|e| e.into_inner()).remove(&id) {
            let _ = tx.send(result);
        }
    }

    fn stopped(&self) {
        self.alive.store(false, Ordering::SeqCst);
        for (_, tx) in self.pending.lock().unwrap_or_else(|e| e.into_inner()).drain() {
            let _ = tx.send(Err(format!("{} stopped", self.name)));
        }
    }
}

struct Running {
    child: Child,
    link: Arc<Link>,
    generation: u64,
}

pub struct Adapter {
    pub dir: PathBuf,
    pub manifest: Manifest,
    running: Option<Running>,
}

type Deliver = Arc<dyn Fn(Message) + Send + Sync>;

/// All installed adapters, owned by the UI thread.
pub struct Host {
    pub all: Vec<Adapter>,
    send: Deliver,
    generation: u64,
}

impl Host {
    pub fn new(send: impl Fn(Message) + Send + Sync + 'static) -> Self {
        Host { all: discover(), send: Arc::new(send), generation: 0 }
    }

    /// Looks for adapters again, stopping the ones that were running.
    pub fn reload(&mut self) {
        self.stop_all();
        self.all = discover();
    }

    /// Starts the adapters that ask to start with Leaf.
    pub fn start_launch_adapters(&mut self) {
        for i in 0..self.all.len() {
            if self.all[i].manifest.start == Start::Launch
                && let Err(e) = self.start(i)
            {
                crate::log::push(crate::series::NAME, e);
            }
        }
    }

    pub fn index(&self, name: &str) -> Option<usize> {
        self.all.iter().position(|a| a.manifest.name == name)
    }

    pub fn has(&self, name: &str, permission: &str) -> bool {
        self.index(name).is_some_and(|i| self.all[i].manifest.permissions.iter().any(|p| p == permission))
    }

    /// The first adapter that upscales pages (and may see them).
    pub fn upscaler(&self) -> Option<usize> {
        self.all.iter().position(|a| a.manifest.filters.iter().any(|f| f == "upscale") && a.manifest.permissions.iter().any(|p| p == BOOK_PAGES))
    }

    /// The adapter that opens books like `path`, by extension.
    pub fn reader_for(&self, path: &Path) -> Option<usize> {
        let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
        self.all.iter().position(|a| a.manifest.opens.iter().any(|o| o.trim_start_matches('.').eq_ignore_ascii_case(&ext)))
    }

    /// Starts adapter `i` if it isn't running, asking the user first if it
    /// hasn't been allowed yet.
    pub fn start(&mut self, i: usize) -> Result<Arc<Link>, String> {
        if let Some(r) = &self.all[i].running
            && r.link.alive.load(Ordering::SeqCst)
        {
            return Ok(r.link.clone());
        }
        let manifest = self.all[i].manifest.clone();
        if !approve(&manifest) {
            return Err(format!("{} wasn't allowed to run", manifest.name));
        }
        let mut child = spawn(&self.all[i].dir, &manifest.run).map_err(|e| format!("Couldn't start {}: {e}", manifest.name))?;
        self.generation += 1;
        let generation = self.generation;
        let stdout = child.stdout.take().expect("stdout is piped");
        let stdin = child.stdin.take().expect("stdin is piped");
        let link = Arc::new(Link {
            name: manifest.name.clone(),
            stdin: Mutex::new(stdin),
            pending: Mutex::default(),
            next_id: AtomicU64::new(0),
            alive: AtomicBool::new(true),
        });

        if let Some(stderr) = child.stderr.take() {
            let name = manifest.name.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    crate::log::push(&name, line);
                }
            });
        }
        let (reader_link, send, name) = (link.clone(), self.send.clone(), manifest.name.clone());
        std::thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            loop {
                let line = match read_message(&mut stdout) {
                    Ok(Some(line)) => line,
                    Ok(None) => break,
                    Err(e) => {
                        crate::log::push(&name, e);
                        continue;
                    }
                };
                if line.trim().is_empty() {
                    continue;
                }
                let Ok(message) = serde_json::from_str::<Value>(&line) else {
                    crate::log::push(&name, format!("not a JSON message: {line}"));
                    continue;
                };
                match message.get("method").and_then(Value::as_str) {
                    Some(method) => send(Message::Call {
                        adapter: name.clone(),
                        id: message.get("id").cloned(),
                        method: method.to_owned(),
                        params: message.get("params").cloned().unwrap_or(Value::Null),
                    }),
                    None => {
                        let Some(id) = message.get("id").and_then(Value::as_u64) else { continue };
                        let result = match message.get("error") {
                            Some(e) => Err(e.as_str().map(String::from).unwrap_or_else(|| e.to_string())),
                            None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                        };
                        reader_link.answered(id, result);
                    }
                }
            }
            reader_link.stopped();
            send(Message::Stopped { adapter: name, generation });
        });

        link.notify(
            "initialize",
            // The 9000 series' family fields (app, version, "leaf9000",
            // protocol, family, permissions, sisters), and "leaf" for
            // adapters written for Leaf.
            {
                let mut p = core9000::family::initialize(&manifest.permissions, &serde_json::Map::new());
                p["leaf"] = json!(env!("CARGO_PKG_VERSION"));
                p
            },
        );
        crate::log::push(crate::series::NAME, format!("started {} {}", manifest.name, manifest.version));
        self.all[i].running = Some(Running { child, link: link.clone(), generation });
        Ok(link)
    }

    /// Runs an adapter's command, starting the adapter if needed.
    pub fn run_command(&mut self, name: &str, id: &str) -> Result<(), String> {
        let i = self.index(name).ok_or_else(|| format!("No adapter called {name}"))?;
        if !self.all[i].manifest.commands.iter().any(|c| c.id == id) {
            return Err(format!("{name} has no command {id}"));
        }
        self.start(i)?.notify("command", json!({ "command": id }));
        Ok(())
    }

    /// Sends an event to the running adapters that asked for it and may read the book.
    pub fn emit(&self, kind: &str, mut params: Value) {
        params["kind"] = json!(kind);
        for a in &self.all {
            let m = &a.manifest;
            if let Some(r) = &a.running
                && m.events.iter().any(|e| e == kind)
                && m.permissions.iter().any(|p| p == BOOK_READ)
            {
                r.link.notify("event", params.clone());
            }
        }
    }

    pub fn reply(&self, adapter: &str, id: Value, result: Result<Value, String>) {
        let Some(link) = self.index(adapter).and_then(|i| self.all[i].running.as_ref()).map(|r| &r.link) else { return };
        link.write(match result {
            Ok(value) => json!({ "id": id, "result": value }),
            Err(e) => json!({ "id": id, "error": e }),
        });
    }

    /// An adapter's program ended. Ignored (and false) if it has been
    /// stopped on purpose or restarted since.
    pub fn stopped(&mut self, adapter: &str, generation: u64) -> bool {
        let Some(i) = self.index(adapter) else { return false };
        if !self.all[i].running.as_ref().is_some_and(|r| r.generation == generation) {
            return false;
        }
        let mut r = self.all[i].running.take().expect("checked");
        let status = r.child.wait().map(|s| s.to_string()).unwrap_or_else(|e| e.to_string());
        crate::log::push(crate::series::NAME, format!("{adapter} stopped ({status})"));
        true
    }

    /// Asks every running adapter to shut down, then stops the ones that don't.
    pub fn stop_all(&mut self) {
        let mut children = Vec::new();
        for a in &mut self.all {
            if let Some(r) = a.running.take() {
                r.link.notify("shutdown", Value::Null);
                children.push(r.child);
            }
        }
        let deadline = Instant::now() + Duration::from_millis(500);
        for mut child in children {
            while Instant::now() < deadline && matches!(child.try_wait(), Ok(None)) {
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.stop_all();
    }
}

/// The longest message Leaf reads from an adapter: room for a big page as
/// base64, but not for one that would exhaust memory.
const MAX_MESSAGE: u64 = 96 << 20;

/// Reads one line, refusing (and skipping) one longer than MAX_MESSAGE.
/// None at the end of the output.
fn read_message(r: &mut impl BufRead) -> Result<Option<String>, String> {
    let mut buf = Vec::new();
    let n = r.by_ref().take(MAX_MESSAGE).read_until(b'\n', &mut buf).map_err(|e| e.to_string())?;
    if n == 0 {
        return Ok(None);
    }
    if buf.last() != Some(&b'\n') && n as u64 == MAX_MESSAGE {
        // Throw the rest of the line away, a piece at a time.
        loop {
            buf.clear();
            let n = r.by_ref().take(1 << 20).read_until(b'\n', &mut buf).map_err(|e| e.to_string())?;
            if n == 0 || buf.last() == Some(&b'\n') {
                break;
            }
        }
        return Err(format!("sent a message over {} MB; it was ignored", MAX_MESSAGE >> 20));
    }
    Ok(Some(String::from_utf8_lossy(&buf).into_owned()))
}

/// Refuses an adapter whose program was built for another kind of computer,
/// with a message saying so, instead of failing to start later.
pub fn check_platform(dir: &Path, manifest: &Manifest) -> Result<(), String> {
    let Some(program) = manifest.run.first().filter(|p| p.contains(['/', '\\'])) else {
        return Ok(()); // a script run by an interpreter, or a program in PATH
    };
    let path = dir.join(program);
    let mut magic = [0u8; 20];
    let n = std::fs::File::open(&path)
        .and_then(|mut f| f.read(&mut magic))
        .map_err(|_| format!("{} is missing its program ({program})", manifest.name))?;
    let magic = &magic[..n];
    let built_for = if magic.starts_with(b"MZ") {
        "Windows"
    } else if magic.starts_with(b"\x7fELF") {
        "Linux"
    } else if magic.starts_with(&[0xcf, 0xfa, 0xed, 0xfe]) || magic.starts_with(&[0xca, 0xfe, 0xba, 0xbe]) {
        // A thin Mach-O names its processor; Apple Silicon runs Intel ones too.
        let arm_only = magic.starts_with(&[0xcf, 0xfa, 0xed, 0xfe]) && magic.get(4..8) == Some(&[0x0c, 0x00, 0x00, 0x01]);
        if arm_only && cfg!(target_arch = "x86_64") { "Apple Silicon Macs" } else { "macOS" }
    } else {
        return Ok(()); // a script; let it try
    };
    let ours = if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(windows) {
        "Windows"
    } else {
        "Linux"
    };
    if built_for == ours {
        Ok(())
    } else {
        Err(format!("{} is built for {built_for}, not this computer. Look for its {ours} version.", manifest.name))
    }
}

/// Starts the program in the adapter's folder. A program given as a path
/// (`./unrar-pages`) is found there.
fn spawn(dir: &Path, run: &[String]) -> Result<Child, String> {
    let (program, args) = run.split_first().ok_or("adapter.toml has an empty `run`")?;
    let program = if program.contains(['/', '\\']) { dir.join(program).into_os_string() } else { program.into() };
    Command::new(&program)
        .args(args)
        .current_dir(dir)
        // So adapters can keep their own files next to Leaf's.
        .env("LEAF9000_CONFIG_DIR", crate::state::dir())
        // Adapters written for Leaf read it under its old name.
        .env("LEAF_CONFIG_DIR", crate::state::dir())
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => format!("{} not found", program.to_string_lossy()),
            _ => e.to_string(),
        })
}

// ---- Finding and installing ----------------------------------------------

pub fn adapters_dir() -> PathBuf {
    crate::state::dir().join("adapters")
}

/// Installed adapters. Leaf only looks in its config folder, so nothing
/// appears unless someone put it there.
pub fn discover() -> Vec<Adapter> {
    discover_in(&adapters_dir())
}

fn discover_in(root: &Path) -> Vec<Adapter> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.join("adapter.toml").is_file()).collect();
    dirs.sort();
    let mut found: Vec<Adapter> = Vec::new();
    for dir in dirs {
        match load_manifest(&dir).and_then(|m| check_platform(&dir, &m).map(|()| m)) {
            Ok(manifest) => {
                found.retain(|a| a.manifest.name != manifest.name);
                found.push(Adapter { dir, manifest, running: None });
            }
            Err(e) => crate::log::push(crate::series::NAME, e),
        }
    }
    found
}

fn load_manifest(dir: &Path) -> Result<Manifest, String> {
    let path = dir.join("adapter.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let manifest: Manifest = toml::from_str(&text).map_err(|e| format!("{}: {}", path.display(), e.message()))?;
    if manifest.run.is_empty() {
        return Err(format!("{}: `run` is empty", path.display()));
    }
    // An adapter made for other sisters says so (`apps`), and isn't run here.
    if !manifest.apps.is_empty() && !manifest.apps.iter().any(|a| a.eq_ignore_ascii_case(crate::series::NAME) || a.eq_ignore_ascii_case("leaf")) {
        return Err(format!("{} is an adapter for {}, not leaf9000.", manifest.name, manifest.apps.join(" and ")));
    }
    for p in &manifest.permissions {
        if !KNOWN_PERMISSIONS.contains(&p.as_str()) {
            crate::log::push(crate::series::NAME, format!("{}: unknown permission \"{p}\"", manifest.name));
        }
    }
    Ok(manifest)
}

/// Whether `path` is an adapter to install rather than a book: a folder
/// with `adapter.toml`, or a zip with one at its top or one folder down.
pub fn is_package(path: &Path) -> bool {
    if path.is_dir() {
        return path.join("adapter.toml").is_file();
    }
    zip_manifest(path).is_some()
}

fn zip_manifest(path: &Path) -> Option<String> {
    let file = std::fs::File::open(path).ok()?;
    let archive = zip::ZipArchive::new(std::io::BufReader::new(file)).ok()?;
    archive
        .file_names()
        .filter_map(Result::ok)
        .find(|n| n.trim_end_matches("adapter.toml").matches('/').count() <= 1 && n.rsplit('/').next() == Some("adapter.toml"))
        .map(|n| n.into_owned())
}

/// Reads the manifest of a package without installing it.
pub fn package_manifest(path: &Path) -> Result<Manifest, String> {
    if path.is_dir() {
        return load_manifest(path);
    }
    let name = zip_manifest(path).ok_or("not an adapter package")?;
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| e.to_string())?;
    let mut text = String::new();
    archive.by_name(&name).map_err(|e| e.to_string())?.read_to_string(&mut text).map_err(|e| e.to_string())?;
    toml::from_str(&text).map_err(|e| format!("adapter.toml: {}", e.message()))
}

/// Copies a package into the adapters folder, replacing an older copy of
/// the same adapter. The new copy is assembled beside it first, so a
/// failure leaves what was there.
pub fn install(source: &Path) -> Result<Manifest, String> {
    install_into(source, &adapters_dir())
}

fn install_into(source: &Path, root: &Path) -> Result<Manifest, String> {
    let manifest = package_manifest(source)?;
    let folder: String = manifest
        .name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '-' })
        .collect();
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let incoming = root.join(format!(".incoming-{folder}"));
    let _ = std::fs::remove_dir_all(&incoming);
    let result = if source.is_dir() { copy_dir(source, &incoming).map_err(|e| e.to_string()) } else { unzip(source, &incoming) };
    if let Err(e) = result.and_then(|()| check_platform(&incoming, &manifest)) {
        let _ = std::fs::remove_dir_all(&incoming);
        return Err(format!("Couldn't install {}: {e}", manifest.name));
    }
    let target = root.join(&folder);
    let _ = std::fs::remove_dir_all(&target);
    std::fs::rename(&incoming, &target).map_err(|e| e.to_string())?;
    Ok(manifest)
}

pub fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if [".git", "target", "dist"].iter().any(|skip| name == *skip) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            copy_dir(&path, &to.join(&name))?;
        } else {
            std::fs::copy(&path, to.join(&name))?;
        }
    }
    Ok(())
}

/// Unpacks a zip into `to`. A zip holding one folder is unpacked from inside
/// that folder. Paths that would escape `to` are refused.
fn unzip(zip_path: &Path, to: &Path) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| e.to_string())?;
    let manifest = zip_manifest(zip_path).ok_or("not an adapter package")?;
    let prefix = manifest.trim_end_matches("adapter.toml").to_owned();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(inside) = entry.enclosed_name() else { return Err(format!("unsafe path in zip: entry {i}")) };
        let Ok(rel) = inside.strip_prefix(&prefix) else { continue };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let out = to.join(rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        std::fs::write(&out, bytes).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode & 0o777));
        }
    }
    Ok(())
}

/// A running program wired up as an adapter, without a manifest or the
/// approval dialog: for tests of things that talk to adapters.
#[cfg(test)]
pub fn test_link(program: &Path) -> Arc<Link> {
    let mut child = Command::new(program).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn().expect("start the test adapter");
    let link = Arc::new(Link {
        name: program.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        stdin: Mutex::new(child.stdin.take().expect("piped")),
        pending: Mutex::default(),
        next_id: AtomicU64::new(0),
        alive: AtomicBool::new(true),
    });
    let (reader, stdout) = (link.clone(), child.stdout.take().expect("piped"));
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let Ok(m) = serde_json::from_str::<Value>(&line) else { continue };
            let Some(id) = m["id"].as_u64() else { continue };
            let result = match m.get("error") {
                Some(e) => Err(e.as_str().unwrap_or("error").to_owned()),
                None => Ok(m["result"].clone()),
            };
            reader.answered(id, result);
        }
        reader.stopped();
        drop(child);
    });
    link
}

// ---- Approval --------------------------------------------------------------

pub fn describe(permission: &str) -> String {
    match permission {
        BOOK_READ => "see which book is open and the page you're on".into(),
        BOOK_CONTROL => "open books and turn pages".into(),
        BOOK_PAGES => "see the pages you read (to change how they look)".into(),
        NETWORK => "connect to the internet or other computers".into(),
        other => other.to_owned(),
    }
}

fn approvals_path() -> PathBuf {
    crate::state::dir().join("approvals.json")
}

/// What `manifest` will be allowed to do, in words, for dialogs.
pub fn summary(m: &Manifest) -> String {
    let by = if m.author.is_empty() { String::new() } else { format!(" by {}", m.author) };
    let mut text = format!("“{}” {}{by}.", m.name, m.version);
    if !m.license.is_empty() {
        text += &format!(" License: {}.", m.license);
    }
    if !m.description.is_empty() {
        text += &format!("\n\n{}", m.description);
    }
    if !m.homepage.is_empty() {
        text += &format!("\n{}", m.homepage);
    }
    if !m.opens.is_empty() {
        text += &format!("\n\nIt opens: {}.", m.opens.join(", "));
    }
    if m.filters.iter().any(|f| f == "upscale") {
        text += "\n\nIt upscales pages shown bigger than they are.";
    }
    if m.permissions.is_empty() {
        text += "\n\nIt doesn't ask for any permissions.";
    } else {
        text += "\n\nIt asks to:";
        for p in &m.permissions {
            text += &format!("\n•  {}", describe(p));
        }
    }
    text + "\n\nAdapters are separate programs. Only allow ones you trust."
}

fn load_approvals() -> BTreeMap<String, Vec<String>> {
    std::fs::read_to_string(approvals_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

/// Records that the user allowed `m` with its current permissions. Installing
/// through Leaf's dialog counts, since it showed the same details.
pub fn remember_approval(m: &Manifest) {
    let mut approvals = load_approvals();
    approvals.insert(m.name.clone(), m.permissions.clone());
    let _ = std::fs::write(approvals_path(), serde_json::to_string_pretty(&approvals).expect("serializes"));
}

/// Forgets an approval, for a removed adapter: if it comes back, it's asked about again.
pub fn forget_approval(name: &str) {
    let mut approvals = load_approvals();
    if approvals.remove(name).is_some() {
        let _ = std::fs::write(approvals_path(), serde_json::to_string_pretty(&approvals).expect("serializes"));
    }
}

/// Asks the user with a native dialog, remembering a yes in approvals.json
/// until the adapter asks for more. Adapters copied into the folder by hand
/// are asked about here, the first time they run.
fn approve(m: &Manifest) -> bool {
    if load_approvals().get(&m.name).is_some_and(|granted| m.permissions.iter().all(|p| granted.contains(p))) {
        return true;
    }
    let allowed = crate::platform::confirm("Allow adapter?", &format!("{} wants to run.\n\n{}", m.name, summary(m)), "Allow", "Don't Allow");
    if allowed {
        remember_approval(m);
    }
    allowed
}

/// Reads `adapter "Name" id` from keys.toml or a menu.
pub fn parse_command(text: &str) -> Option<(String, String)> {
    let rest = text.strip_prefix("adapter ")?.trim_start().strip_prefix('"')?;
    let (name, id) = rest.split_once('"')?;
    let id = id.trim();
    (!name.is_empty() && !id.is_empty()).then(|| (name.to_owned(), id.to_owned()))
}

pub fn command_name(adapter: &str, id: &str) -> String {
    format!("adapter \"{adapter}\" {id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("leaf-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A shell-script adapter that answers `book.open` and `book.page`, and
    /// on its "hello" command asks Leaf for the book.
    fn write_adapter(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("adapter.toml"),
            r#"name = "Test Pages"
version = "0.1.0"
run = ["sh", "adapter.sh"]
permissions = ["book.read"]
opens = ["fake"]
[[commands]]
id = "hello"
label = "Say Hello"
keys = "cmd+alt+h"
"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("adapter.sh"),
            r#"while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  case "$line" in
    *'"book.open"'*) printf '{"id":%s,"result":{"pages":["a.png","b.png"]}}\n' "$id" ;;
    *'"book.page"'*) printf '{"id":%s,"result":{"base64":"aGk="}}\n' "$id" ;;
    *'"command"'*) printf '{"id":7,"method":"book.get","params":{}}\n' ;;
    *'"shutdown"'*) exit 0 ;;
  esac
  echo "got $line" >&2
done
"#,
        )
        .unwrap();
    }

    #[test]
    fn talks_both_ways() {
        let root = temp("talk");
        write_adapter(&root.join("test"));
        let found = discover_in(&root);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].manifest.commands[0].keys.as_deref(), Some("cmd+alt+h"));

        let (tx, rx) = channel();
        let tx = Mutex::new(tx);
        let mut host = Host { all: found, send: Arc::new(move |m| drop(tx.lock().unwrap().send(m))), generation: 0 };
        assert_eq!(host.reader_for(Path::new("/x/book.FAKE")), Some(0));
        // Start without the dialog: spawn directly.
        let link = {
            let mut child = spawn(&host.all[0].dir, &host.all[0].manifest.run).unwrap();
            let link = Arc::new(Link {
                name: "Test Pages".into(),
                stdin: Mutex::new(child.stdin.take().unwrap()),
                pending: Mutex::default(),
                next_id: AtomicU64::new(0),
                alive: AtomicBool::new(true),
            });
            let (reader, send) = (link.clone(), host.send.clone());
            let stdout = child.stdout.take().unwrap();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    let m: Value = serde_json::from_str(&line).unwrap();
                    match m.get("method").and_then(Value::as_str) {
                        Some(method) => send(Message::Call {
                            adapter: "Test Pages".into(),
                            id: m.get("id").cloned(),
                            method: method.into(),
                            params: Value::Null,
                        }),
                        None => reader.answered(m["id"].as_u64().unwrap(), Ok(m["result"].clone())),
                    }
                }
                reader.stopped();
            });
            host.all[0].running = Some(Running { child, link: link.clone(), generation: 1 });
            link
        };

        let pages = link.request("book.open", json!({ "path": "/x/book.fake" }), Duration::from_secs(10)).unwrap();
        assert_eq!(pages["pages"][1], "b.png");
        let page = link.request("book.page", json!({ "page": 0 }), Duration::from_secs(10)).unwrap();
        assert_eq!(page["base64"], "aGk=");

        host.run_command("Test Pages", "hello").unwrap();
        match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
            Message::Call { method, id, .. } => assert_eq!((method.as_str(), id), ("book.get", Some(json!(7)))),
            Message::Stopped { .. } => panic!("stopped early"),
        }
        assert!(host.run_command("Test Pages", "nope").is_err());
        host.stop_all();
        assert!(link.request("book.open", Value::Null, Duration::from_secs(1)).is_err());
    }

    #[test]
    fn installs_folders_and_zips() {
        let root = temp("install");
        let src = root.join("src");
        write_adapter(&src);
        let dest = root.join("adapters");
        let m = install_into(&src, &dest).unwrap();
        assert_eq!(m.name, "Test Pages");
        assert!(dest.join("test-pages/adapter.sh").is_file());

        // The same adapter zipped inside a folder, as release zips are.
        let zip_path = root.join("test-pages-0.1.0.zip");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let opts = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
        for name in ["adapter.toml", "adapter.sh"] {
            zip.start_file(format!("test-pages/{name}"), opts).unwrap();
            zip.write_all(&std::fs::read(src.join(name)).unwrap()).unwrap();
        }
        zip.finish().unwrap();
        assert!(is_package(&zip_path));
        install_into(&zip_path, &dest).unwrap();
        assert_eq!(discover_in(&dest).len(), 1);
        assert!(!is_package(&root));
    }

    /// A released adapter zip, installed the way a drop on Leaf's window
    /// installs it, then started as Leaf starts it and asked if it's ready.
    /// CI runs it on each system with that system's own zip
    /// (`LEAF_ADAPTER_ZIP`); without one there's nothing to test.
    #[test]
    fn installs_and_runs_a_released_zip() {
        let Some(zip) = crate::series::env("ADAPTER_ZIP").map(PathBuf::from) else { return };
        assert!(is_package(&zip), "{} isn't seen as an adapter", zip.display());
        let dest = temp("released").join("adapters");
        let m = install_into(&zip, &dest).unwrap();
        let found = discover_in(&dest);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].manifest.name, m.name);
        let host = Host { all: found, send: Arc::new(|_| {}), generation: 0 };
        let i = host.upscaler().expect("an upscaler with book.pages");

        let mut child = spawn(&host.all[i].dir, &host.all[i].manifest.run).unwrap();
        let mut stdin = child.stdin.take().unwrap();
        writeln!(stdin, r#"{{"id":1,"method":"upscale.ready"}}"#).unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        drop(stdin);
        let _ = child.wait();
        let answer: Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("{e}: {line:?}"));
        // A real GPU names itself; a software one is turned down, by name.
        let device = answer["result"]["device"].as_str();
        let refused = answer["error"].as_str().is_some_and(|e| e.contains("drawn by the CPU"));
        assert!(device.is_some() || refused, "{answer}");
        eprintln!("{} {} installed and answered: {answer}", m.name, m.version);
    }

    #[test]
    fn refuses_programs_for_other_computers() {
        let dir = temp("platform");
        let m = |run: &str| Manifest {
            name: "X".into(),
            version: String::new(),
            author: String::new(),
            license: String::new(),
            description: String::new(),
            homepage: String::new(),
            run: vec![run.into()],
            permissions: Vec::new(),
            apps: Vec::new(),
            start: Start::Command,
            events: Vec::new(),
            commands: Vec::new(),
            opens: Vec::new(),
            filters: Vec::new(),
        };
        std::fs::write(dir.join("win.exe"), b"MZ\x90\x00").unwrap();
        std::fs::write(dir.join("linux"), b"\x7fELF\x02\x01").unwrap();
        std::fs::write(dir.join("mac"), [0xca, 0xfe, 0xba, 0xbe, 0, 0, 0, 2]).unwrap();
        std::fs::write(dir.join("script.sh"), b"#!/bin/sh").unwrap();
        // Each runs only on its own system, and is refused by name elsewhere.
        for (file, system, ours) in [
            ("./win.exe", "Windows", cfg!(windows)),
            ("./linux", "Linux", cfg!(target_os = "linux")),
            ("./mac", "macOS", cfg!(target_os = "macos")),
        ] {
            match check_platform(&dir, &m(file)) {
                Ok(()) => assert!(ours, "{file} was let through"),
                Err(e) => assert!(!ours && e.contains(system), "{file}: {e}"),
            }
        }
        assert!(check_platform(&dir, &m("./script.sh")).is_ok());
        assert!(check_platform(&dir, &m("python3")).is_ok());
        assert!(check_platform(&dir, &m("./missing")).is_err());
        let own = std::env::current_exe().unwrap();
        std::fs::copy(&own, dir.join("native")).unwrap();
        assert!(check_platform(&dir, &m("./native")).is_ok());
    }

    #[test]
    fn long_messages_are_refused_not_kept() {
        let mut big = vec![b'x'; (MAX_MESSAGE + 10) as usize];
        big.extend_from_slice(b"\n{\"method\":\"log\"}\n");
        let mut r = std::io::Cursor::new(big);
        assert!(read_message(&mut r).is_err());
        assert_eq!(read_message(&mut r).unwrap().unwrap().trim(), "{\"method\":\"log\"}");
        assert_eq!(read_message(&mut r).unwrap(), None);
    }

    #[test]
    fn reads_adapter_commands() {
        assert_eq!(parse_command(r#"adapter "Big Reader" next"#), Some(("Big Reader".into(), "next".into())));
        assert_eq!(parse_command(&command_name("A", "b")), Some(("A".into(), "b".into())));
        assert_eq!(parse_command("next"), None);
    }
}
