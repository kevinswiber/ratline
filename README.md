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
rat watch --fullscreen -- ./status.sh         # alternate screen, restored on exit
rat watch --append -- ./status.sh             # append frames; scrollback keeps them
rat watch --once -- ./status.sh               # render one frame
rat watch --shell -- 'date; df -h | head -3'  # through sh -c
rat watch --shell=fish -- 'date; df -h | head -3'  # through fish -c
```

Cursor hiding, synchronized frames, redraw-only-on-change, height capping,
and terminal restore on exit/ctrl-c are all built in. Repaints rewrite
only the rows that actually changed, so steady dashboards stay calm —
and cheap over SSH. ANSI colors from the child pass through untouched.
Piped output degrades to plain text, so `rat watch ... | tee log` stays
readable. The interval is the quiet time between runs: a command slower
than its interval never overlaps itself — the next run simply waits its
turn.

Bare `--shell` always means the platform's own shell — `sh -c` on unix,
`%COMSPEC% /C` on Windows — and `--shell=NAME` picks another by name or
full path. A named shell gets its own dialect's flag: `cmd` gets `/C`,
`powershell` and `pwsh` get `-NoProfile -Command`, and everything else
gets `-c` (sh, bash, zsh, fish, nu all agree on it). Unknown names are
not rejected — a wrong name surfaces as the spawn error naming that
shell. PowerShell skips the profile deliberately: the child respawns
every tick, so the profile's load cost would recur at the interval and
anything it prints would land in the frame; a script that wants the
profile opts back in (`--shell=pwsh -- '. $PROFILE; Get-Thing'`), while
there would be no way to opt out.

Three screen contracts, and a flag to pick one. The default paints
inline: the frame sits under your last command, the session reads as a
transcript, and exit leaves the last frame behind. `--fullscreen`
(watch and dashboard both) takes the alternate screen instead, like
`less` or `htop`: the status row is pinned to the bottom of the
screen, and exit restores your terminal exactly as it was — the frames
never enter scrollback, so keep a frame with `S` rather than by
scrolling back. Ignored when piped. (A hard `SIGKILL` can leave the
terminal on the alternate screen; `reset` recovers.)

`--append` (watch only) is the opposite bargain: every distinct frame
is appended to the scrollback as plain lines — no status row, no
repainting, nothing ever rewritten — so the terminal's own scrollback
holds the whole history and its wheel does the scrolling. Duplicate
frames still write nothing, and rat speaks in whole lines prefixed
`rat watch: ` when it has something to say (a trigger ending, lines
dropped, the command's exit status changing). Four keys work — `q`
quits, `Ctrl-C` aborts, `S` snapshots, `?` lists exactly this — and
every viewport key is deliberately inert; the scrollback is the
viewport. This mode exists for linear readers of all kinds: in our
testing with VoiceOver in macOS Terminal, appended lines were announced
as they arrived while in-place repaints were not, and a slower
`--interval` gives each announcement room to finish. Ignored when
piped: a piped watch already appends.

Beside (or instead of) the interval, `--trigger` refreshes on an
external event. The sweet spot is two speeds — a slow heartbeat for what
only polling can see, a fast lane for what a file can announce:

```sh
# Refresh within a blink when state.json changes; poll the slow stuff
# (network checks, git state) once a minute.
rat watch -n 60s --trigger file:./state.json --trigger-debounce 1s -- ./render.sh

