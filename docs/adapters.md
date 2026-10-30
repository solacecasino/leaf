# leaf9000 adapters

leaf9000 ships bare. An **adapter** is an optional add-on you install on
purpose, like foobar2000's components. It's an ordinary program in any
language. leaf9000 starts it only when it's needed and talks to it with JSON
messages, one per line, over stdin and stdout. Adapters work the same way as
[Folio's](https://github.com/solacecasino/folio/blob/main/docs/adapters.md),
so the same habits carry over.

An adapter can:

- **add commands**, shown in leaf9000's menus and bindable to keys;
- **open books leaf9000 can't read itself**, such as `.cbr` or PDF: leaf9000 asks it
  for the page list, then for each page's image;
- **follow along**: be told when a book opens or the page changes, and ask
  leaf9000 which book is open or to turn the page;
- **upscale pages** (protocol 3): make the bigger picture whenever leaf9000
  shows a page bigger than it is, for example on the GPU.

leaf9000 ships with none. [The catalog](../adapters/README.md) lists the ones
built with leaf9000, such as [Upscale 4K](../adapters/upscale-4k/).

## Installing

Drop an adapter's `.zip` or folder on leaf9000's window (or open it with leaf9000).
leaf9000 shows its name, author, license, what it opens and what it may do, and
installs it only when you click **Install**: Return and Escape cancel. It
works right away, with no restart.

Installed adapters live in the `adapters` folder of the config folder
(**leaf9000 → Show Adapters Folder**). You can also copy an adapter folder there
by hand and choose **leaf9000 → Reload Adapters**. leaf9000 then asks you to allow it
the first time it runs. Your answers are kept in `approvals.json`, until an
adapter asks for more permissions.

A package is a folder containing `adapter.toml`, or a zip holding that
folder (or those files at its top).

## `adapter.toml`

```toml
name = "RAR Pages"              # shown in menus and dialogs
version = "0.1.0"
author = "you"
license = "MIT"
description = "Opens .cbr comics."
homepage = "https://github.com/you/rar-pages"

run = ["./rar-pages"]           # program + arguments, run from this folder
permissions = ["book.read"]
start = "command"               # "command" (default) or "launch"
events = ["opened", "page"]     # events to receive while running
opens = ["cbr"]                 # file extensions this adapter opens as books
# filters = ["upscale"]         # upscales pages (needs "book.pages")

[[commands]]
id = "info"                     # sent back to you when chosen
label = "Book Info"             # menu text
keys = "cmd+alt+i"              # optional default shortcut
# menu = "go"                   # optional: File, View or Go instead of Adapters
```

- `run` starts in the adapter's folder. A program given as a path
  (`./rar-pages`) is found there.
- leaf9000 sets `LEAF9000_CONFIG_DIR` (and `LEAF_CONFIG_DIR`, its name from before the series) to its config folder, for adapters that keep
  files of their own, and `PYTHONIOENCODING=utf-8`.
- `start = "command"` starts the adapter the first time you pick one of its
  commands or open a book it reads. `start = "launch"` starts it with leaf9000.
- When leaf9000 quits (or reloads adapters) it sends `shutdown`, waits half a
  second, then stops the program.
- Whatever the program prints to **stderr** goes to `leaf.log`
  (**Help → Show Log**).
- An adapter that crashes doesn't take leaf9000 with it. leaf9000 logs it, says so
  in the title bar, and starts it again the next time it's needed.
- Users can bind your commands in `keys.toml`:
  `"cmd+k" = 'adapter "RAR Pages" info'`.

### Permissions

| Permission | Allows |
|---|---|
| `book.read` | `book.get`, and the `opened` and `page` events |
| `book.control` | `book.goto`, `book.open` |
| `book.pages` | Seeing pages' pixels: needed by `filters` |
| `network` | Nothing through leaf9000. It tells the user the adapter goes online |

Permissions control what an adapter can do **through leaf9000**. An adapter is a
normal program running as you, so `network` is an honest declaration, not a
sandbox. Only install adapters you trust; being open source is what lets
people check. Opening books (`opens`) needs no permission: leaf9000 only sends
the adapter the file you chose to open.

## Messages

Every message is one JSON object on one line, in UTF-8.

- A **notification** has `method` and `params` and gets no reply.
- A **request** also has an `id`. The reply has the same `id` and either
  `result` or `error` (a string).

### leaf9000 → adapter

| Method | Params | Notes |
|---|---|---|
| `initialize` | `leaf` (version), `protocol` (currently `2`), `permissions` | Notification, right after starting |
| `command` | `command`: the command's `id` | Notification: you picked one of its commands |
| `event` | `kind` plus fields below | Notification, if listed in `events` and you have `book.read` |
| `shutdown` | | Notification: exit soon |
| `book.open` | `path` | **Request**, for adapters with `opens`. Reply `{ "pages": ["001.jpg", ...], "name": "optional title" }` |
| `book.page` | `path`, `page` (from 0), `name` (from your list) | **Request**. Reply `{ "file": "/path/to/image" }` or `{ "base64": "..." }` with JPEG, PNG, WebP, GIF or BMP bytes |
| `upscale.ready` | | **Request**, for adapters with `filters = ["upscale"]`, sent when leaf9000 starts them. Get ready (set up the GPU) and reply `{ "device": "what it runs on" }`; leaf9000 shows that once. Until you answer, leaf9000 scales pages itself |
| `page.upscale` | see below | **Request**: make one upscaled picture |

leaf9000 asks for pages from a background thread, up to three at a time around
the page being read, and waits up to 30 seconds for each answer.

Events:

| `kind` | Fields |
|---|---|
| `opened` | `path`, `name`, `pages` |
| `page` | `page` (from 0), `pages` |

### Adapter → leaf9000

| Method | Params | Result | Permission |
|---|---|---|---|
| `book.get` | | `{ path, name, page, pages, rtl, fit, mode }`, or `null` with no book open. `mode` is `comic`, `manga` or `webtoon` | `book.read` |
| `book.goto` | `page` (from 0) | `null` | `book.control` |
| `book.open` | `path` | `null` | `book.control` |
| `notify` | `text` | `null`. Shows a short message in the title bar | none |
| `log` | `message` | `null`. Written to `leaf.log` | none |
| `ui.confirm` | `title`, `message` | `true` or `false` from a native dialog; Return answers No | none |

### Upscaling (protocol 3)

An adapter with `filters = ["upscale"]` and the `book.pages` permission is
asked for every picture leaf9000 shows bigger than it is (on a 4K or Retina
screen, a webtoon widened to the window, the magnifier), while **View →
Upscale Pages** (U) is on. Pixels don't go through the JSON pipe: they're
in two files in a private temporary folder, which both sides can map into
memory.

| Param | |
|---|---|
| `input`, `width`, `height`, `format` | The source: `width`×`height` pixels, `"rgb8"` (3 bytes a pixel, rows top to bottom) |
| `output`, `out_width`, `out_height`, `out_format` | Where to write the result: `out_width`×`out_height` pixels, `"0rgb32"` (4 bytes a pixel, little-endian `0x00RRGGBB`). The file is already that size; write into it |
| `rect` | `[x0, y0, x1, y1]`: the part of the source, in its pixels, the output covers. A webtoon is asked for in bands, each with a few rows of source around it |
| `style` | `"lines"` (manga and webtoon line art) or `"soft"` (colour comics), by the book's mode or `upscale_style` in settings.toml |
| `strength` | `"gentle"`, `"normal"` or `"strong"`, from `upscale_strength` in settings.toml (Leaf 0.0.5 and later; treat it as `"normal"` when it's missing). An adapter with one way of upscaling can ignore it |

Reply `{ "written": <bytes> }` once the output is written, or an error.
leaf9000 checks `written`, waits up to 10 seconds, and on any problem scales the
page itself and stops asking until upscaling is switched on again. Pages are
asked for one at a time, from leaf9000's page thread.

## Example exchange

```
→ {"method":"initialize","params":{"leaf":"0.0.7","leaf9000":"0.0.7","protocol":3,"permissions":[]}}
→ {"id":1,"method":"book.open","params":{"path":"/Users/me/One Piece 01.cbr"}}
← {"id":1,"result":{"pages":["001.jpg","002.jpg"],"name":"One Piece 01"}}
→ {"id":2,"method":"book.page","params":{"path":"/Users/me/One Piece 01.cbr","page":0,"name":"001.jpg"}}
← {"id":2,"result":{"file":"/tmp/rar-pages/001.jpg"}}
```

(`→` is leaf9000 to the adapter, `←` is the adapter to leaf9000.)


## The 9000 family

leaf9000 is one of five sisters: text9000, splice9000, wave9000, grace9000 and
leaf9000. Their adapters all speak the same **family protocol**, from the
shared [core9000](https://github.com/solacecasino/core9000).
leaf9000's own requests, above, come on top.

**What's the same in every sister:**

- The framing: JSON lines, with requests that have an `id`.
- From the app: `initialize`, `command`, `event` and `shutdown`.
- From the adapter: `log`, `notify`, `panel.set` and the `ui.*` prompts.

**`initialize` also carries these fields:**

| Field | What it is |
|---|---|
| `app` | `"leaf9000"` |
| `version` | The app's version |
| `family` | The family protocol's version, `1` |
| `sisters` | All five apps' names |

The fields adapters already read stay as they were.

**Two more requests, answered by every sister:**

| Method | Params | Result | Permission |
|---|---|---|---|
| `sisters.list` | | `[{"name", "kind", "installed", "opens"}]`: each sister, whether it's installed, and the file extensions it opens | none |
| `sister.open` | `app` (a sister's name), `paths` | `null`. Opens the files in that sister app, or says it isn't installed and where to get it | none |

**`apps` in adapter.toml.** An adapter made for some sisters says which:

```toml
apps = ["splice9000", "wave9000"]
```

Any other sister refuses to install it, and says which apps it's for. Left
out, any sister that installs it runs it. An adapter that uses only the
family messages can name all five.

## Versions

`initialize` says which protocol version leaf9000 speaks. New versions only add
things, so an adapter written for an older version keeps working.

- **1** (Leaf 0.0.1): commands, events, opening books.
- **2** (Leaf 0.0.2): `mode` in `book.get`. An adapter made for version 1
  works unchanged.
- **3** (Leaf 0.0.4): `filters = ["upscale"]`, the `book.pages` permission,
  `upscale.ready` and `page.upscale`. Older adapters work unchanged.

Leaf 0.0.2 also refuses adapters whose program was built for another kind of
computer, ignores (and logs) any message over 96 MB, and moves a removed
adapter to the Trash.
