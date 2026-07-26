#!/usr/bin/env pwsh
# The interactive primitives, chained in PowerShell. Needs a terminal.

$name = rat input --placeholder 'Your name' --header 'Who are you?'
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
rat log --level info "hello, $name"

$fruit = rat choose --header 'Pick a fruit:' apple banana cherry
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# Multi-select stdout arrives as an array of lines in PowerShell.
$toppings = rat choose --no-limit --header 'Toppings (space to select):' nuts sprinkles honey
$toppingText = if ($toppings) { $toppings -join ', ' } else { 'no toppings' }

$pick = 'alpha', 'beta', 'gamma', 'delta' | rat filter --header 'Fuzzy find:'

# ${name} braces matter: a bare $name? would interpolate the
# variable "name?" (question marks are valid in PS variable names).
rat confirm "Order $fruit with [$toppingText] for ${name}?"
if ($LASTEXITCODE -eq 0) {
    $me = (Get-Process -Id $PID).Path
    rat spin --title 'Placing order...' -- $me -NoProfile -Command 'Start-Sleep 2'
    rat style --foreground 42 "Ordered. ($pick was a fine choice too.)"
} else {
    rat log --level warn 'cancelled'
}
