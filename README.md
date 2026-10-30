# leaf9000

The reader of the [9000 series](#the-9000-series): a bare comic and manga
reader. Open it and you're back on the page you were
reading, before the window has finished appearing. No library screen, no
account, no settings window.

## Install

Download from [Releases](https://github.com/solacecasino/leaf9000/releases):

| System | Download |
|---|---|
| Mac, any (macOS 11 or later) | `leaf9000-<version>-macos-universal.zip` |
| Mac with Apple Silicon / Intel | `leaf9000-<version>-macos-arm64.zip` / `-macos-x64.zip` (smaller) |
| Windows, x64 / Arm | `leaf9000-<version>-windows-x64.zip` / `-windows-arm64.zip` |
| Linux, x86-64 / Arm | `leaf9000-<version>-linux-x64.tar.gz` / `-linux-arm64.tar.gz` |

- **Mac**: unzip it and move **leaf9000.app** to Applications. The first time,
  **right-click leaf9000 → Open**, then **Open**: leaf9000 isn't signed with an
  Apple developer certificate, so macOS asks once. (Or run
  `xattr -dr com.apple.quarantine /Applications/leaf9000.app`.)
- **Windows**: unzip it and run **leaf9000.exe** (keep `pdfium.dll` beside it:
  it draws PDFs). SmartScreen may ask once, since leaf9000 isn't signed:
  **More info → Run anyway**. Double-click `install.cmd` in the same
  folder to put leaf9000, with its icon, under **Open with** for books and
  images and "Open in leaf9000" on folders, and to open comic archives in leaf9000
  unless another app already does (`install.cmd --remove` takes it out
  again; only your own account's settings change, no administrator needed).
- **Linux**: unpack it and run `leaf9000` (keep `libpdfium.so` beside it).
  `./install.sh` in the same folder adds leaf9000 to your app menu with its
  icon, offers it under **Open With** for books and images, and opens comic
  archives in leaf9000 unless another app already does (`./install.sh
  --remove` takes it out again; nothing outside your home folder changes). The
  window needs libxkbcommon (and libxkbcommon-x11 on X11), which every
  desktop has. RAR, 7-Zip and TAR use the system's libarchive when it's
  there, and leaf9000's own readers when it isn't; HEIC pictures need libheif.

Every system reads every format below. On a Mac, macOS draws PDFs and lays
out text with its own renderers; elsewhere leaf9000 does (with pdfium and
cosmic-text). HEIC uses the system's decoder everywhere: on Windows that's
the HEIF and HEVC extensions from the Microsoft Store, which most PCs have.

## Opening books

Any of these opens a book instantly:

- **Drag it onto leaf9000's window.** An outline in page gold shows it'll be taken.
- **Drag it onto leaf9000 in the Dock**, or onto leaf9000.app in Finder. If leaf9000
  isn't running, it starts straight on that book.
- **Double-click a `.cbz`** (Mac): comic archives open in leaf9000. For zips,
  EPUBs, images and folders, use **Open With → leaf9000**; leaf9000 doesn't take
  those over.
- **File → Open…** (⌘O), **Open Folder…** (⌘⇧O) or **Open Recent**.

After that, just open leaf9000: it reopens the last book on the last page.
At the end of a book, turning the page opens the **next book in the same
folder** (Vol 02 after Vol 01, or the next chapter), and turning back from
the first page opens the one before at its last page.

What leaf9000 reads (it looks at what's inside a file, then its name):

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

leaf9000 works out what you've opened and reads it that way:

| Kind | How it's shown | How leaf9000 tells |
|---|---|---|
| **Manga** | Two pages side by side, right to left | `ComicInfo.xml` saying manga (or a Japanese publisher or language), an EPUB that reads right to left, a Japanese or "manga" name, or black-and-white pages |
| **Comic** | One page at a time, left to right | `ComicInfo.xml` saying it isn't manga (or a Western publisher), an issue number like `#012` in the name, or colour pages; also PDFs and text books |
| **Webtoon** | One long strip you scroll down | Tall, narrow pages, a webtoon format or publisher in `ComicInfo.xml`, or "webtoon"/"manhwa" in the name |

What the book says comes first; the pages decide only when it says nothing.
leaf9000 says what it decided in the title bar, once, and remembers it for that
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
for the magnifier. A thin gold line along the bottom shows how far
through the book you are.

The menu bar (Mac and Windows) has everything too: **File**, **View** (the
reading mode, pairing, direction, fit and magnifier, with checkmarks),
**Go**, **9000** (the sister apps) and **Help**. The menus show your own shortcuts.

## Your files

Everything lives in one folder. **leaf9000 → Open Config Folder** (⌘,) shows
it: `~/Library/Application Support/leaf9000` on a Mac, `%APPDATA%\leaf9000`
on Windows, `~/.config/leaf9000` on Linux. Copy it to move your setup.
Coming from Leaf? The first start copies Leaf's folder over (where you were
in every book, settings, keys and adapters) and leaves Leaf's as it was.

| File | What it is |
|---|---|
| `state.json` | Where you are in each book, and recent books. Readable and safe to edit. |
| `keys.toml` | Rebind any key, including adapter commands. Written on first run with every command in comments. **leaf9000 → Edit Key Bindings** opens it. Saved changes apply straight away. |
| `settings.toml` | Colours, the gap between two pages, the webtoon width, the text books' font and paper, whether to go on to the next book, the magnifier's zoom, how new books are read, and how hard an upscaler works (`upscale_style`, `upscale_strength`). Written on first run with every setting commented out; **leaf9000 → Edit Settings** opens it. Saved changes apply straight away. |
| `adapters/` | Installed adapters, one folder each. |
| `approvals.json` | Which adapters you've allowed to run. |
| `leaf9000.log` | What happened this run, including adapters' output. **Help → Show Log**. |

## Safety

leaf9000 follows the same rules as its sisters in the 9000 series:

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
- **Adapters ask first** (see below), can't crash leaf9000, can only do what
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
to the window, the magnifier) are upscaled on the GPU instead of by leaf9000.
In the spirit of Anime4K, manga and webtoon line art isn't just enlarged:
its lines are retraced, so edges stay crisp and lines keep their weight.
Colour comics are upscaled softly. leaf9000 never waits for it: pages show at
once and the upscaled ones replace them as they arrive. **View → Upscale
Pages** (U) turns it off and on, and `upscale_strength` in settings.toml
makes it `gentle`, `normal` or `strong`. On an Apple M2, a page shown at 4K
takes 32 ms (line art) or 22.5 ms (colour), against 48 ms for leaf9000's own,
plainer scaler; the adapter uses 148 MB while it works. It needs a real
GPU: on a software one it turns itself down and leaf9000 scales as before.

## Adapters

leaf9000 ships with no adapters; [the catalog](adapters/README.md) lists the
ones built with it. An adapter is an optional add-on: a separate
program, in any language, that can add commands or teach leaf9000 to open a new
kind of book. To install one, drop its `.zip` or folder on leaf9000's window;
leaf9000 shows what it is and what it may do first. Its commands appear in an
**Adapters** menu, which only exists once something is installed, along
with **Remove**, which moves it to the Trash.

[docs/adapters.md](docs/adapters.md) describes how to write one.

## The 9000 series

leaf9000 is one of five apps built the same way: bare, modular, configured
with plain files, in the series' graphite with one colour each. leaf9000's
is page gold. Its sisters are
[text9000](https://github.com/solacecasino/text9000) (writing),
[splice9000](https://github.com/solacecasino/splice9000) (video),
[wave9000](https://github.com/solacecasino/wave9000) (sound) and
[grace9000](https://github.com/solacecasino/grace9000) (photos).

- **Open or drop** a `.t9k` document, a video, a sound file, or a `.splice`,
  `.wave` or `.grace` project, and it opens in the sister app that makes
  it. Books and pictures, photos included, stay here: leaf9000 is the
  series' reader.
- **The 9000 menu** opens any sister app that's installed, and only those.
  `app grace9000` in `keys.toml` binds one to a key.
- text9000 sends comic archives (`.cbz`, `.cbr`, `.cb7`, `.cbt`) here.

leaf9000 finds its sisters by itself: on macOS wherever they're installed,
on Linux through their app-menu entries or your `PATH`, and on Windows on
your `PATH` or unzipped next to leaf9000's own folder. Otherwise, name
their paths under `[apps]` in `settings.toml`.

## Building

```bash
cargo run --release -- "some manga.cbz"
cargo run --release --example sample   # makes samples/Demo Manga.cbz to try
scripts/package.sh                     # dist/leaf9000-<version>-macos-universal.zip
scripts/test-books.sh                  # a book in every format, for leaf9000 --check
leaf9000 --check some.cbr                  # opens a book without a window: exits 0 if it worked
```

`scripts/make_icon.swift` draws the icon. `LEAF9000_TIMING=1` prints how long each
page took. `LEAF9000_CONFIG_DIR` points leaf9000 at another config folder.
Leaf's spellings (`LEAF_TIMING`, `LEAF_CONFIG_DIR`, ...) still work.

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
