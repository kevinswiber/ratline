#!/usr/bin/env fish
# A live system dashboard in fish. Run it, watch it repaint flicker-free,
# ctrl-c to leave. `./dashboard.fish --once` prints one frame.

function render
    rat style --bold --foreground 212 'System dashboard'
    rat style --faint (date)
    echo

    # Disk usage: df rows become one aligned block of threshold-colored bars.
    df -Pk / 2>/dev/null | awk 'NR == 2 {
        printf "disk /\t%d\t%d\n", ($3 / 1024 / 1024), (($3 + $4) / 1024 / 1024)
    }' | rat bar --width 24 --thresholds '70:42,90:214,100:196' --annotation percent

    # A simulated deploy that animates with the clock.
    set -f step (math (date +%s) % 120)
    rat bar --label 'deploy (simulated)' --value "$step" --total 120 \
        --width 24 --state (rat duration (math 120 - $step))' left'

    # Load averages as a sparkline.
    printf 'load  '
    uptime | sed 's/.*load average[s]*: //' | tr -d ',' | rat spark --min 0
    echo

    rat log --level info "frame rendered "(rat date --relative (date +%s))
end

switch "$argv[1]"
    case --render --once
        render
    case '*'
        exec rat watch --clear --interval 2s -- fish (status filename) --render
end
