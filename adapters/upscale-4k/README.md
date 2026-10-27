# Upscale 4K

A [Leaf](https://github.com/solacecasino/leaf) adapter that upscales pages on
the GPU whenever Leaf shows one bigger than it is: on a 4K (or Retina)
screen, in full screen, for webtoons widened to the window, and in the
magnifier.

It follows [Anime4K](https://github.com/bloc97/Anime4K)'s idea rather than
its code: line art isn't just enlarged, its lines are **retraced**. Its own
shaders, written for comic and manga pages, do it in a few steps:

1. **Upscale** with a bicubic filter held to the four nearest pixels' range,
   so edges get no halo.
2. **Retrace** (line art): an enlarged edge is a ramp a few pixels wide.
   Pixels on the ramp take the colour of the neighbour with the flattest
   gradient, so the ramp collapses back into a clean edge.
3. **Thin** (line art): enlarging spreads a dark line's edges into grey;
   pixels between a clearly dark side and a clearly light one move toward
   the light side, so lines keep their weight.
4. **Sharpen**, adapting to contrast, like AMD's FSR 1 RCAS: busy areas
   aren't over-sharpened, and nothing goes past its neighbours (no halos).

Manga and webtoons get all four (**lines**); colour comics get 1 and 4
(**soft**). Leaf picks by the book's reading mode; `upscale_style` in
Leaf's settings.toml can fix it to one.

## Install

Download the zip for your computer from
[Leaf's releases](https://github.com/solacecasino/leaf/releases) and drop it
on Leaf's window. Leaf shows what it is and asks before installing. It's on
from then on: **View → Upscale Pages** (U) turns it off and on.

| Download | For |
|---|---|
| `upscale-4k-<version>-macos-universal.zip` | Macs (Metal) |
| `upscale-4k-<version>-windows-x64.zip`, `-windows-arm64.zip` | Windows (DirectX 12, or Vulkan) |
| `upscale-4k-<version>-linux-x64.zip`, `-linux-arm64.zip` | Linux (Vulkan) |

It needs the `book.pages` permission: it sees the pages you read, to
upscale them. It never goes online.

## How fast

`upscale-4k --bench` times it on your GPU; `upscale-4k --selftest` checks
it (crisp edges, lines that keep their weight, untouched paper, smooth
gradients). On an Apple M2, a 1200×1700 page upscaled to 2711×3840
(a page on a 4K screen), through Leaf:

| | Each page |
|---|---|
| Leaf's own scaler (CPU, no retracing) | 47 ms |
| Line art (lines) | 31–33 ms |
| Colour (soft) | 23 ms |
| A webtoon band (512 rows) | 5 ms |

The shaders themselves take 13.5 ms (lines) and 5.1 ms (soft) a page when
the GPU is busy; the rest is the GPU waking up and handing the picture back.
Leaf never waits for it: pages show at once, scaled by Leaf, and the
upscaled ones replace them as they arrive.

## How it talks to Leaf

It's an ordinary Leaf adapter (protocol 3, `filters = ["upscale"]`): Leaf
sends `upscale.ready` when it starts (the GPU is set up then), then
`page.upscale` with two files' paths, the source (RGB) and the result
(0RGB), which both map into memory. See Leaf's
[adapter docs](../../docs/adapters.md).

## Building

```bash
cargo build --release
./target/release/upscale-4k --selftest
```

Then put `upscale-4k` (or `upscale-4k.exe`, with `run` changed to match)
in a folder with `adapter.toml`, and drop the folder on Leaf.
