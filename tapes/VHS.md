# VHS — Terminal Video Reference

VHS is Charmbracelet's "CLI home video recorder": you write a script (a `.tape`
file) that drives a virtual terminal, and VHS renders it to a GIF, MP4, WebM,
PNG frame sequence, or plain-text "golden file". It is the standard way to
produce the demo animations in this repository's README and release notes.

- Project: <https://github.com/charmbracelet/vhs>
- Official README/command reference: <https://github.com/charmbracelet/vhs>
- Offline reference: `vhs manual` (printed by the binary itself)
- Syntax highlighting: [tree-sitter-vhs](https://github.com/charmbracelet/tree-sitter-vhs) (Neovim, Emacs, Helix, …)
- CI: [charmbracelet/vhs-action](https://github.com/charmbracelet/vhs-action)

Tape files in this repo live in [`tapes/`](./) with `.tape` extensions and are
rendered to `tapes/out/*.gif` (see [Project conventions](#project-conventions)
at the bottom).

> **Version note.** This machine has VHS **v0.11.0** installed. The commands
> and settings marked ⚠️ below (`Env`, `Margin`, `MarginFill`, `WindowBar`,
> `BorderRadius`, `CursorBlink`, `LoopOffset`, `Home`, `End`) were added in
> **newer** releases and will be ignored by v0.11.0. Everything else works on
> v0.11.0. Confirm with `vhs manual`.

---

## Table of Contents

1. [Installation](#installation)
2. [Quick start](#quick-start)
3. [Tape file format](#tape-file-format)
4. [Comments](#comments)
5. [Time values](#time-values)
6. [String escaping](#string-escaping)
7. [Command reference](#command-reference)
   - [Output](#output)
   - [Require](#require)
   - [Set (settings)](#set-settings)
   - [Type](#type)
   - [Key commands](#key-commands)
   - [Ctrl / Alt / Shift](#ctrl--alt--shift)
   - [Arrow keys](#arrow-keys)
   - [Scroll Up / Down](#scroll-up--down)
   - [Wait](#wait)
   - [Sleep](#sleep)
   - [Hide / Show](#hide--show)
   - [Screenshot](#screenshot)
   - [Copy / Paste](#copy--paste)
   - [Env](#env)
   - [Source](#source)
8. [CLI subcommands](#cli-subcommands)
9. [Continuous integration & golden files](#continuous-integration--golden-files)
10. [Project conventions](#project-conventions)

---

## Installation

VHS needs two runtime dependencies on your `PATH`:

- [`ttyd`](https://github.com/tsl0922/ttyd) — the terminal emulator VHS drives.
- [`ffmpeg`](https://ffmpeg.org) — encodes GIF/MP4/WebM output.

Install VHS itself:

```sh
# macOS / Linux
brew install vhs

# Arch Linux
pacman -S vhs

# Nix
nix-env -iA nixpkgs.vhs

# Windows
winget install charmbracelet.vhs
# or
scoop install vhs

# Ubuntu / Debian (after adding the Charm apt repo, see the official README)
sudo apt install vhs ffmpeg

# Fedora / RHEL (after adding the Charm yum repo)
sudo yum install vhs ffmpeg

# Via Go
go install github.com/charmbracelet/vhs@latest
```

Or run it in Docker with the dependencies included:

```sh
docker run --rm -v "$PWD:/vhs" ghcr.io/charmbracelet/vhs <cassette>.tape
```

Check everything is wired up:

```sh
vhs --version
vhs themes   # dump the list of built-in color themes
vhs manual   # print the full command reference
```

---

## Quick start

```sh
# Scaffold a tape file.
vhs new demo.tape

# Render it.
vhs demo.tape          # reads Output <path> from the tape
vhs demo.tape out.gif  # or force the output path on the CLI
```

Minimal example:

```elixir
Output demo.gif

Set FontSize 32
Set Width 1200
Set Height 600

Type "echo 'Welcome to VHS!'"
Sleep 500ms
Enter
Sleep 5s
```

The tape is a plain-text list of commands executed top-to-bottom against a
fresh virtual terminal.

---

## Tape file format

- A tape is a sequence of one command per line, executed in order.
- Commands are parsed by a small lexer; each line starts with a command name
  followed by its arguments.
- Line continuations are not supported — keep one command per line.
- The general shape of a command is:

  ```
  Command[@<speed>] [arguments]
  ```

  The optional `@<speed>` is a per-command timing override (used by `Type`,
  key commands, and scroll commands).

- **Settings and `Require` must appear at the top of the file**, before any
  non-setting/non-output command. The one exception is `Set TypingSpeed`,
  which may appear anywhere. Settings applied too late are silently ignored.

---

## Comments

`#` begins a comment to the end of the line. Use them liberally — they make the
demo self-documenting:

```elixir
# Where should we write the GIF?
Output demo.gif

# Set up a 1200x600 terminal with a 46px font.
Set FontSize 46
Set Width 1200
Set Height 600
```

---

## Time values

Durations are accepted in a number of formats. They are used by `Sleep`,
`Set TypingSpeed`, the `@<speed>` override, `Wait` timeouts, and `Set WaitTimeout`.

```elixir
Sleep 0.5      # 500ms
Sleep .5       # also 500ms
Sleep 100ms    # 100 milliseconds
Sleep 2        # 2 seconds
Sleep 1s       # 1 second
Sleep 1m       # 1 minute
Sleep 1h       # 1 hour
```

- A bare number is seconds.
- Suffixes: `ms` (milliseconds), `s` (seconds), `m` (minutes), `h` (hours).

---

## String escaping

`Type` (and `Copy`) take a string argument. You may use:

- Double quotes: `Type "hello"`
- Backtick-quoted strings for text that contains quotes, e.g. shell variable
  assignment: ``Type `VAR="Escaped"` ``

In practice, prefer backticks whenever your text contains `"` or `'` so you
don't have to escape anything.

---

## Command reference

### Output

Sets where (and in what format) the recording is rendered. You may declare
**multiple** `Output` lines to render several formats at once.

```elixir
Output out.gif            # animated GIF
Output out.mp4            # H.264 MP4
Output out.webm           # WebM
Output frames/            # a directory of PNG frames (trailing slash required)
Output golden.ascii       # plain-text capture (great for CI golden files)
Output golden.txt
```

> **Path gotcha.** Paths are resolved relative to the **current working
> directory** (where you run `vhs`), not relative to the tape file. Also, the
> lexer uses `/` as the regex delimiter, so a path that *starts* with `/` is
> rejected. Use relative paths, and prefix them with `./` when in doubt:
> `Output ./tapes/out/status.gif` works; `Output /abs/path.gif` does not.

### Require

Fails the tape early if a program is missing from `$PATH`. Must appear at the
top of the file, before any non-setting/non-output command.

```elixir
Require gtm
Require gtmd
Require ffmpeg
```

### Set (settings)

`Set <Setting> <Value>` configures the virtual terminal. All settings must
appear before the first non-setting/non-output command (except `TypingSpeed`).

| Setting            | Type      | Default          | Example                              |
| ------------------ | --------- | ---------------- | ------------------------------------ |
| `Shell`            | string    | `bash`           | `Set Shell "fish"`                   |
| `FontFamily`       | string    | (VHS default)    | `Set FontFamily "Monoflow"`          |
| `FontSize`         | number    | `20`             | `Set FontSize 32`                    |
| `Width`            | number    | `1200`           | `Set Width 1200`                     |
| `Height`           | number    | `600`            | `Set Height 600`                     |
| `LetterSpacing`    | number    | `0`              | `Set LetterSpacing 2`                |
| `LineHeight`       | number    | `1`              | `Set LineHeight 1.3`                 |
| `TypingSpeed`      | duration  | `20ms`           | `Set TypingSpeed 50ms`               |
| `Theme`            | name/JSON | (VHS default)    | `Set Theme "Catppuccin Mocha"`       |
| `Padding`          | number    | `36`             | `Set Padding 20`                     |
| `Margin` ⚠️        | number    | `0`              | `Set Margin 20`                      |
| `MarginFill` ⚠️    | color     | (VHS default)    | `Set MarginFill "#674EFF"`           |
| `WindowBar` ⚠️     | string    | (none)           | `Set WindowBar "Colorful"`           |
| `BorderRadius` ⚠️  | number    | `0`              | `Set BorderRadius 10`                |
| `Framerate`        | number    | `60`             | `Set Framerate 60`                   |
| `PlaybackSpeed`    | float     | `1.0`            | `Set PlaybackSpeed 2.0`              |
| `LoopOffset` ⚠️    | num / pct | `0`              | `Set LoopOffset 50%`                 |
| `CursorBlink` ⚠️   | bool      | `true`           | `Set CursorBlink false`              |
| `WaitTimeout`      | duration  | `15s`            | `Set WaitTimeout 10s`                |
| `WaitPattern`      | regexp    | `>$`             | `Set WaitPattern "ready$"`           |

Notes on individual settings:

- **`Set Shell`** — which shell the virtual terminal runs. Defaults to `bash`.
- **`Set FontFamily`** — use a monospace font installed on your system, e.g.
  `"Monoflow"`, `"JetBrainsMono Nerd Font"`, `"Fira Code"`.
- **`Set Theme`** — a built-in theme name (see `vhs themes`) or an inline JSON
  object with the 16 base-16 colors plus `foreground`, `background`,
  `selection`, and `cursor`:

  ```elixir
  Set Theme { "name": "Whimsy", "black": "#535178", "red": "#ef6487", "green": "#5eca89", "yellow": "#fdd877", "blue": "#65aef7", "magenta": "#aa7ff0", "cyan": "#43c1be", "white": "#ffffff", "brightBlack": "#535178", "brightRed": "#ef6487", "brightGreen": "#5eca89", "brightYellow": "#fdd877", "brightBlue": "#65aef7", "brightMagenta": "#aa7ff0", "brightCyan": "#43c1be", "brightWhite": "#ffffff", "background": "#29283b", "foreground": "#b3b0d6", "selection": "#3d3c58", "cursor": "#b3b0d6" }
  ```

- **`Set TypingSpeed`** — delay per keystroke. Example: `0.1` = 100ms per key.
  This is the one setting that may be changed anywhere in the tape, and it can
  be overridden per-command with `@<speed>`:

  ```elixir
  Set TypingSpeed 0.1
  Type "100ms delay per character"
  Type@500ms "500ms delay per character"
  ```

- **`Set WindowBar`** — values: `Colorful`, `ColorfulRight`, `Rings`,
  `RingsRight`.
- **`Set LoopOffset`** — choose the GIF's first frame (used for previews).
  Accepts a frame number or a percentage: `Set LoopOffset 5` or `Set LoopOffset 50%`.
- **`Set PlaybackSpeed`** — `0.5` = half speed (2× slower output), `1.0` =
  normal, `2.0` = twice as fast.

### Type

Emulates typing into the terminal. Takes a string; honors `Set TypingSpeed` and
the per-command `@<time>` override.

```elixir
Type "gtm status"
Type@500ms "Slow down there, partner."
Type `gtm queue-add --position 2 ~/music`
```

### Key commands

All key commands share the shape `Key[@<time>] [count]` — an optional timing
override and an optional repeat count (the key is pressed every `<time>`).

```elixir
Enter            # press Enter once
Enter 2          # press Enter twice
Tab@500ms 2      # press Tab twice, 500ms apart
Backspace 18     # backspace 18 times
```

| Command     | Key                       |
| ----------- | ------------------------- |
| `Enter`     | Return / Enter            |
| `Backspace` | Backspace                 |
| `Delete`    | Delete                    |
| `Insert`    | Insert                    |
| `Tab`       | Tab                       |
| `Space`     | Space bar                 |
| `Escape`    | Esc                       |
| `Up`        | Up arrow                  |
| `Down`      | Down arrow                |
| `Left`      | Left arrow                |
| `Right`     | Right arrow               |
| `PageUp`    | Page Up                   |
| `PageDown`  | Page Down                 |
| `Home` ⚠️    | Home                      |
| `End` ⚠️     | End                       |

Examples:

```elixir
Escape 2           # press Esc twice (e.g. close a TUI overlay)
Down 3             # move the cursor down three rows
PageDown 5         # page down five times
```

### Ctrl / Alt / Shift

Combine a modifier with one or more keys. Chained modifiers are supported:

```elixir
Ctrl+C          # interrupt
Ctrl+D          # EOF
Ctrl+L          # clear screen
Ctrl+R          # reverse search
Ctrl+Alt+L      # ctrl + alt + L
Shift+Tab       # reverse tab
```

`@<time>` and a repeat count are also accepted here (e.g. `Ctrl+C@100ms 2`).

### Scroll Up / Down

Scroll the terminal *viewport* (not the cursor) by rows:

```elixir
ScrollUp 10
ScrollDown 4
ScrollDown@100ms 12
```

### Wait

Wait until a pattern matches, so you can capture slow operations (spinners,
long commands) without arbitrary `Sleep`s.

```
Wait[+<scope>][@<timeout>] /<regex>/
```

- Default regex: `>$`
- Default timeout: `15s`
- Default scope: `Line`
- Scopes: `+Line` (only the last line) or `+Screen` (the whole viewport)

```elixir
Wait
Wait            /World/
Wait+Screen     /World/
Wait+Line       /World/
Wait@10ms       /World/
Wait+Line@10ms  /World/
Wait /loaded/   # wait until "loaded" appears on the current line
```

Typical gtm usage — wait for the prompt after launching the daemon:

```elixir
Type "gtmd"
Enter
Wait+Screen /listening/
```

### Sleep

Keep capturing frames for a fixed duration. Use it to "admire" output, or to
let a spinner/loading state show.

```elixir
Sleep 0.5
Sleep 2
Sleep 100ms
Sleep 1s
Sleep 1m
```

### Hide / Show

`Hide` stops frame capture; `Show` resumes it. Use to perform setup/cleanup
without the viewer seeing it (build the binary, remove temp files, start
services). Hidden commands still execute in the terminal.

```elixir
Output example.gif

# Setup (not recorded).
Hide
Type "cargo build --release && clear"
Enter
Show

# Recording.
Type 'gtm status'
Enter
Sleep 3s

# Cleanup (not recorded).
Hide
Type 'rm -rf /tmp/gtm-demo'
Enter
```

### Screenshot

Capture the current frame as a PNG. Unlike `Output` (a full recording), this
snaps a single still:

```elixir
# At any point...
Screenshot tapes/out/frame.png
```

### Copy / Paste

Interact with the system clipboard:

```elixir
Copy "https://github.com/prjctimg/gtm.rs"
Type "open "
Sleep 500ms
Paste
```

### Env ⚠️

Set an environment variable for the session (requires a VHS release newer than
v0.11.0):

```elixir
Env GTM_SOCKET "/tmp/gtm-demo.sock"
Env HELLO "WORLD"

Type "echo $HELLO"
Enter
Sleep 1s
```

### Source

Inline the commands of another tape into this one:

```elixir
Source ./tapes/config.tape
```

Handy for sharing a common header (settings + `Require` lines) across every
demo in this directory.

> **Path gotcha.** `Source` obeys the same rules as `Output`: the path is
> relative to the **current working directory** (not the tape file), and it
> must not start with `/`. When a demo lives in `tapes/` and you run `vhs`
> from the repo root, source it as `./tapes/config.tape`.

---

## CLI subcommands

| Command                     | Purpose                                                        |
| --------------------------- | -------------------------------------------------------------- |
| `vhs new <file>.tape`       | Scaffold a new tape file.                                      |
| `vhs <file>.tape`           | Render a tape (output path from the `Output` line).            |
| `vhs <file>.tape <out>`     | Render, overriding the output path.                            |
| `vhs record > cassette.tape`| Record your live keystrokes into a tape. `exit` to finish.     |
| `vhs publish <file>.gif`    | Upload to `vhs.charm.sh`; prints shareable HTML/Markdown links.|
| `vhs serve`                 | Run the built-in SSH VHS server (see env vars below).          |
| `vhs themes`                | List built-in color themes.                                    |
| `vhs manual`                | Print the full command reference.                              |
| `vhs validate`              | Validate a tape without rendering.                             |

`vhs serve` configuration (all optional):

- `VHS_PORT` — listen port (default `1976`)
- `VHS_HOST` — bind host (default `localhost`)
- `VHS_GID` / `VHS_UID` — run as this group/user
- `VHS_KEY_PATH` — SSH key path (default `.ssh/vhs_ed25519`)
- `VHS_AUTHORIZED_KEYS_PATH` — authorized keys file (default empty = public)

Remote usage once the server is up:

```sh
ssh vhs.example.com < demo.tape > demo.gif
```

---

## Continuous integration & golden files

### GitHub Actions

Hook `charmbracelet/vhs-action` into CI to keep GIFs fresh:

```yaml
- uses: charmbracelet/vhs-action@v2
  with:
    path: tapes/*.tape
```

### Golden files / integration testing

Use the `.txt` or `.ascii` output format to produce a plain-text capture that
you commit to the repo. A diff between runs then flags any regression in
terminal output — ideal for testing gtm's CLI output and TUI rendering:

```elixir
Output golden.ascii
```

```sh
vhs tapes/status.golden.tape   # regenerates golden.ascii
git diff -- golden.ascii        # did the output change?
```

---

## Project conventions

- Store demo tapes in [`tapes/`](./) as `<name>.tape`.
- Render artifacts to `tapes/out/` (gitignored) so the repo stays clean.
- **Always run `vhs` from the repository root.** Every path in a tape is
  relative to the working directory, so reference tapes as `./tapes/...`:

  ```sh
  mkdir -p tapes/out
  vhs tapes/status.tape tapes/out/status.gif
  ```

- A `tapes/config.tape` holds shared settings/`Require` lines; each demo
  `Source`s it via `Source ./tapes/config.tape`.
- Start every demo with `Require gtm` / `Require gtmd` so the tape fails early
  if the binaries aren't built.
- Use `Set Width 1200` / `Set Height 600` and `Set FontSize 32` for README
  readability.
- Prefer `Wait` over long `Sleep`s when the demo must match command completion.
- Wrap build/cleanup commands in `Hide` … `Show` so viewers only see the demo.

### Example: `tapes/status.tape`

```elixir
Source ./tapes/config.tape

Output ./tapes/out/status.gif

Hide
Type "cargo build --release 2>/dev/null && gtmd & sleep 1 && clear"
Enter
Show

Type "gtm status"
Sleep 500ms
Enter
Sleep 3s

Hide
Type "kill %1"
Enter
```

---

## Further reading

- Official repo & examples: <https://github.com/charmbracelet/vhs/tree/main/examples>
- VHS themes list: <https://github.com/charmbracelet/vhs/blob/main/THEMES.md>
- tree-sitter grammar for `.tape` files: <https://github.com/charmbracelet/tree-sitter-vhs>
- GitHub Action: <https://github.com/charmbracelet/vhs-action>
