# ratto

Ratatui-powered terminal primitives for shell dashboards. The binary is `rat`.

`ratto` is a small CLI in the spirit of [gum](https://github.com/charmbracelet/gum),
built for one job gum doesn't cover: **scripts that act as live dashboards** —
watching long-running jobs, rendering progress, and repainting flicker-free.
It keeps gum's scripting ergonomics (results on stdout, UI on the terminal,
meaningful exit codes) and adds the terminal-control plumbing you'd otherwise
hand-roll in every watcher script.

*Ratto* is Italian for rat — a nod to [ratatui](https://ratatui.rs), which
does the rendering under the hood (this project is not affiliated with
ratatui).

```sh
# The pitch, in one line: a flicker-free dashboard loop with zero escape codes.
rat watch --interval 2s -- ./render-status.sh
```

## Install

```sh
cargo install ratto
rat completion bash > ~/.local/share/bash-completion/completions/rat
rat completion fish > ~/.config/fish/completions/rat.fish   # zsh/powershell/elvish too
```

Works in any shell; examples here are plain bash, and [`examples/`](examples/)
has full scripts in bash, zsh, fish, and PowerShell. Synchronized-output repainting
uses terminal mode 2026 (Ghostty, Kitty, Alacritty, WezTerm, iTerm2, Windows
Terminal, …). Terminals without it just ignore the escapes — everything still
works. Check yours with `rat doctor`.

## The dashboard toolkit

### `rat watch` — run a command on an interval, repaint in place

```sh
rat watch --interval 2s -- ./status.sh        # flicker-free live view
rat watch --clear -- ./status.sh              # wipe the screen first, atomically
rat watch --once -- ./status.sh               # render one frame
rat watch --shell -- 'date; df -h | head -3'  # through sh -c
```

Cursor hiding, synchronized frames, redraw-only-on-change, height capping,
and terminal restore on exit/ctrl-c are all built in. ANSI colors from the
child pass through untouched. Piped output degrades to plain text, so
`rat watch ... | tee log` stays readable.

Every tick, the child runs with `RAT_WIDTH` and `RAT_HEIGHT` set to the
current terminal size, so scripts can adapt their layout (branch on width,
or just pass `--fit` to `rat join`) and re-adapt live on resize. The child
also gets `RAT_APPEARANCE` set to the parent's light/dark verdict, so it
inherits the theme instead of asking the terminal itself — which it must
not do while `watch` owns the keyboard.

While watching: `q` quits, and `v` (or Enter) opens the full untruncated
frame in your pager — resolved bat-style from `RAT_PAGER`, then `PAGER`,
then `less` (with `-R` ensured so colors survive; quit the pager and the
watch resumes). On Windows, when `less` isn't installed the stock
`more.com` steps in. When output is taller than the screen, the truncation
line says so: `… 12 more lines · v views all · q quits`.

### `rat frame` — flicker-free repaint for script-owned loops

When you want your own loop, pipe each frame's content through `rat frame`:

```sh
while true; do
    {
        rat style --bold --foreground 212 'My Dashboard'
        rat bar --label build --value "$done" --total "$total"
    } | rat frame
    sleep 2
done
rat frame --finish   # show the cursor again when done
```

Unchanged frames write nothing; changed frames repaint in place; a terminal
resize forces a clean repaint. `rat frame begin` / `rat frame end` emit raw
synchronized-output escapes for full manual control.

### `rat bar` — progress bars without the arithmetic

```sh
rat bar --label 'release recovery' --value 1242 --total 1288 --state running
# release recovery                   ██████████████████████████████░░  1242/1288  96.4%  running
```

Batch mode reads `label<TAB>value<TAB>total[<TAB>state]` rows and aligns one
label column automatically:

```sh
printf 'build\t8\t10\ttests\ndeploy\t2\t10\twaiting\n' | rat bar --width 20
# build  ████████████████░░░░   8/10  80.0%  tests
# deploy ████░░░░░░░░░░░░░░░░   2/10  20.0%  waiting
```

An explicit `--label-width` pins the label column instead, so bars from
separate `rat bar` invocations line up too.

Color by completion band instead of picking colors in the caller, or animate
an unknown total:

```sh
rat bar --value 45 --thresholds '33:196,66:214,100:42'   # red → amber → green
rat bar --indeterminate --tick $i --width 16              # moving block
```

Presets: `--preset blocks|shade|ascii|line|dots`.

### `rat table` — columns without the arithmetic

A layout filter: tab-separated rows in, aligned columns out. Widths are
measured in display cells, so cells styled by `rat style` or `rat bar`
line up correctly — escapes are free and wide glyphs count double, which
is exactly what `column -t` and `printf '%-27s'` get wrong.

```sh
printf 'build\t8/10\tpassing\ndeploy\t2/10\twaiting\n' | rat table
# build   8/10  passing
# deploy  2/10  waiting
```

Per-column configuration is a positional comma list — an empty entry or a
short list keeps that column's default (auto width, left, truncate):

```sh
ps -o pid=,etime=,command= | tr -s ' ' '\t' |
    rat table --align r,r --widths ,,24
# 42  03:06  cargo nextest run --no-…
#  7  00:12  git push

printf 'Worktree\tfix/layout @ 47dfd63 with a very long description\n' |
    rat table --widths 10,28 --overflow ,wrap
# Worktree    fix/layout @ 47dfd63 with a
#             very long description
```

An explicit width is the column, so bars and tables from separate
invocations share an edge: `rat table --widths 27 --separator ' '` lines up
with `rat bar --label-width 27`.

### `rat join` — blocks side by side

Compose whole blocks: each positional argument (or `--file`, with `-` for
stdin) is a block, padded to its own widest line and joined row by row.

```sh
rat join --gap 2 "$(rat style --border rounded 'left panel')" \
                 "$(rat style --border rounded 'right')"
# ╭──────────╮  ╭─────╮
# │left panel│  │right│
# ╰──────────╯  ╰─────╯
```

Capture blocks with `"$(…)"` in bash/zsh, `(… | string collect)` in fish,
and `(… | Out-String)` in PowerShell. `--vertical` stacks instead, with
`--gap` blank lines between; `--align` takes top/middle/bottom beside and
left/center/right stacked.

Add `--fit` for responsive dashboards: when the joined width would exceed
the available width, the blocks stack vertically instead. Available width
resolves from `--max-width`, then `RAT_WIDTH` (which `rat watch` sets for
its children), then the terminal; with no signal at all the blocks stay
side by side, so plain pipelines remain deterministic.

### `rat spark` — sparklines

```sh
rat spark 3 1 4 1 5 9 2 6          # ▃▁▄▁▅█▂▅
seq 1 20 | rat spark --spark-color 212
```

### `rat duration` / `rat date` — time, portably

```sh
rat duration 5548                   # 1h 33m
rat duration --format clock 5592    # 01:33:12
rat duration --seconds 1h33m        # 5580

rat date --epoch 2026-07-26T12:00:00Z        # 1785067200 (replaces BSD date -j)
rat date --format '%l:%M %p' 1785067200      # 5:00 AM    (replaces date -r)
rat date --relative 1785067200               # in 2h 39m
rat date --since $start_epoch                # seconds elapsed, for ETA math
```

Same flags on macOS and Linux — no more `date -j -u -f '%Y-%m-%dT%H:%M:%SZ'`.

### `rat style` / `rat log` — styled text

```sh
rat style --bold --foreground 212 'Deploy status'
rat style --foreground '#04b575' 'ok'        # hex, 256 index, or names
rat log --level warn 'disk space low'        # WARN disk space low (stderr)
rat log --time '%H:%M:%S' --level info up    # timestamped
```

`style` also owns the box model — borders, padding, margin, a title in the
top border, and a pinned content width:

```sh
rat style --border rounded --title Deploy --padding '0 1' 'status: green'
# ╭─ Deploy ──────╮
# │ status: green │
# ╰───────────────╯
```

Borders come in `rounded`, `normal`, `thick`, `double`, and `ascii`;
`--border-color` styles the frame without touching the content, and the
title is inserted verbatim, so a pre-styled title
(`--title "$(rat style --bold Deploy)"`) keeps its own look. `--padding`
and `--margin` take CSS shorthand (`'1'`, `'0 2'`, `'1 2 3 4'`). With a
border, the painted width is the content `--width` plus horizontal padding
plus two. `NO_COLOR` governs color, not glyphs — borders keep their box
characters; `--border ascii` is the dumb-terminal opt-out. To draw a box
around *already styled* content (say, colored status lines), add
`--no-strip-ansi` so the input's own escapes survive the trip.

Colors survive command substitution — capability is detected from the
terminal, never from stdout, so `banner=$(rat style --bold hi)` keeps its
escapes even though stdout is a pipe. (This is the opposite of
`grep --color=auto`, on purpose: capturing styled text is the whole point.)

Under the default `--color auto`, output goes plain only when:

- there is no terminal at all — `/dev/tty` cannot be opened and stderr is
  not a tty (cron, CI runners, fully detached processes);
- `NO_COLOR` is set (wins over everything, including `CLICOLOR_FORCE`);
- `CLICOLOR=0` is set (unless `CLICOLOR_FORCE` overrides it);
- `CI` is set — CI logs are treated as not-a-terminal;
- `TERM` is `dumb` or names no color support — or, on unix, is unset
  (native Windows consoles never set `TERM` and get full color).

`--color always` and `--color never` beat the environment entirely: an
explicit flag outranks ambient variables, so `always` colors at full
`TERM` depth even under `NO_COLOR` or in CI, and `never` always strips.
To strip ANSI coming from *other* programs, pipe through a bare
`rat style`: input escapes are removed by default and an empty style adds
nothing back.

### Light and dark themes

`--appearance light|dark|auto` (global, default `auto`, also read from
`RAT_APPEARANCE`) selects the palette behind the semantic color tokens
below. Under `auto`, `rat` asks the terminal for its background color at
startup and falls back to `COLORFGBG`, then to dark. The question is
only asked when stderr is a terminal and the process is in the foreground,
so redirected or backgrounded runs simply use the fallback. Passing
`--appearance` alongside `--color never` (or under `NO_COLOR`) is accepted
and silently does nothing — output is plain either way, which composes
better in scripts than a warning would.

Every flag that takes a color (`--foreground`, `--background`,
`--border-color`, `--fill-color`, `--empty-color`, `--spark-color`, and
each half of `--thresholds`) accepts these token names in addition to
literal colors; each name resolves through the selected palette
(`on-accent` is black on the dark accent and white on the light one):

| Token | Meaning |
| --- | --- |
| `accent` | the brand highlight: bar fill and prompts |
| `on-accent` | text drawn *on* `accent` |
| `muted` | secondary text and the unfilled part of a bar |
| `border` | box and frame rules |
| `ok` | healthy / passing |
| `warn` | needs attention |
| `error` | failing |
| `debug` | the `DEBU` log tag |
| `info` | the `INFO` log tag |
| `fatal` | the `FATA` log tag |
| `selection` | the row under the cursor in `rat choose` and `rat filter` |
| `match` | the matched characters in `rat filter` |
| `cursor` | the `rat input` caret cell — the terminal's default foreground |
| `placeholder` | placeholder text in `rat input` and `rat filter` — the terminal's default foreground, drawn faint |

`cursor` and `placeholder` resolve to the terminal's default foreground in
both palettes, so naming them in `--foreground` yields uncolored text;
placeholder text is set apart by its faint attribute rather than a hue.

`--empty-color`'s default is the `muted` token rather than a literal
index, and `--fill-color`'s default is `accent`. `rat doctor` reports the
resolved appearance and where it came from, in both text and `--json`.

On unix, `rat watch` also follows the terminal while it runs. With
`--appearance auto` and a terminal that announces theme changes — Ghostty,
kitty, or tmux 3.7+ passing one through — switching your system or
terminal between light and dark repaints the dashboard and re-renders its
children in the new palette, without a restart. `rat` re-measures the
terminal's colors when it is told something changed, so a terminal whose
colors are pinned independently of the desktop theme keeps the palette
that matches what is actually on screen.

Opting out is the same pin as everywhere else: `--appearance light|dark`
or `RAT_APPEARANCE` fixes the palette for the run. Nothing is subscribed
to at all under `--color never`, `NO_COLOR`, `CI`, `--once`, or when
output is piped. Two limits worth knowing: a change that happens while the
pager (`v`) has the screen is picked up at the *next* change after you
leave the pager, and on Windows a `watch` session keeps the appearance it
resolved at startup.

While the dashboard runs, `rat watch` asks the terminal to announce theme
changes, and tells it to stop before exiting — on `q`, Ctrl-C, or a
signal. If a session is killed outright (`kill -9`, a terminal window
crash), the terminal can keep announcing changes to whatever runs next;
`printf '\033[?2031l'` or `reset` clears it.

## Interactive prompts

The gum staples, rendering to `/dev/tty` so stdout stays clean:

```sh
fruit=$(rat choose apple banana cherry)
names=$(rat choose --no-limit alice bob carol)   # space selects, enter confirms
rat confirm 'Ship it?' && deploy                 # exit 0 = yes, 1 = no
name=$(rat input --placeholder 'Your name')
pw=$(rat input --password)
branch=$(git branch --format='%(refname:short)' | rat filter)
rat spin --title 'Building...' -- cargo build    # child exit code passes through
```

Exit codes everywhere: `0` success, `1` no selection / negative / error,
`2` usage error, `124` timeout (`--timeout 30s`), `130` ctrl-c, and
`rat spin` forwards the child's code.

## A complete dashboard

```sh
#!/usr/bin/env bash
render() {
    rat style --bold --foreground accent 'Build pipeline'
    rat style --faint "$(date)"
    echo
    printf 'compile\t%s\t128\ntest\t%s\t96\n' "$compiled" "$tested" |
        rat bar --thresholds '50:warn,100:ok'
    echo
    rat log --level info "last artifact $(rat date --relative "$last_epoch")"
}

case "${1:-}" in
    --render) render ;;
    *) exec rat watch --clear --interval 2s -- "$0" --render ;;
esac
```

Runnable versions of this — plus the interactive prompts chained together —
live in [`examples/`](examples/) for bash, zsh, fish, and PowerShell.

## Differences from gum

`rat` is not gum-complete, on purpose. It is gum's scripting primitives plus
the dashboard toolkit above.

- **Not ported:** `format`, `write`, `file`, `pager` — none of them earn
  their keep in a dashboard script.
- **Added:** `bar`, `spark`, `watch`, `frame`, `doctor`, `duration`, `date`,
  `table`.
- **`rat table` is a layout filter**, not gum's interactive row picker — no
  selection or sorting, and per-column config is positional comma lists
  (`--widths 27,,8`).
- **Named colors are accepted** (`--foreground red`); gum silently drops
  them — and so are semantic token names (`accent`, `ok`, `warn`, …) that
  follow the terminal's light or dark background.
- **UI goes to `/dev/tty`** with an stderr fallback, so prompts survive
  `2>/dev/null`; gum writes UI to stderr only.
- **`rat filter` quits on one Esc press**; gum needs two.
- **`rat spin` uses pipes, not a PTY**; children that only colorize on a tty
  get `CLICOLOR_FORCE=1` instead.
- **`--color always` trusts `TERM`** even when piped, so forced color keeps
  its full depth in scripts and CI.

## Windows

ratto builds and runs on Windows (PowerShell, Windows Terminal, conhost,
or ssh'd into from any terminal). Native sessions get full color with no
`TERM` needed — a bare Windows console reports truecolor — and light/dark
is detected where the terminal answers the background query (Windows
Terminal does; others fall back to dark). The UI stream uses `CONOUT$`
where unix uses `/dev/tty`; `watch --shell` runs through `%COMSPEC% /C`;
`rat` enables VT processing on the console itself, so escapes are
processed even in legacy conhost, which simply ignores the synchronized-
output mode it doesn't implement (Windows Terminal supports it). Three
notes:

- The `v` key in `watch` prefers `less.exe` on PATH (Git for Windows,
  scoop, and winget all provide one) and falls back to the stock `more.com`,
  with the console held in UTF-8 while the pager runs so glyphs render
  correctly; set `RAT_PAGER` to override.
- `rat frame`'s default state file is keyed per terminal session; when
  running several dashboards in one console session, pass `--state`.
- Following the terminal's light/dark switch while `watch` runs is
  unix-only; on Windows a session keeps the appearance it resolved at
  startup.

## Exit codes

| Situation | Code |
|---|---|
| Success | 0 |
| Esc / nothing selected / `confirm` no / error | 1 |
| Usage error | 2 |
| `spin` child exited N | N |
| `--timeout` expired | 124 |
| Ctrl-C | 130 |
