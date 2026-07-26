#!/usr/bin/env fish
# The interactive primitives, chained in fish. Needs a terminal.

set name (rat input --placeholder 'Your name' --header 'Who are you?')
rat log --level info "hello, $name"

set fruit (rat choose --header 'Pick a fruit:' apple banana cherry)
set toppings (rat choose --no-limit --header 'Toppings (space to select):' nuts sprinkles honey)

set pick (printf 'alpha\nbeta\ngamma\ndelta\n' | rat filter --header 'Fuzzy find:')

if rat confirm "Order $fruit with ["(string join ', ' $toppings)"] for $name?"
    rat spin --title 'Placing order...' -- sleep 2
    rat style --foreground 42 "Ordered. ($pick was a fine choice too.)"
else
    rat log --level warn cancelled
end
