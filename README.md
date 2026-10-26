# Leaf

A bare comic and manga reader. Open it and you're back on the page you were
reading, before the window has finished appearing. No library screen, no
account, no settings window.

## Install (Mac)

1. Download `Leaf-<version>-macos-universal.zip` from
   [Releases](https://github.com/solacecasino/leaf/releases) and unzip it.
   It runs on Apple Silicon and Intel Macs (macOS 11 or later).
2. Move **Leaf.app** to Applications.
3. The first time, **right-click Leaf → Open**, then **Open**. Leaf isn't
   signed with an Apple developer certificate, so macOS asks once.
   (Or run `xattr -dr com.apple.quarantine /Applications/Leaf.app`.)

## Opening books

Any of these opens a book instantly:

- **Drag it onto Leaf's window.** An outline in Leaf green shows it'll be taken.
- **Drag it onto Leaf in the Dock**, or onto Leaf.app in Finder. If Leaf
  isn't running, it starts straight on that book.
- **Double-click a `.cbz`**: comic archives open in Leaf. For zips, EPUBs,
  images and folders, use **Open With → Leaf**; Leaf doesn't take those over.
- **File → Open…** (⌘O), **Open Folder…** (⌘⇧O) or **Open Recent**.

After that, just open Leaf: it reopens the last book on the last page.

What Leaf reads (it looks at what's inside a file, not its name):

- **Archives**: `.cbz`, `.zip`, and image `.epub`
- **Folders** of images, including chapter subfolders
- **A single image**, which opens its folder at that image
- Anything an installed [adapter](docs/adapters.md) opens, such as `.cbr` or PDF

Pages can be JPEG, PNG, WebP, GIF or BMP, in natural order (`page2` before
`page10`). Every book remembers its own page, reading direction and fit.

## Reading

| Key | Does |
|---|---|
| → / ← (or click the right or left third) | Turn toward that side |
| Space, ↓, PageDown / Shift+Space, ↑, PageUp | Next / previous screen |
| Home, ⌘↑ / End, ⌘↓ | First / last page |
| M | Manga mode (right to left) |
| W | Fit the whole page, or fit the width and scroll |
| F or ⌃⌘F / Esc | Full screen / leave it |
| ⌘O / ⌘⇧O | Open a file / a folder |
| ⌘W | Close the book |

The mouse wheel turns pages, or scrolls in fit-width mode. A thin green line
along the bottom shows how far through the book you are.

The menu bar has everything too: **File**, **View** (fit and direction, with
checkmarks), **Go**, and **Help → Show Log**. The menus show your own
shortcuts.

## Your files

Everything lives in one folder. **Leaf → Open Config Folder** (⌘,) shows
it: `~/Library/Application Support/Leaf`. Copy it to move your setup.

| File | What it is |
|---|---|
| `state.json` | Where you are in each book, and recent books. Readable and safe to edit. |
| `keys.toml` | Rebind any key, including adapter commands. Written on first run with every command in comments. **Leaf → Edit Key Bindings** opens it. Changes apply the next time Leaf starts. |
| `adapters/` | Installed adapters, one folder each. |
| `approvals.json` | Which adapters you've allowed to run. |
| `leaf.log` | What happened this run, including adapters' output. **Help → Show Log**. |

A broken `state.json` is set aside as `state.json.bad`, not lost, and a
mistake in `keys.toml` shows in the title bar instead of stopping Leaf.

## Adapters

Leaf ships with no adapters. An adapter is an optional add-on: a separate
program, in any language, that can add commands or teach Leaf to open a new
kind of book. To install one, drop its `.zip` or folder on Leaf's window;
Leaf shows what it is and what it may do first. Its commands appear in an
**Adapters** menu, which only exists once something is installed.

[docs/adapters.md](docs/adapters.md) describes how to write one.

## Building

```bash
cargo run --release -- "some manga.cbz"
cargo run --release --example sample   # makes samples/Demo Manga.cbz to try
scripts/package.sh                     # dist/Leaf-<version>-macos-universal.zip
```

`scripts/make_icon.swift` draws the icon. `LEAF_TIMING=1` prints how long each
page took.

## How it's built

About 2,700 lines of Rust: `winit` for the window, the CPU for the pixels
(`softbuffer`), no GPU, the system's own menus and dialogs.

- **The page is ready before the window is.** The last book opens and its
  page is decoded and scaled on a background thread while macOS is still
  creating the window, sized from the window size and display scale saved
  last time.
- **The next and previous pages are always ready**, so turning is just
  copying pixels.
- **Scaling is done in integers** with a filter that averages every source
  pixel, so screentone doesn't shimmer.
- **Big buffers go straight back to the system** when they're freed.
- **One owner of state.** Everything lives on the UI thread; the page loader
  and adapters talk to it through events.

## Not yet

- RAR (`.cbr`), 7-Zip (`.cb7`) and PDF without an adapter. Leaf says so
  instead of guessing.
- Text EPUBs and other text books; two-page spreads.
- Live reload of `keys.toml`.
- Windows and Linux builds. The code is cross-platform, but only the Mac app
  is built and tested.
