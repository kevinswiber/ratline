#!/usr/bin/env fish
# The interactive primitives, chained in fish. Needs a terminal.

set name (rat input --placeholder 'Your name' --header 'Who are you?')
rat log --level info "hello, $name"

set fruit (rat choose --header 'Pick a fruit:' apple banana cherry)
set toppings (rat choose --no-limit --header 'Toppings (space to select):' nuts sprinkles honey)

set pick (printf 'alpha\nbeta\ngamma\ndelta\n' | rat filter --header 'Fuzzy find:')

# Build the message in a variable: concatenating a quoted string with a
# command substitution that prints nothing would cancel the whole argument.
set -l topping_text 'no toppings'
if set -q toppings[1]
    set topping_text (string join ', ' $toppings)
end
if rat confirm "Order $fruit with [$topping_text] for $name?"
    rat spin --title 'Placing order...' -- sleep 2
    rat style --foreground 42 "Ordered. ($pick was a fine choice too.)"
else
    rat log --level warn cancelled
end