# Event-driven only: omit -n and nothing runs until something fires.
rat watch --trigger fifo:/tmp/rat.t -- ./render.sh   # echo go > /tmp/rat.t
```

Three sources, all repeatable: `file:PATH` stat-polls a file's mtime —
or a directory's, taken together with its immediate entries — and works
everywhere, including piped and on Windows; `fifo:PATH` reads a named
pipe you create with `mkfifo` (any write fires it, writers may come and
go); `fd:N` watches an inherited descriptor, composing with process
substitution (`--trigger fd:3 3< <(producer)`) and firing one last
notice when the descriptor ends. `fifo:` and `fd:` need an interactive
unix terminal; `file:` is the portable form.

Bursts collapse: `--trigger-debounce` (default `250ms`) turns any storm
of fires inside the window into one refresh, scheduled from the first
fire — an editor's multi-write save costs one run, and a file written
continuously still repaints once per window. A fire landing while a run
is already in flight never kills it; the fresh run starts the moment the
stale one finishes. With a trigger configured the bottom row reads
`every 60s or on trigger` (or just `on trigger`), and `?` lists the
configured sources.

Every tick, the child runs with `RAT_WIDTH` and `RAT_HEIGHT` set to the
current terminal size, so scripts can adapt their layout (branch on width,
or just pass `--fit` to `rat join`) and re-adapt live on resize. The child
also gets `RAT_APPEARANCE` set to the parent's light/dark verdict, so it
inherits the theme instead of asking the terminal itself — which it must
not do while `watch` owns the keyboard.

The command runs in the background of the watch's own loop, so every
key answers immediately — even while a slow command is still mid-run —
and the frame updates when the run finishes. While watching: `q` quits,
stopping the command it is running, and `v` (or Enter) opens the full
untruncated frame in your pager — resolved bat-style from `RAT_PAGER`,
then `PAGER`, then `less` (with `-R` ensured so colors survive; quit
the pager and the watch returns to the frame at once). On Windows, when
`less` isn't installed the stock `more.com` steps in. `?` pages the
full key reference the same way.

Every live frame's bottom row names the last time the output actually
changed and the refresh cadence, with `? help` as the one standing
hint: `since 14:03:52 · every 2s · ? help`. When output is taller than
the screen it merges into the truncation line:
`… 12 more lines · since 14:03:52 · every 2s · ? help`.

Scroll with less-style keys: `j`/`k` (or the arrows) move one line,
`d`/`u` half a window, `f`/`b` (or PgDn/PgUp) a full window, and `g`/`G`
(or Home/End) jump to the ends. The window moves over the live frame:
nothing pauses, new output keeps arriving under you, `G` sticks to the
end, and `g` (or scrolling back to the top) returns to the live view.
The bottom row keeps the live row's time and cadence around the range:
`live · since 12:07:45 · lines 9-30 of 46 · every 2s · ? help`.
With `--mouse`, the wheel drives the same scrolling — a notch is three
lines, shift+wheel a half window, a horizontal wheel the `h`/`l`
shift. Capture is opt-in because the terminal reports the whole mouse
or nothing: while rat holds it, the wheel cannot reach the terminal's
own scrollback and plain-drag selection needs the terminal's escape
hatch (usually shift). `m` hands the mouse back mid-session and takes
it again.
Scrolled lines render chopped, like a horizontally shifted view. If the
output changes shape while you're scrolled, the window rides along —
the row's total updates, a pinned window keeps tracking the end — and
if the moment you were reading slides away, step back to it with `<`.

Nothing ever pauses on its own. `p` parks the frame deliberately — the
command keeps running behind it, but nothing repaints over what you're
reading — and `Esc` or `F` return to the live tail. The paused row
stamps the moment the frame on screen was current:
`paused · at 14:03:52 · lines 2-23 of 30 · Esc resumes`. `q` quits and `S`
snapshots from either mode, and while paused `v` pages the frozen frame —
which is also where search lives: page into `less` and search there. One
deliberate divergence from `less`: Enter pages rather than scrolling one
line.

Step back in time with `<` (or `,`): each press parks on the previous
distinct frame, and `>` (or `.`) steps forward again — one more press
past the newest returns to the live view, keeping your scroll position
(`Esc` always resumes at the top). The paused row's
stamp says when the frame on screen was current, and `S` and `v` act on the
frame being viewed — step back to when it broke, press `S`. History
lives in memory only while the session runs, bounded to a few MiB of
distinct frames.

Each run's output is bounded too: rat keeps the newest 1000 lines of a
watched command and says so on the status row when it drops the rest.
The same bound applies to every dashboard pane, where `overflow`
decides which end survives; `rat dashboard` below states the rule.

`t` flips both time rows from wall-clock stamps to counting ages,
without changing what they mean: the live row's `since 14:03:52`
becomes `changed 14s ago`, and the paused row's `at 14:03:52` becomes
`14s ago`. One style at a time, on every surface — press `t` again to
flip back.

Two view toggles work live or frozen, without pausing anything: `w`
switches long lines between wrapped and chopped, and `h`/`l` (or
Left/Right) scroll the view horizontally in 8-column steps. As in `less`,
a horizontally shifted view shows chopped lines until you shift back to
the left edge. Start chopped with `--no-wrap`.

Two change markers show what moved against the previous distinct
frame, and both work live, scrolled, frozen, or stepped back in time —
a scrubbed frame shows what changed into it. `D` toggles a margin
column marking the changed lines; `c` highlights the changed
characters in place, in reverse video layered over the output's own
colors — except glyphs whose ink coverage is the value (bar blocks,
sparkline ticks, braille), which recolor their ink instead, so a bar
never reads emptier at the moment it advances. Run either or both. The marks stay put until the output
changes again, so on a slow dashboard they answer "what moved last",
and they mark content only: a line that merely changed color stays
unmarked, and so does pure whitespace reshuffling, like a table
re-aligning its columns. A removed line leaves nothing to highlight,
so deletions show in the gutter only. The gutter is its own column —
it never scrolls away when you shift with `h`/`l`, and while it's on,
long lines render chopped, the same rule as a shifted view. The
character highlights follow the text wherever it goes: wrapped,
chopped, or shifted.

`S` writes the frame being viewed to `rat-watch-YYYYMMDD-HHMMSS.txt` in
`--snapshot-dir` (or `RAT_SNAPSHOT_DIR`, or the directory the watch was
launched from) and shows the path in the notice row. Snapshots are plain
text — ready for `grep` — unless `--snapshot-ansi` keeps the colors, and
a second snapshot in the same second gets a numbered name instead of
overwriting. The snapshot is the data, not the viewport: it always
contains the full untruncated frame, however the view is scrolled,
wrapped, or shifted.

### `rat dashboard` — N panes, N cadences, one frame

One command, one file, N panes composed into one flicker-free frame —
each pane running its own command on its own interval, with its own
triggers:

```sh
rat dashboard panes.kdl
rat dashboard panes.kdl --once   # render one frame and exit
```

The declaration file names each pane's command and cadence, shared
defaults, and where each pane sits, in KDL:

```kdl
gap 1

