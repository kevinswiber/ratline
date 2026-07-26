#!/usr/bin/env bash
# The interactive primitives, chained in plain bash. Needs a terminal.
set -euo pipefail

name=$(rat input --placeholder 'Your name' --header 'Who are you?')
rat log --level info "hello, ${name:-stranger}"

fruit=$(rat choose --header 'Pick a fruit:' apple banana cherry)
toppings=$(rat choose --no-limit --header 'Toppings (space to select):' nuts sprinkles honey)

pick=$(printf 'alpha\nbeta\ngamma\ndelta\n' | rat filter --header 'Fuzzy find:')

topping_text=${toppings//$'\n'/, }
msg="Order ${fruit:-nothing} with [${topping_text:-no toppings}] for ${name:-you}?"
if rat confirm "$msg"; then
    rat spin --title 'Placing order...' -- sleep 2
    rat style --foreground 42 "Ordered. ($pick was a fine choice too.)"
else
    rat log --level warn 'cancelled'
fi
