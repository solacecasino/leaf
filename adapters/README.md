# Leaf adapters

Adapters are optional add-ons for [Leaf](../README.md): separate programs
that add commands, open more kinds of books, or (new in Leaf 0.0.4) change
how pages look. Leaf ships with none. Each one here is built with Leaf's
releases, for every system Leaf runs on; download its zip from
[the releases page](https://github.com/solacecasino/leaf/releases) and drop
it on Leaf's window.

| Adapter | What it does | Needs |
|---|---|---|
| [Upscale 4K](upscale-4k/) | Upscales pages on the GPU: manga line art retraced and kept thin (in the spirit of Anime4K), colour comics upscaled softly (like FSR) | A GPU with Metal, DirectX 12 or Vulkan; the `book.pages` permission |

To make one, read [docs/adapters.md](../docs/adapters.md). An adapter here
lives in its own folder with an `adapter.toml`, a README, and the code to
build it; the release workflow builds and tests it on every system.