defaults interval="5s" border="rounded" padding="0 1" height=7

row {
    pane "log" {
        command "git" "log" "--oneline" "-3"
        interval "15s"
    }
    pane "branch" {
        command "git" "status" "--short" "--branch"
    }
}

pane "clock" {
    command "date" "+%H:%M:%S"
    interval "1s"
    height 4
}
```

A pane is declared inside the row or column that places it, so its id
is written once. `defaults` supplies anything a pane omits. A pane's
id is its identity — its default title, the value of `RAT_PANE` in the
child's environment (so one script can serve every pane by dispatching
on it), and the anchor a `ref="#id"` points at. An id sticks to
letters, digits, and `-` `.` `_` `~` — every id is a valid URI
fragment — and display text belongs in `title`. `command` is a string split like a shell word list,
multiple arguments taken verbatim as argv, or a raw script string with
`shell #true`. A string names a specific shell instead —
`shell="fish"` — with the same dialect table `--shell=NAME` uses. For
a script with backslashes in it — a `sed` program,
say — reach for KDL's raw strings (`command #"sed 's/\t/ · /'"#`) so
the escaping is the shell's job alone.

A pane can also declare a whole `script` instead of a `command` — a
multi-line body in KDL's `"""` block form:

```kdl
pane "deps" {
    script """
        #!/usr/bin/env python3
        import json, pathlib
        lock = json.loads(pathlib.Path("deps.json").read_text())
        print(len(lock["packages"]), "packages")
        """
}
```

A body whose first two bytes are `#!` runs through its own
interpreter: rat writes it once per run to a private file and
re-executes that file every tick, so python, node, ruby — anything —
works without `-c`/`-e` gymnastics. On unix the kernel does the
shebang honors; on Windows rat reads the first line itself, resolves
`/usr/bin/env X` by name on `PATH`, and gives the file the extension
the interpreter insists on (`.ps1` for PowerShell; `.cmd` for cmd,
whose `#!` line is stripped — `#` is not batch syntax). A body with
no `#!` runs through the pane's shell instead — the `shell` key, or
the platform's — exactly the unix no-shebang fallback.

