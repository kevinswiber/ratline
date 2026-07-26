#!/usr/bin/env pwsh
# A live system dashboard in PowerShell (works in pwsh 7 on any OS).
# Run it, watch it repaint flicker-free, ctrl-c to leave.
# `./dashboard.ps1 -Once` prints one frame.
param([switch]$Render, [switch]$Once)

function Show-Frame {
    rat style --bold --foreground 212 'System dashboard'
    rat style --faint (Get-Date).ToString()
    Write-Output ''

    # Disk usage: the first filesystem drive becomes a threshold-colored bar.
    $drive = Get-PSDrive -PSProvider FileSystem |
        Where-Object { $_.Used -gt 0 } | Select-Object -First 1
    if ($drive) {
        $usedGiB = [math]::Round($drive.Used / 1GB)
        $totalGiB = [math]::Round(($drive.Used + $drive.Free) / 1GB)
        "disk $($drive.Name)`t$usedGiB`t$totalGiB" |
            rat bar --width 24 --thresholds '70:42,90:214,100:196' --annotation percent
    }

    # A simulated deploy that animates with the clock.
    $step = [DateTimeOffset]::Now.ToUnixTimeSeconds() % 120
    $left = rat duration (120 - $step)
    rat bar --label 'deploy (simulated)' --value $step --total 120 `
        --width 24 --state "$left left"

    # Busiest processes by CPU time, as a sparkline.
    $cpu = Get-Process | Where-Object CPU | Sort-Object CPU -Descending |
        Select-Object -First 8 | ForEach-Object { [int]$_.CPU }
    if ($cpu) {
        $spark = $cpu | rat spark --min 0 --spark-color 212
        Write-Output "top cpu  $spark"
    }
    Write-Output ''

    rat log --level info "frame rendered $(rat date --relative ([DateTimeOffset]::Now.ToUnixTimeSeconds()))"
}

if ($Render -or $Once) {
    Show-Frame
} else {
    # Re-invoke this script with the same PowerShell for every tick.
    $me = (Get-Process -Id $PID).Path
    & rat watch --clear --interval 2s -- $me -NoProfile -File $PSCommandPath -Render
}
