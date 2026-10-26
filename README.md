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

What Leaf reads (it looks at what's inside a file, then its name):

- **Comic archives**: `.cbz`/`.zip`, `.cbr`/`.rar`, `.cb7`/`.7z`, `.cbt`/`.tar`
- **PDFs**, drawn sharp at whatever size the window is
- **EPUBs**: picture books in their own page order, novels as text
- **Text books**: `.txt`, Markdown, `.rtf`, Word (`.doc`, `.docx`),
  OpenDocument (`.odt`) and HTML, laid out into pages
- **Folders** of images, including chapter subfolders, or a single image
  (which opens its folder at that image)
- Anything an installed [adapter](docs/adapters.md) opens

Pages can be any image macOS can show: JPEG, PNG, WebP, GIF, BMP, HEIC,
AVIF, JPEG XL, TIFF, PSD, camera RAW and more, in natural order (`page2`
before `page10`).

## How it reads

Leaf works out what you've opened and reads it that way:

| Kind | How it's shown | How Leaf tells |
|---|---|---|
| **Manga** | Two pages side by side, right to left | Black-and-white pages, `ComicInfo.xml` saying manga, or a Japanese or "manga" name |
| **Comic** | One page at a time, left to right | Colour pages; also PDFs and text books |
| **Webtoon** | One long strip you scroll down | Tall, narrow pages, or "webtoon"/"manhwa" in the name |

It says what it decided in the title bar, once, and remembers it for that
book. If it guessed wrong, pick another in **View** (or press 1, 2 or 3).

In two-page mode the cover stands alone and the pages after it pair up;
a page that's already a two-page spread stands alone. If the pairs are off
by one, **View → Pair the Cover** (P) shifts them. **Two Pages** (D) turns
pairing on or off for any book. Every book remembers its own page, mode,
direction and fit.

## Reading

| Key | Does |
|---|---|
| → / ← (or click the right or left third) | Turn toward that side |
| Space, ↓, PageDown / Shift+Space, ↑, PageUp | Next / previous screen |
| Home, ⌘↑ / End, ⌘↓ | First / last page |
| 1 / 2 / 3 | Read as a comic / manga / webtoon |
| D | Two pages side by side, or one |
| P | Pair the cover (when the pairs are off by one) |
| M | Right to left, or left to right |
| W | Fit the whole page, or fit the width and scroll |
| F or ⌃⌘F / Esc | Full screen / leave it |
| ⌘O / ⌘⇧O | Open a file / a folder |
| ⌘W | Close the book |

The mouse wheel turns pages, or scrolls a webtoon or a page in fit-width
mode. A thin green line along the bottom shows how far through the book you
are.

The menu bar has everything too: **File**, **View** (the reading mode,
pairing, direction and fit, with checkmarks), **Go**, and **Help**. The
menus show your own shortcuts.

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

## Safety

Leaf follows the same rules as [Folio](https://github.com/solacecasino/folio):

- **It never goes online** and has no account or telemetry. The only web
  page it ever opens is Help, when you choose it.
- **Bad files can't take it down.** Pages are size-checked before they're
  decoded, archive entries over 256 MB are refused, and a page that can't be
  read shows as a dim card with the reason in the title bar.
- **Bad settings don't stop it.** A broken `state.json` is set aside as
  `state.json.bad`, not lost, and a mistake in `keys.toml` shows in the
  title bar.
- **A crash leaves a report.** The next launch says so, and **Help → Show
  Log** has the details. Your place in every book is saved as you read.
- **Adapters ask first** (see below), can't crash Leaf, can only do what
  their permissions allow, and one built for another kind of computer is
  refused with a message saying so.
- **Questions default to the safe answer.** Return and Escape decline;
  installing, allowing or removing takes a click.
- **Bug reports don't give you away.** **Help → Copy Debug Info** copies
  what a report needs (version, memory, settings, recent problems) without
  book names, and every path in it and in the log shows `~` for your home
  folder.

## Adapters

Leaf ships with no adapters. An adapter is an optional add-on: a separate
program, in any language, that can add commands or teach Leaf to open a new
kind of book. To install one, drop its `.zip` or folder on Leaf's window;
Leaf shows what it is and what it may do first. Its commands appear in an
**Adapters** menu, which only exists once something is installed, along
with **Remove**, which moves it to the Trash.

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

About 5,000 lines of Rust: `winit` for the window, the CPU for the pixels
(`softbuffer`), no GPU, the system's own menus and dialogs. Formats come
from macOS itself where it can read them, so they cost nothing to download:
libarchive for RAR, 7-Zip and TAR, Core Graphics for PDF, ImageIO for
images, and Core Text and AppKit for text.

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

- MOBI/AZW, DjVu and CBA (ACE) books, except through an adapter.
- Pictures inside text EPUBs; a text book is shown as its words.
- Live reload of `keys.toml`.
- Windows and Linux builds. The code is cross-platform, but only the Mac app
  is built and tested, and its archive, PDF and text reading is Mac-only.