One alignment rule matters: KDL removes the CLOSING `"""`'s
indentation from every line, so align the closing quotes with the
script to land the `#!` on the body's first byte — an indented `#!`
is refused with an error naming this rule. For regex-heavy bodies the
raw block form (`#"""` … `"""#`) keeps backslashes untouched. See
[examples/script.kdl](examples/script.kdl).

Every key a `pane` or `defaults` block accepts holds exactly one value,
so it may be written either as a property or as a child node —
`interval="5s"` and `interval "5s"` mean the same thing, and the
`defaults` line above is the property form of the same four keys.
`command` and `trigger` hold lists, and a KDL property holds exactly
one value, so those two are written as child nodes only.

Panes at the top level stack; a `row` puts them side by side. Rows and
columns nest to any depth, so grids need no second mechanism:

```kdl
row {
    column {
        pane "log" { … }
        pane "branch" { … }
    }
    pane "clock" { … }
}
```

`gap` is the columns between panes in a row, `row-gap` the blank rows
between rows, and `title` is the dashboard's own name — one bold line
above the composed panes, the same treatment `rat watch --title`
gives a plain frame. All three belong to the whole dashboard and are
written once at the top level. (`title` also exists as a pane key,
where it labels that one box's border — the file position keeps the
two meanings apart.) A declared title costs one row of the frame's
height budget, exactly like a pane row.

`title` may also point at a pane: `title "Deploy status" ref="#header"`
makes the pane with id `header` the dashboard's title. The pane stays
exactly where the file placed it and IS the visible title — styled
however its command likes, updating on its own cadence — and no extra
line is rendered; the positional text is the title's fallback until
the pane first speaks. A reference is a URI fragment, `#` plus a pane
id, and nothing else is accepted there yet, on purpose. Duplicate ids
load first-win — a `ref` binds the first declaration — and are listed
under `?`'s diagnostics section.

An interactive dashboard also carries its title to the terminal tab:
`▞` plus the declared text, the referenced pane's latest first line,
or the file's stem — restored on exit where the terminal supports the
title stack. Piped and `--once` runs never touch the tab.

A pane may even run `rat dashboard … --once` as its child: the inner
one-shot sizes itself to the pane through the handed-down `RAT_WIDTH`/
`RAT_HEIGHT` and renders as a dashboard-in-a-dashboard, re-run at the
outer pane's cadence.

Per pane, `interval` takes a duration or `"never"` for a pane only a
trigger moves; `trigger` takes the same `file:` / `fifo:` / `fd:`
sources `rat watch --trigger` does, with `trigger-debounce` as its
window; a pane with neither runs every 2s. `height` pins the finished
box, borders and chrome included — the pin is what keeps the frame's
row count constant and repaints cheap. Longer output is cut by
`overflow`: `keep-top` (the default) or `keep-bottom` for a log tail.
`width` takes cells (`"40"`), a weight (`"2fr"`), or `"auto"`.

Every pane's last inner row is a faint `{cadence} · {stamp}` line the
loop owns. The stamp is when that pane's output last *changed*, not
when it last ran, so a calm dashboard stays calm; `t` flips every
time-bearing row — footer and panes together — to counting ages. A
pane that fails fails inside its own box: a spawn error renders as its
text, a nonzero exit shows the command's output and stderr with
` · exit N` on the chrome row, and the rest of the dashboard is
untouched.

Freeze, scrub, snapshot, the pager, and the view toggles all act on
the whole composed frame, exactly as in `rat watch`, and `?` pages the
key reference with each pane's cadence listed. Scroll is the one key
group with two targets: whole-frame by default, and a focused pane's
own window once a pane holds the focus — see Pane navigation below,
which also adds three pane-scoped gestures (focus itself, zoom, and
collapse). `--once` runs every pane
once in parallel, prints one frame, and exits; piped output degrades to
plain text with each pane's stderr folded into its own box. If a pane
follows instead of exiting, `--once` says so on stderr after five quiet
seconds — naming the pane and the `live=#true` declaration to write —
and `--once-timeout 30s` bounds the wait: on expiry the run exits 124
with an empty stdout rather than printing a partial frame.

