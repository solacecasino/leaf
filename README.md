# Leaf

A bare comic and manga reader. Open it and you're back on the page you were
reading, before the window has finished appearing. No library screen, no
account, no settings window.

**Prototype.** It reads image books. See "Not yet" below.

## Try it

```bash
cargo run --release --example sample     # makes samples/Demo Manga.cbz
cargo run --release -- "samples/Demo Manga.cbz"
```

After that, just run `leaf` with no arguments: it opens the last book at the
last page. You can also drop a file on the window, or press ⌘O (Ctrl+O).

## What it opens

What a file is gets decided by its contents, not its name:

- **Archives**: `.cbz`, `.zip`, and image `.epub` (any zip of images)
- **Folders** of images, including chapter subfolders
- **A single image**, which opens its folder at that image

Pages can be JPEG, PNG, WebP, GIF or BMP, sorted naturally (`page2` comes
before `page10`). Every book remembers its own page, reading direction and fit.

## Keys

| Key | Does |
|---|---|
| → / ← (or click the right or left third) | Turn toward that side |
| Space, ↓, PageDown / Shift+Space, ↑, PageUp | Next / previous screen |
| Home / End | First / last page |
| M | Manga mode (right to left) |
| W | Fit the whole page, or fit the width and scroll |
| F / Esc | Fullscreen / leave it |
| ⌘O / ⌘⇧O | Open a file / a folder |

The mouse wheel turns pages, or scrolls in fit-width mode. A thin line
along the bottom shows how far through the book you are.

## Your files

Everything lives in one folder. Copy it to move your setup:
`~/Library/Application Support/Leaf` (macOS), `%APPDATA%\Leaf` (Windows),
`~/.config/leaf` (Linux).

- `state.json`: where you are in each book. Readable and safe to edit.
- `keys.toml`: rebind any key. It's written on first run with every command
  and examples in comments.

A broken `state.json` is set aside as `state.json.bad`, not lost, and a
mistake in `keys.toml` shows in the title bar instead of stopping Leaf.

## How it's built

About 1,200 lines of Rust: `winit` for the window, the CPU for the pixels
(`softbuffer`), no GPU.

- **The page is ready before the window is.** The last book opens and its
  page is decoded and scaled on a background thread while macOS is still
  creating the window, sized from the window size and display scale saved
  last time.
- **The next and previous pages are always ready**, so turning is just
  copying pixels.
- **Scaling is done in integers** with a triangle filter that averages every
  source pixel, so screentone doesn't shimmer. It's 1.6–2× faster than the
  image library's float resize (~40 → ~20 ms for a 1200×1700 page), which
  had been most of the wait.
- **Big buffers go straight back to the system.** Allocations of 256 KB or
  more are mapped and unmapped directly; macOS's allocator was keeping
  ~60 MB of freed page memory dirty.

`LEAF_TIMING=1 leaf` prints how long each page took.

## Measured (Apple Silicon, macOS, Retina window ~820×890 pt)

| | |
|---|---|
| Launch to your page on screen | ~115 ms (window appears at ~95 ms) |
| Decode + scale one page | ~22 ms (1200×1700 PNG), ~60 ms (2400×3400 JPEG) |
| Turning to a ready page | one frame |
| Memory with a book open | ~75–85 MB (three pages ready at Retina size) |
| Idle CPU | 0% |
| Release binary | 1.6 MB |

What didn't work: prefetching five pages (≈190 MB after reading a while),
and asking macOS's allocator to release memory with
`malloc_zone_pressure_relief` (no change; replaced by direct mapping).

## Not yet

- RAR (`.cbr`), 7-Zip (`.cb7`) and PDF: Leaf says so instead of guessing.
- Text EPUBs and other text books.
- Two-page spreads, a recent-books list, live reload of `keys.toml`.
- Very tall webtoon strips in fit-width use a lot of memory.
- Only tested on macOS so far. The code builds for Windows and Linux, but
  hasn't been run there.
