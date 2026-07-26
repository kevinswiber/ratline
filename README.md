# ratline

Ratatui-powered terminal primitives for shell dashboards. The binary is `rat`.

`ratline` is a small CLI in the spirit of [gum](https://github.com/charmbracelet/gum),
built for one job gum doesn't cover: **scripts that act as live dashboards** —
watching long-running jobs, rendering progress, and repainting flicker-free.
It keeps gum's scripting ergonomics (results on stdout, UI on the terminal,
meaningful exit codes) and adds the terminal-control plumbing you'd otherwise
hand-roll in every watcher script.

The name is nautical: ratlines are the rope rungs sailors climb a ship's
rigging by. This one is also a nod to [ratatui](https://ratatui.rs), which
does the rendering under the hood (this project is not affiliated with
ratatui).

```sh
# The pitch, in one line: a flicker-free dashboard loop with zero escape codes.
rat watch --interval 2s -- ./render-status.sh
```

## Install

```sh
cargo install ratline
rat completion bash > ~/.local/share/bash-completion/completions/rat
rat completion fish > ~/.config/fish/completions/rat.fish   # zsh/powershell/elvish too
```

Works in any shell; examples here are plain bash, and [`examples/`](examples/)
has full scripts in bash, zsh, and fish. Synchronized-output repainting
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

Color by completion band instead of picking colors in the caller, or animate
an unknown total:

```sh
rat bar --value 45 --thresholds '33:196,66:214,100:42'   # red → amber → green
rat bar --indeterminate --tick $i --width 16              # moving block
```

Presets: `--preset blocks|shade|ascii|line|dots`.

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
- `TERM` is `dumb`, unset, or names no color support.

`--color always` forces color (trusting `TERM` even when piped) and
`--color never` strips it; both beat the environment except `NO_COLOR`.
To strip ANSI coming from *other* programs, pipe through a bare
`rat style`: input escapes are removed by default and an empty style adds
nothing back.

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
    rat style --bold --foreground 212 'Build pipeline'
    rat style --faint "$(date)"
    echo
    printf 'compile\t%s\t128\ntest\t%s\t96\n' "$compiled" "$tested" |
        rat bar --thresholds '50:214,100:42'
    echo
    rat log --level info "last artifact $(rat date --relative "$last_epoch")"
}

case "${1:-}" in
    --render) render ;;
    *) exec rat watch --clear --interval 2s -- "$0" --render ;;
esac
```

Runnable versions of this — plus the interactive prompts chained together —
live in [`examples/`](examples/) for bash, zsh, and fish.

## Differences from gum

`rat` is not gum-complete, on purpose. It is gum's scripting primitives plus
the dashboard toolkit above.

- **Not ported:** `table`, `join`, `format`, `write`, `file`, `pager` — none
  of them earn their keep in a dashboard script.
- **Added:** `bar`, `spark`, `watch`, `frame`, `doctor`, `duration`, `date`.
- **Named colors are accepted** (`--foreground red`); gum silently drops them.
- **UI goes to `/dev/tty`** with an stderr fallback, so prompts survive
  `2>/dev/null`; gum writes UI to stderr only.
- **`rat filter` quits on one Esc press**; gum needs two.
- **`rat spin` uses pipes, not a PTY**; children that only colorize on a tty
  get `CLICOLOR_FORCE=1` instead.
- **`--color always` trusts `TERM`** even when piped, so forced color keeps
  its full depth in scripts and CI.
- Box-model styling (`--border`, `--margin`, padding, alignment) is not
  implemented.

## Exit codes

| Situation | Code |
|---|---|
| Success | 0 |
| Esc / nothing selected / `confirm` no / error | 1 |
| Usage error | 2 |
| `spin` child exited N | N |
| `--timeout` expired | 124 |
| Ctrl-C | 130 |