#### Pane navigation

`Tab` and `BackTab` cycle the focus through the panes in layout
reading order, wrapping; `Alt-h/j/k/l` moves it directionally, and a
direction with no candidate pane is a no-op rather than a wrap;
`Alt-1`–`Alt-9` jump straight to a pane by its reading-order number.
While any pane is focused, every title counts itself in declaration
order (`1 · alpha`), so the jump targets are visible exactly while
you are navigating — at rest the board stays unnumbered. On a board
with more than nine panes the count keeps going even though the jump
keys stop at `Alt-9`: the number still names the pane's place, and
`Tab` reaches everything. The
focused pane wears the accent border and the footer names it. On a
board taller than the window the frame viewport follows the focus:
focusing a pane below the fold scrolls it into view, and the gestures
work from a scrolled frame too. `Esc` peels one layer at a time — the
zoom, then the focus (the frame scroll holds its place), then the
frame scroll itself. Only a paused or scrubbed frame ignores the pane
keys; freeze/scrub stay whole-frame, unchanged.

With a pane focused, the scroll keys (`j/k`, `d/u`, `f/b`, `g/G`) and
the wheel drive that pane's own window over its retained lines instead
of the whole frame; the chrome row gains a `lines a-b of N` badge while
the window is off its declared rest, and `v` pages the focused pane's
whole retained body. `Enter` zooms the focused pane first; a second
`Enter`, zoomed, hands that body to the pager. The window holds its
place when the pane's next run replaces the body — clamped into the
new shape, never reset. The horizontal shift (`h/l`) is a plain-watch
affair: pane content is clipped to its box, so on a board those keys
are inert.

`z` zooms the focused pane to the full frame and back. A live pane
just re-clips to the new width — a view gesture never restarts a
long-lived child, the same rule the gutter toggle and a resize already
follow. A batch pane's content was rendered at its old declared width,
so it re-runs once, debounced, to arrive at the zoomed width honestly —
on zoom-in and zoom-out alike. While zoomed, `Tab`/`BackTab` — and an
`Alt-digit` jump — carry the zoom from pane to pane along the reading
order, and the chrome row's `zoomed 2/4` badge names the pane's place
in that cycle. The hidden panes
keep running underneath, and per-pane scroll stays active over the
zoomed body.

`Space` collapses the focused pane to a one-row title line; the child
keeps running and being captured underneath — only what is rendered
changes, so expanding returns the retained body without a re-run. A
collapsed pane in a column shortens the composed frame by its height;
one in a row frees nothing, since its row keeps its tallest pane's
height. `?` carries the full key table.

**Panes are for watching, not for doing.** A pane's command runs again
and again, and its declared interval is a floor rather than the whole
story — a pane also re-runs on every trigger that fires, on a debounced
respawn after the terminal is resized, and when the terminal switches
between light and dark, because a child already in flight was told the
old appearance. So "every 60s" is not 60 runs an hour; it is at least
that, on a schedule the dashboard controls and the command cannot see.

Write pane commands that can run at any moment and any number of times
without it mattering — read a file, query a status, format some text.
A command with side effects will have them again on events that have
nothing to do with its cadence. If a pane must touch something that
changes, have the command skip the write when nothing changed, and put
the part that cannot be skipped inside a script the dashboard only
*reads* the result of. Writing the same bytes again is not enough: a
`file:` trigger fires on modification time, not content, so a command
that rewrites a file identically — `cp`, `sed -i`, a formatter — fires
it every time. The same applies to a nested `rat dashboard … --once`:
the inner panes all run once per outer tick, and an inner `interval`
has nothing to schedule.

**A side effect on a watched path is a loop.** If a pane's command
touches a file that any pane triggers on — another pane's or its own —
then those panes drive each other for as long as the dashboard runs, at
a rate you did not choose. The frame itself will not say so: a pane's
stamp moves only when its output *changes*, so a loop whose output is
constant never repaints, and a dashboard can sit there spawning a shell
several times a second looking perfectly still. `interval "never"` is
not a brake — it removes the clock, and the trigger is what runs the
command. Point a trigger at a path no pane writes.

