# Leaf

A bare comic and manga reader. Open it and you're back on the page you were
reading, before the window has finished appearing. No library screen, no
account, no settings window.

## Install

Download from [Releases](https://github.com/solacecasino/leaf/releases):

| System | Download |
|---|---|
| Mac, any (macOS 11 or later) | `Leaf-<version>-macos-universal.zip` |
| Mac with Apple Silicon / Intel | `Leaf-<version>-macos-arm64.zip` / `-macos-x64.zip` (smaller) |
| Windows, x64 / Arm | `Leaf-<version>-windows-x64.zip` / `-windows-arm64.zip` |
| Linux, x86-64 / Arm | `leaf-<version>-linux-x64.tar.gz` / `-linux-arm64.tar.gz` |

- **Mac**: unzip it and move **Leaf.app** to Applications. The first time,
  **right-click Leaf → Open**, then **Open**: Leaf isn't signed with an
  Apple developer certificate, so macOS asks once. (Or run
  `xattr -dr com.apple.quarantine /Applications/Leaf.app`.)
- **Windows**: unzip it and run **Leaf.exe** (keep `pdfium.dll` beside it:
  it draws PDFs). SmartScreen may ask once, since Leaf isn't signed:
  **More info → Run anyway**.
- **Linux**: unpack it and run `leaf` (keep `libpdfium.so` beside it). The
  window needs libxkbcommon (and libxkbcommon-x11 on X11), which every
  desktop has. RAR, 7-Zip and TAR use the system's libarchive when it's
  there, and Leaf's own readers when it isn't; HEIC pictures need libheif.

Every system reads every format below. On a Mac, macOS draws PDFs and lays
out text with its own renderers; elsewhere Leaf does (with pdfium and
cosmic-text). HEIC uses the system's decoder everywhere: on Windows that's
the HEIF and HEVC extensions from the Microsoft Store, which most PCs have.

## Opening books

Any of these opens a book instantly:

- **Drag it onto Leaf's window.** An outline in Leaf green shows it'll be taken.
- **Drag it onto Leaf in the Dock**, or onto Leaf.app in Finder. If Leaf
  isn't running, it starts straight on that book.
- **Double-click a `.cbz`** (Mac): comic archives open in Leaf. For zips,
  EPUBs, images and folders, use **Open With → Leaf**; Leaf doesn't take
  those over.
- **File → Open…** (⌘O), **Open Folder…** (⌘⇧O) or **Open Recent**.

After that, just open Leaf: it reopens the last book on the last page.
At the end of a book, turning the page opens the **next book in the same
folder** (Vol 02 after Vol 01, or the next chapter), and turning back from
the first page opens the one before at its last page.

What Leaf reads (it looks at what's inside a file, then its name):

- **Comic archives**: `.cbz`/`.zip`, `.cbr`/`.rar`, `.cb7`/`.7z`, `.cbt`/`.tar`
- **PDFs**, drawn sharp at whatever size the window is
- **EPUBs**: picture books in their own page order, novels as text with
  their illustrations on pages of their own
- **Text books**: `.txt`, Markdown, `.rtf`, Word (`.doc`, `.docx`),
  OpenDocument (`.odt`) and HTML, laid out into pages (on a Mac with their
  formatting; elsewhere as their words and paragraphs)
- **Folders** of images, including chapter subfolders, or a single image
  (which opens its folder at that image)
- Anything an installed [adapter](docs/adapters.md) opens

Pages can be JPEG, PNG, WebP, GIF, BMP, HEIC, AVIF, JPEG XL, JPEG 2000,
TIFF, PSD, ICNS, camera RAW (DNG, CR2, CR3, NEF, ARW, RAF, ORF, RW2...),
EXR, HDR, TGA and more, in natural order (`page2` before `page10`).

## How it reads

Leaf works out what you've opened and reads it that way:

| Kind | How it's shown | How Leaf tells |
|---|---|---|
| **Manga** | Two pages side by side, right to left | `ComicInfo.xml` saying manga (or a Japanese publisher or language), an EPUB that reads right to left, a Japanese or "manga" name, or black-and-white pages |
| **Comic** | One page at a time, left to right | `ComicInfo.xml` saying it isn't manga (or a Western publisher), an issue number like `#012` in the name, or colour pages; also PDFs and text books |
| **Webtoon** | One long strip you scroll down | Tall, narrow pages, a webtoon format or publisher in `ComicInfo.xml`, or "webtoon"/"manhwa" in the name |

What the book says comes first; the pages decide only when it says nothing.
Leaf says what it decided in the title bar, once, and remembers it for that
book. If it guessed wrong, pick another in **View** (or press 1, 2 or 3).
`default_mode` in settings.toml can skip the guessing for new books.

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
| G or ⌘G, a number, Return | Go to that page |
| ⌘] / ⌘[ | Next / previous book in the folder |
| Z | The magnifier: enlarges what's under the pointer |
| U | Upscale pages (with an upscaler adapter), or not |
| 1 / 2 / 3 | Read as a comic / manga / webtoon |
| D | Two pages side by side, or one |
| P | Pair the cover (when the pairs are off by one) |
| M | Right to left, or left to right |
| W | Fit the whole page, or fit the width and scroll |
| F or ⌃⌘F / Esc | Full screen / leave it |
| ⌘O / ⌘⇧O | Open a file / a folder |
| ⌘W | Close the book |

The mouse wheel turns pages, or scrolls a webtoon or a page in fit-width
mode. On a trackpad, **swipe sideways** with two fingers to turn the page,
**pinch out** to fit a page's width (or, with two pages, bring out the
magnifier) and pinch in to go back, and **double-tap with two fingers**
for the magnifier. A thin green line along the bottom shows how far
through the book you are.

The menu bar (Mac and Windows) has everything too: **File**, **View** (the
reading mode, pairing, direction, fit and magnifier, with checkmarks),
**Go**, and **Help**. The menus show your own shortcuts.

## Your files

Everything lives in one folder. **Leaf → Open Config Folder** (⌘,) shows
it: `~/Library/Application Support/Leaf` on a Mac, `%APPDATA%\Leaf` on
Windows, `~/.config/leaf` on Linux. Copy it to move your setup.

| File | What it is |
|---|---|
| `state.json` | Where you are in each book, and recent books. Readable and safe to edit. |
| `keys.toml` | Rebind any key, including adapter commands. Written on first run with every command in comments. **Leaf → Edit Key Bindings** opens it. Saved changes apply straight away. |
| `settings.toml` | Colours, the gap between two pages, the webtoon width, the text books' font and paper, whether to go on to the next book, the magnifier's zoom, and how new books are read. Written on first run with every setting commented out; **Leaf → Edit Settings** opens it. Saved changes apply straight away. |
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
  `state.json.bad`, not lost, and a mistake in `keys.toml` or
  `settings.toml` shows in the title bar while everything else keeps working.
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

## Upscaling (Upscale 4K)

Install the [Upscale 4K](adapters/upscale-4k/) adapter and pages shown bigger
than they are (on a 4K or Retina screen, in full screen, a webtoon widened
to the window, the magnifier) are upscaled on the GPU instead of by Leaf.
In the spirit of Anime4K, manga and webtoon line art isn't just enlarged:
its lines are retraced, so edges stay crisp and lines keep their weight.
Colour comics are upscaled softly. Leaf never waits for it: pages show at
once and the upscaled ones replace them as they arrive. **View → Upscale
Pages** (U) turns it off and on. On an Apple M2, a page shown at 4K takes
31–33 ms (line art) or 23 ms (colour), against 47 ms for Leaf's own,
plainer scaler.

## Adapters

Leaf ships with no adapters; [the catalog](adapters/README.md) lists the
ones built with it. An adapter is an optional add-on: a separate
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
scripts/test-books.sh                  # a book in every format, for leaf --check
leaf --check some.cbr                  # opens a book without a window: exits 0 if it worked
```

`scripts/make_icon.swift` draws the icon. `LEAF_TIMING=1` prints how long each
page took. `LEAF_CONFIG_DIR` points Leaf at another config folder.

Every push is built and tested on Apple Silicon and Intel Macs, Windows and
Linux by `.github/workflows/release.yml`, which also opens real RAR, 7-Zip,
TAR, PDF and text books with each download. Pushing a tag like `v0.0.3`
publishes the release.

## How it's built

About 8,500 lines of Rust (and 800 for the upscaler): `winit` for the window, the CPU for the pixels
(`softbuffer`), no GPU, the system's own menus and dialogs. Formats come
from the system itself where it can read them, so they cost nothing to
download: libarchive for RAR, 7-Zip and TAR (macOS and Linux), and on macOS
Core Graphics for PDF, ImageIO for images, and Core Text and AppKit for text.
Windows and Linux get pure-Rust readers for the rest (cosmic-text, unrar,
sevenz, jxl-oxide, rav1d and others) and pdfium for PDFs.

- **The page is ready before the window is.** The last book opens and its
  page is decoded and scaled on a background thread while macOS is still
  creating the window, sized from the window size and display scale saved
  last time.
- **The next and previous pages are always ready**, so turning is just
  copying pixels.
- **Scaling is done in integers** with a filter that averages every source
  pixel, so screentone doesn't shimmer.
- **A webtoon is held in bands** 512 pixels tall, only those near the
  window, never whole strips several screens long.
- **A text book is laid out as you read**: the first page right away, the
  rest in the background, so a long novel opens as fast as a short one.
- **Big buffers go straight back to the system** when they're freed.
- **One owner of state.** Everything lives on the UI thread; the page loader
  and adapters talk to it through events.

## Not yet

- MOBI/AZW, DjVu and CBA (ACE) books, except through an adapter.
- A menu bar on Linux (every command has a key).
- The formatting of Word, RTF and OpenDocument files on Windows and Linux
  (their words and paragraphs are there).
- Signed and notarized downloads.
