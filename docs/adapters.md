# Leaf adapters

Leaf ships bare. An **adapter** is an optional add-on you install on
purpose, like foobar2000's components. It's an ordinary program in any
language. Leaf starts it only when it's needed and talks to it with JSON
messages, one per line, over stdin and stdout. Adapters work the same way as
[Folio's](https://github.com/solacecasino/folio/blob/main/docs/adapters.md),
so the same habits carry over.

An adapter can:

- **add commands**, shown in Leaf's menus and bindable to keys;
- **open books Leaf can't read itself**, such as `.cbr` or PDF: Leaf asks it
  for the page list, then for each page's image;
- **follow along**: be told when a book opens or the page changes, and ask
  Leaf which book is open or to turn the page.

There are no adapters yet. Leaf reads most formats itself now (RAR, 7-Zip,
PDF, EPUB, text), so adapters are for the rest, and for adding commands.

## Installing

Drop an adapter's `.zip` or folder on Leaf's window (or open it with Leaf).
Leaf shows its name, author, license, what it opens and what it may do, and
installs it only when you click **Install**: Return and Escape cancel. It
works right away, with no restart.

Installed adapters live in the `adapters` folder of the config folder
(**Leaf → Show Adapters Folder**). You can also copy an adapter folder there
by hand and choose **Leaf → Reload Adapters**. Leaf then asks you to allow it
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

[[commands]]
id = "info"                     # sent back to you when chosen
label = "Book Info"             # menu text
keys = "cmd+alt+i"              # optional default shortcut
# menu = "go"                   # optional: File, View or Go instead of Adapters
```

- `run` starts in the adapter's folder. A program given as a path
  (`./rar-pages`) is found there.
- Leaf sets `LEAF_CONFIG_DIR` to its config folder, for adapters that keep
  files of their own, and `PYTHONIOENCODING=utf-8`.
- `start = "command"` starts the adapter the first time you pick one of its
  commands or open a book it reads. `start = "launch"` starts it with Leaf.
- When Leaf quits (or reloads adapters) it sends `shutdown`, waits half a
  second, then stops the program.
- Whatever the program prints to **stderr** goes to `leaf.log`
  (**Help → Show Log**).
- An adapter that crashes doesn't take Leaf with it. Leaf logs it, says so
  in the title bar, and starts it again the next time it's needed.
- Users can bind your commands in `keys.toml`:
  `"cmd+k" = 'adapter "RAR Pages" info'`.

### Permissions

| Permission | Allows |
|---|---|
| `book.read` | `book.get`, and the `opened` and `page` events |
| `book.control` | `book.goto`, `book.open` |
| `network` | Nothing through Leaf. It tells the user the adapter goes online |

Permissions control what an adapter can do **through Leaf**. An adapter is a
normal program running as you, so `network` is an honest declaration, not a
sandbox. Only install adapters you trust; being open source is what lets
people check. Opening books (`opens`) needs no permission: Leaf only sends
the adapter the file you chose to open.

## Messages

Every message is one JSON object on one line, in UTF-8.

- A **notification** has `method` and `params` and gets no reply.
- A **request** also has an `id`. The reply has the same `id` and either
  `result` or `error` (a string).

### Leaf → adapter

| Method | Params | Notes |
|---|---|---|
| `initialize` | `leaf` (version), `protocol` (currently `2`), `permissions` | Notification, right after starting |
| `command` | `command`: the command's `id` | Notification: you picked one of its commands |
| `event` | `kind` plus fields below | Notification, if listed in `events` and you have `book.read` |
| `shutdown` | | Notification: exit soon |
| `book.open` | `path` | **Request**, for adapters with `opens`. Reply `{ "pages": ["001.jpg", ...], "name": "optional title" }` |
| `book.page` | `path`, `page` (from 0), `name` (from your list) | **Request**. Reply `{ "file": "/path/to/image" }` or `{ "base64": "..." }` with JPEG, PNG, WebP, GIF or BMP bytes |

Leaf asks for pages from a background thread, up to three at a time around
the page being read, and waits up to 30 seconds for each answer.

Events:

| `kind` | Fields |
|---|---|
| `opened` | `path`, `name`, `pages` |
| `page` | `page` (from 0), `pages` |

### Adapter → Leaf

| Method | Params | Result | Permission |
|---|---|---|---|
| `book.get` | | `{ path, name, page, pages, rtl, fit, mode }`, or `null` with no book open. `mode` is `comic`, `manga` or `webtoon` | `book.read` |
| `book.goto` | `page` (from 0) | `null` | `book.control` |
| `book.open` | `path` | `null` | `book.control` |
| `notify` | `text` | `null`. Shows a short message in the title bar | none |
| `log` | `message` | `null`. Written to `leaf.log` | none |
| `ui.confirm` | `title`, `message` | `true` or `false` from a native dialog; Return answers No | none |

## Example exchange

```
→ {"method":"initialize","params":{"leaf":"0.0.2","protocol":2,"permissions":[]}}
→ {"id":1,"method":"book.open","params":{"path":"/Users/me/One Piece 01.cbr"}}
← {"id":1,"result":{"pages":["001.jpg","002.jpg"],"name":"One Piece 01"}}
→ {"id":2,"method":"book.page","params":{"path":"/Users/me/One Piece 01.cbr","page":0,"name":"001.jpg"}}
← {"id":2,"result":{"file":"/tmp/rar-pages/001.jpg"}}
```

(`→` is Leaf to the adapter, `←` is the adapter to Leaf.)

## Versions

`initialize` says which protocol version Leaf speaks. New versions only add
things, so an adapter written for an older version keeps working.

- **1** (Leaf 0.0.1): commands, events, opening books.
- **2** (Leaf 0.0.2): `mode` in `book.get`. An adapter made for version 1
  works unchanged.

Leaf 0.0.2 also refuses adapters whose program was built for another kind of
computer, ignores (and logs) any message over 96 MB, and moves a removed
adapter to the Trash.