**rat says so when it notices.** A pane it suspects of looping carries
`· looping` on its chrome row, where a failing pane shows `· exit N`,
and the first time a loop is noticed one row names the panes involved
and the paths they watch, so you can check the claim against what you
declared. **Nothing is stopped.** Those panes keep running at whatever
rate they had — the report is a report. Press `?` for what the badge
means and both ways to fix it.

It can be wrong, and it can say nothing at all. rat cannot see who
writes a file. What it sees is that a watched path changes while the
dashboard is busy and never while it is idle, which is what a loop
looks like — and also what a pane fed only by other panes looks like.
A dashboard whose panes are busy most of the time has too little idle
time for that test to mean anything, so rat declines to answer rather
than guess, and a loop of slow commands is the kind it misses. Treat
the paragraph above as the fix and the badge as a warning you might
not get.

Cost, rather than correctness, has a lever: a pane declared
`interval "never"` with a `trigger` runs only when its trigger says
something changed, so an expensive command can sit behind a cheap file
whose modification time is the signal — written by something outside the
dashboard.

**Authoring for panes:** a pane's child prints *content only* — boxes,
titles, heights, and the side-by-side layout are the loop's job, so a
child that draws its own border just gets another drawn around it.
Each child is told its pane's inner size through `RAT_WIDTH` and
`RAT_HEIGHT` (and its id through `RAT_PANE`) and should format to
that width; height-stable output keeps repaints cheapest, which is
equally true for plain `rat watch` scripts — a placeholder row beats a
row that comes and goes.

**A command that never stops printing is bounded.** rat keeps at most
1000 lines of each run's output, per stream — a count of lines, never a
size, because a thousand short lines and a thousand long ones cost
about the same to hold and a byte budget would bound neither. Which end
survives is the pane's `overflow`: the head by default, the tail where
you declared `keep-bottom`. `rat watch` has no pane to declare it on and
keeps the newest, so a watch whose command floods now shows its tail
instead of everything it has ever printed.

Being a line count, it is not directly a memory bound: a single line is
kept up to 64 KiB before it is dropped whole, so the ceiling is 62.5 MiB
per stream rather than the ~100 KiB ordinary output costs. Terminal
lines run well under 100 bytes, so you would need output that is
uniformly enormous to approach it — but that is the number, and it
scales with the line count.

Past the bound, the pane says so on the chrome row where a failing pane
shows `· exit N`: `· 1.2k lines dropped`. A plain `rat watch` puts it on
the status row, and a piped run puts it on stderr, so the data you are
parsing stays the data. Press `?` for what it means. **Nothing is
stopped or slowed to make this happen** — rat reads the output to the
end and stops *keeping*, so a command never blocks writing into a pipe
nobody is draining.

**A pane can follow instead of poll.** `live #true` spawns the pane's
command once and paints its output as it arrives — the shape of
`tail -f`, `kubectl logs -f`, `docker logs -f` — instead of running it
on a cadence. The chrome row says `live` in place of an interval,
because there is none to report; the stamp still moves only when the
output changes. A live pane keeps the *tail* of its stream by default,
and `keep-top` is refused at load — a follower's newest line is the
point of it. The bound above applies unchanged, and a follower is the
shape most likely to reach it: a stream with no end meets a retained
set with one, and the pane wears the marker when it does.

`interval` still means something on a live pane — just not a cadence.
It is how soon a replacement spawns if the child ever exits: nothing
while the child runs, the delay before a fresh one when it dies. It is
deliberately not refused, because `defaults interval="5s"` would
otherwise be a load error for every live pane that inherits it, and a
mixed dashboard is the normal shape. `interval "never"` means no
replacement — the pane keeps its exit badge. A `trigger` on a live
pane *restarts* the child: the running one is asked to exit
(`SIGTERM`), forced two seconds later if it will not, and the
replacement spawned once it is reaped, debounced like every other
fire. A child that handles the signal gets to flush a final line, and
that line reaches the pane. (On Windows the restart is a hard kill;
there is no polite signal to send.) Resizes and theme flips leave a
live child alone — a follower
mid-stream is not restarted for cosmetics — so give it a `trigger` if
you want a handle to restart it by. `?` says all of this where the
chrome row has no room to.

**What `live` cannot fix: a pipeline that buffers.** The last stage of
a pipeline block-buffers its stdout when it is not a terminal, and
under rat it is not one. Measured plainly: `tail -f log` alone
delivered each line at its emission instant, while
`tail -f log | grep ERROR` delivered 0 bytes in 4 seconds — the same
log, the same appends. That is the stage's stdio buffering, not the
follower, and nothing rat does can change it. Give the stage a
line-buffered mode and the pipeline follows fine: `grep
--line-buffered`, `awk '{print; fflush()}'`, or `stdbuf -oL` in front
of a tool with no such flag.

A runnable declaration lives at
[`examples/panes.kdl`](examples/panes.kdl);
[`examples/panes-nested.kdl`](examples/panes-nested.kdl) shows nested
rows and columns and a dashboard-in-a-dashboard pane together, and
[`examples/follow.kdl`](examples/follow.kdl) is a live log follower
beside a batch pane. [`examples/tail.kdl`](examples/tail.kdl) is that
follower made self-feeding — the batch pane writes the log the live
pane tails, so it needs no second terminal — and
[`examples/tail-windows.kdl`](examples/tail-windows.kdl) is the same
dashboard for `cmd.exe`, where a `shell #true` script may contain
neither a double quote nor a pipe.

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
| `cursor` | reserved — the `rat input` caret is the terminal's own cursor, so no cell reads this token today |
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
resolved at startup. A change that lands while the frame is frozen is
adopted right away — a fresh run re-renders the output in the new
palette — but the frozen picture keeps its colors until you resume.

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
`2` usage error, `124` timeout (`--timeout 30s`, dashboard
`--once-timeout 30s`), `130` ctrl-c, and `rat spin` forwards the
child's code.

`rat spin` holds its child's output so it can replay it after the
spinner stops, and that is bounded too: **the newest 10,000 lines of
each stream**. Ten times what a watch pane keeps, because spin prints
what it kept rather than rendering a window over it — enough for a full
build log, and still a ceiling for a command that never stops. Ordinary
output costs about 1 MiB; the same 64 KiB-per-line rule puts the ceiling
at 625 MiB per stream for output that is uniformly enormous.

When it drops something it says so on stderr, never in the output you
are piping: `rat: 2.0k lines dropped from stdout — kept the newest
10000`. A stream you did not ask to see stays silent — `spin` drains
both pipes whatever the flags say, so the default invocation discards
plenty, and reporting output you never wanted would be debug noise on
every long-running command.

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
live in [`examples/`](examples/) for bash, zsh, fish, and PowerShell. The
shell scripts are single-command watch dashboards: one script renders the
whole frame on one cadence. The declaration files beside them
(`panes.kdl`, `panes-nested.kdl`) are the other shape — N commands on N
cadences, composed by `rat dashboard`.

## Differences from gum

`rat` is not gum-complete, on purpose. It is gum's scripting primitives plus
the dashboard toolkit above.

- **Not ported:** `format`, `write`, `file`, `pager` — none of them earn
  their keep in a dashboard script.
- **Added:** `bar`, `spark`, `watch`, `dashboard`, `frame`, `doctor`,
  `duration`, `date`, `table`.
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
where unix uses `/dev/tty`; `watch --shell` runs through `%COMSPEC% /C`,
and `--shell=NAME` picks another (`--shell=pwsh` runs
`-NoProfile -Command`);
`rat` enables VT processing on the console itself, so escapes are
processed even in legacy conhost, which simply ignores the synchronized-
output mode it doesn't implement (Windows Terminal supports it). A
child that writes in the console's legacy codepage (OEM 437, 850, …)
renders correctly: output that is not valid UTF-8 is decoded with the
active console codepage, per line, and UTF-8 output passes through
untouched — while a child emitting legacy bytes under a `chcp 65001`
console still shows exactly what the console itself would show. Three
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
| `--timeout` / `--once-timeout` expired | 124 |
| Ctrl-C | 130 |
