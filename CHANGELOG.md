# Changelog
All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

- - -
## [v0.5.0](https://github.com/kevinswiber/ratto/compare/4c564934a9c180d246d0390b993fa03661e00f89..v0.5.0) - 2026-07-27
#### Features
- follow terminal theme changes live in watch on unix - ([3556c9f](https://github.com/kevinswiber/ratto/commit/3556c9f05e30d438a0dd09a44fc0e537ae89e746)) - [@kevinswiber](https://github.com/kevinswiber)
- subscribe watch to terminal theme notifications on unix - ([1343e05](https://github.com/kevinswiber/ratto/commit/1343e05a8e253a2b254b83ed59e5cd38d7f931be)) - [@kevinswiber](https://github.com/kevinswiber)
- read the terminal directly in watch on unix - ([96016d6](https://github.com/kevinswiber/ratto/commit/96016d69c6b8a39aa3a677e5a6ea6a78caa97fac)) - [@kevinswiber](https://github.com/kevinswiber)
- re-resolve the watch palette in place from a reported appearance - ([d4a43cd](https://github.com/kevinswiber/ratto/commit/d4a43cd0192166ae216102b9df890881475517ae)) - [@kevinswiber](https://github.com/kevinswiber)
- write the DEC 2031 subscription and verify-query guard - ([3a03b26](https://github.com/kevinswiber/ratto/commit/3a03b265fc8177e7658ed4eccc75092572d3dfbb)) - [@kevinswiber](https://github.com/kevinswiber)
- scan raw terminal input into keys, reports, and color replies - ([26e900b](https://github.com/kevinswiber/ratto/commit/26e900be6f4508a4c72c03f7f50d932a4e6f29cd)) - [@kevinswiber](https://github.com/kevinswiber)
- gate theme-notification subscriptions on ownership, profile, and provenance - ([e036f90](https://github.com/kevinswiber/ratto/commit/e036f9054f5f522da71488cd220058560372542a)) - [@kevinswiber](https://github.com/kevinswiber)
- parse OSC color replies and classify light against dark - ([e59f17d](https://github.com/kevinswiber/ratto/commit/e59f17db9200b49c8b0a55fd36ee8b0cb5138492)) - [@kevinswiber](https://github.com/kevinswiber)
- parse the DSR 997 color-scheme report - ([d8edd33](https://github.com/kevinswiber/ratto/commit/d8edd3383c97afb2325504033b12a958965ee087)) - [@kevinswiber](https://github.com/kevinswiber)
- name terminal-pushed reports as an appearance source - ([619ad25](https://github.com/kevinswiber/ratto/commit/619ad25ba196b5ef5502b62569e4fd5b9c3f7ab2)) - [@kevinswiber](https://github.com/kevinswiber)
- default bare Windows consoles to truecolor - ([eb1f080](https://github.com/kevinswiber/ratto/commit/eb1f0803fe5c403be21e007eeacf852484527491)) - [@kevinswiber](https://github.com/kevinswiber)
- verified light palette values from a live light-terminal pass - ([d0f805c](https://github.com/kevinswiber/ratto/commit/d0f805c77e190b0afb1806ef98e311688feca5ff)) - [@kevinswiber](https://github.com/kevinswiber)
- report the appearance and its source in doctor - ([47f0785](https://github.com/kevinswiber/ratto/commit/47f0785f7d5f98f462f988e74ebbddab52272383)) - [@kevinswiber](https://github.com/kevinswiber)
- export the resolved appearance to watch children - ([a532810](https://github.com/kevinswiber/ratto/commit/a5328107256120553c4c3378a50a072c9f1b5a31)) - [@kevinswiber](https://github.com/kevinswiber)
- take interactive accents from the palette - ([920bb54](https://github.com/kevinswiber/ratto/commit/920bb548d9f751fe8e1423c6618491a6bda831de)) - [@kevinswiber](https://github.com/kevinswiber)
- read log level colors from the palette - ([fba3e65](https://github.com/kevinswiber/ratto/commit/fba3e65d6023430e4fd36e2ac8dbf1376cd68674)) - [@kevinswiber](https://github.com/kevinswiber)
- accept theme tokens wherever a color string is accepted - ([d54836e](https://github.com/kevinswiber/ratto/commit/d54836ee9e86a36d73face4c7a5c6c03015640da)) - [@kevinswiber](https://github.com/kevinswiber)
- resolve terminal appearance once and thread a palette to every command - ([8f631eb](https://github.com/kevinswiber/ratto/commit/8f631eb523b77863d0ff78207a7eed6b823cd9b1)) - [@kevinswiber](https://github.com/kevinswiber)
- probe the terminal background over OSC behind a strict gate - ([cafa6bc](https://github.com/kevinswiber/ratto/commit/cafa6bc30df038dfb37e1010b54014c63ec6479f)) - [@kevinswiber](https://github.com/kevinswiber)
- add appearance policy and a COLORFGBG reader - ([9cddc85](https://github.com/kevinswiber/ratto/commit/9cddc8523fc87af54c8b84b2c1da384b17267a38)) - [@kevinswiber](https://github.com/kevinswiber)
- add semantic color tokens with light and dark palettes - ([744782e](https://github.com/kevinswiber/ratto/commit/744782e12474aa38c7cdc43441951b4f9d7e2998)) - [@kevinswiber](https://github.com/kevinswiber)
- export the frame size to watch children - ([8039fa7](https://github.com/kevinswiber/ratto/commit/8039fa7fb088b0738c18afdba4693f82d2b55595)) - [@kevinswiber](https://github.com/kevinswiber)
- stack joined blocks when the available width is exceeded - ([448a547](https://github.com/kevinswiber/ratto/commit/448a5473cb48a35a7df8bd8e696e1693489be86b)) - [@kevinswiber](https://github.com/kevinswiber)
- place text blocks side by side with rat join - ([47dfd63](https://github.com/kevinswiber/ratto/commit/47dfd63f30206bed213d49ea709c534e26f2c368)) - [@kevinswiber](https://github.com/kevinswiber)
- join blocks horizontally and vertically - ([300a469](https://github.com/kevinswiber/ratto/commit/300a469408bb259d01b3a42f6896b15b620765f4)) - [@kevinswiber](https://github.com/kevinswiber)
- give style a box model with borders, padding, and titles - ([8970b54](https://github.com/kevinswiber/ratto/commit/8970b54050e3a571cba2a2fefc37c60ea0b07914)) - [@kevinswiber](https://github.com/kevinswiber)
- splice a title into the top border - ([84f0dd4](https://github.com/kevinswiber/ratto/commit/84f0dd46332369e9febf9e3aeb8f320b75374cc9)) - [@kevinswiber](https://github.com/kevinswiber)
- render bordered padded boxes around content lines - ([819a8de](https://github.com/kevinswiber/ratto/commit/819a8de62ce4f9ca023f98db936a9f448c442ba0)) - [@kevinswiber](https://github.com/kevinswiber)
- add border presets and css side shorthand - ([64bfb84](https://github.com/kevinswiber/ratto/commit/64bfb849195d861726db44ab9ff32c2ef97fde4d)) - [@kevinswiber](https://github.com/kevinswiber)
- align delimiter-separated rows with rat table - ([87135e2](https://github.com/kevinswiber/ratto/commit/87135e2d3f72ca98ecd5ac4c53398acbbfc79fcc)) - [@kevinswiber](https://github.com/kevinswiber)
- wrap pinned table cells onto continuation lines - ([cc5dd9e](https://github.com/kevinswiber/ratto/commit/cc5dd9efc3468bc7586ce9124d1f734074ac3090)) - [@kevinswiber](https://github.com/kevinswiber)
- render aligned table rows with truncation - ([84c937d](https://github.com/kevinswiber/ratto/commit/84c937d54a481567e572f1ca3dbfc50cf8d823f1)) - [@kevinswiber](https://github.com/kevinswiber)
- add the table row model and column resolution - ([394fedb](https://github.com/kevinswiber/ratto/commit/394fedbb6c8b530df00b544536011ce983c0bc9d)) - [@kevinswiber](https://github.com/kevinswiber)
- track sgr state and wrap styled text by display width - ([00b356b](https://github.com/kevinswiber/ratto/commit/00b356b9851d3e3a8dfed30f3d7f52db71beb705)) - [@kevinswiber](https://github.com/kevinswiber)
- add an ansi-aware display width and truncation core - ([8c6dd05](https://github.com/kevinswiber/ratto/commit/8c6dd0550e8b5f481ba227f549b042819bf6811c)) - [@kevinswiber](https://github.com/kevinswiber)
#### Bug Fixes
- repaint over the frame the pager's alternate screen restores - ([22a2649](https://github.com/kevinswiber/ratto/commit/22a2649947a0301828cffea8d1f931985251bb19)) - [@kevinswiber](https://github.com/kevinswiber)
- keep the watch test module last in the file - ([3bfbb62](https://github.com/kevinswiber/ratto/commit/3bfbb62e5ab9ae2aaacaba7aa3318faf5b18aa70)) - [@kevinswiber](https://github.com/kevinswiber)
- keep the theme input path warning-clean on linux and windows - ([5c1afec](https://github.com/kevinswiber/ratto/commit/5c1afec88846a28dc22e0a470d17ef26f0209ea2)) - [@kevinswiber](https://github.com/kevinswiber)
- enable virtual terminal processing on the console - ([b0c3609](https://github.com/kevinswiber/ratto/commit/b0c360926c4291d00576f13ee0939e8974eb3b7b)) - [@kevinswiber](https://github.com/kevinswiber)
- strip escapes without eating tabs in the layout filters - ([08b712c](https://github.com/kevinswiber/ratto/commit/08b712c132fa809bd0bb78dc95694187f3ef42ca)) - [@kevinswiber](https://github.com/kevinswiber)
- honor an explicit label width in bar batch mode - ([4c56493](https://github.com/kevinswiber/ratto/commit/4c564934a9c180d246d0390b993fa03661e00f89)) - [@kevinswiber](https://github.com/kevinswiber)
#### Documentation
- describe live light/dark switching in watch - ([02674a1](https://github.com/kevinswiber/ratto/commit/02674a15cc728e0effa73cd4f9bec4abef1c8a35)) - [@kevinswiber](https://github.com/kevinswiber)
- describe native Windows color and console VT handling - ([379fc7a](https://github.com/kevinswiber/ratto/commit/379fc7aed6f846f41ec17bef368a84f9908d9bb1)) - [@kevinswiber](https://github.com/kevinswiber)
- document appearance selection and the color tokens - ([ea3fe61](https://github.com/kevinswiber/ratto/commit/ea3fe61787c1d8d495d87c476481203dc0e9ef39)) - [@kevinswiber](https://github.com/kevinswiber)
- describe fit joins and the frame size env - ([d831f50](https://github.com/kevinswiber/ratto/commit/d831f50f3dc7a756790925171b9cfb4ea831f5e6)) - [@kevinswiber](https://github.com/kevinswiber)
- note the no-strip-ansi flag for boxing styled content - ([d96111e](https://github.com/kevinswiber/ratto/commit/d96111ecd37b05102f1980c0c7865147d4d615ef)) - [@kevinswiber](https://github.com/kevinswiber)
- document table, join, and the style box model - ([5e292ad](https://github.com/kevinswiber/ratto/commit/5e292ad9c342a28ae41bc82a7a562fc6ff8bf2cd)) - [@kevinswiber](https://github.com/kevinswiber)

- - -

## [v0.4.0](https://github.com/kevinswiber/ratto/compare/458eb4acddba67c7878264fbf739d7d2012a08db..v0.4.0) - 2026-07-26
#### Features
- fall back to the stock windows pager when less is missing - ([53e22bf](https://github.com/kevinswiber/ratto/commit/53e22bfb162fe5f7e1b1a8b3f3111cc700286568)) - [@kevinswiber](https://github.com/kevinswiber)
#### Bug Fixes
- keep the windows console in utf-8 while the pager runs - ([458eb4a](https://github.com/kevinswiber/ratto/commit/458eb4acddba67c7878264fbf739d7d2012a08db)) - [@kevinswiber](https://github.com/kevinswiber)

- - -

## [v0.3.2](https://github.com/kevinswiber/ratto/compare/dac4e59cf3b147b2f174db429ca1ea3ca021ff33..v0.3.2) - 2026-07-26
#### Bug Fixes
- brace the interpolated name in the powershell example - ([5da19aa](https://github.com/kevinswiber/ratto/commit/5da19aac53cef138a3cea1fcfa07a200158201ea)) - [@kevinswiber](https://github.com/kevinswiber)
- render utf-8 correctly on the windows console - ([3a7c188](https://github.com/kevinswiber/ratto/commit/3a7c188bfab81240dae38fb99c1a5a28180fb357)) - [@kevinswiber](https://github.com/kevinswiber)
- recognize both windows closed-pipe error codes - ([a0ef249](https://github.com/kevinswiber/ratto/commit/a0ef2498d033385f2b99ca17c320c7f825a9fd64)) - [@kevinswiber](https://github.com/kevinswiber)
- exit quietly on closed pipes on windows and test everywhere - ([4e541b8](https://github.com/kevinswiber/ratto/commit/4e541b8c14f24f684c8689c916b2f48dffc5838c)) - [@kevinswiber](https://github.com/kevinswiber)
#### Documentation
- add powershell examples - ([362e63e](https://github.com/kevinswiber/ratto/commit/362e63e950280ba079508a23777e5dd6eba5821f)) - [@kevinswiber](https://github.com/kevinswiber)
- point changelog links at the current repository - ([dac4e59](https://github.com/kevinswiber/ratto/commit/dac4e59cf3b147b2f174db429ca1ea3ca021ff33)) - [@kevinswiber](https://github.com/kevinswiber)

- - -

## [v0.3.1](https://github.com/kevinswiber/ratto/compare/0e698431324f0ee0b67ba50ea5755bd3e3881707..v0.3.1) - 2026-07-26
#### Documentation
- tidy the readme intro - ([0e69843](https://github.com/kevinswiber/ratto/commit/0e698431324f0ee0b67ba50ea5755bd3e3881707)) - [@kevinswiber](https://github.com/kevinswiber)

- - -

## [v0.3.0](https://github.com/kevinswiber/ratto/compare/98e41e20adf4390c1322548e6b65b951ba5982ec..v0.3.0) - 2026-07-26
#### Features
- page the full watch frame through the user pager - ([38a69ac](https://github.com/kevinswiber/ratto/commit/38a69aca08b9abb59f93219cfea9c7b2c7a1efb9)) - [@kevinswiber](https://github.com/kevinswiber)
- compile and behave correctly on windows - ([69db43e](https://github.com/kevinswiber/ratto/commit/69db43e6b0097b32d9f1043360c2ddfa98c9d6fb)) - [@kevinswiber](https://github.com/kevinswiber)
#### Bug Fixes
- let --color always outrank NO_COLOR at full depth - ([58ddab8](https://github.com/kevinswiber/ratto/commit/58ddab8cd87113f2ee1edc950be2158baeda2fe4)) - [@kevinswiber](https://github.com/kevinswiber)
#### Documentation
- spell out when --color auto goes plain - ([98e41e2](https://github.com/kevinswiber/ratto/commit/98e41e20adf4390c1322548e6b65b951ba5982ec)) - [@kevinswiber](https://github.com/kevinswiber)

- - -

## [v0.3.0](https://github.com/kevinswiber/ratto/compare/98e41e20adf4390c1322548e6b65b951ba5982ec..v0.3.0) - 2026-07-26
#### Features
- page the full watch frame through the user pager - ([38a69ac](https://github.com/kevinswiber/ratto/commit/38a69aca08b9abb59f93219cfea9c7b2c7a1efb9)) - [@kevinswiber](https://github.com/kevinswiber)
- compile and behave correctly on windows - ([69db43e](https://github.com/kevinswiber/ratto/commit/69db43e6b0097b32d9f1043360c2ddfa98c9d6fb)) - [@kevinswiber](https://github.com/kevinswiber)
#### Bug Fixes
- let --color always outrank NO_COLOR at full depth - ([58ddab8](https://github.com/kevinswiber/ratto/commit/58ddab8cd87113f2ee1edc950be2158baeda2fe4)) - [@kevinswiber](https://github.com/kevinswiber)
#### Documentation
- spell out when --color auto goes plain - ([98e41e2](https://github.com/kevinswiber/ratto/commit/98e41e20adf4390c1322548e6b65b951ba5982ec)) - [@kevinswiber](https://github.com/kevinswiber)

- - -

## [v0.2.1](https://github.com/kevinswiber/ratto/compare/2038679fbc70ed86523c76a3ea0c04291640bf48..v0.2.1) - 2026-07-26
#### Bug Fixes
- repaint from scratch after a terminal resize in watch - ([4f1c7e2](https://github.com/kevinswiber/ratto/commit/4f1c7e230fcc9df8df6b77ebd36e65cff2ade38c)) - [@kevinswiber](https://github.com/kevinswiber)
- keep the confirm prompt from vanishing in the fish example - ([a7354f2](https://github.com/kevinswiber/ratto/commit/a7354f27a4500a1baf16216441f5220e865d9d48)) - [@kevinswiber](https://github.com/kevinswiber)
- enable crossterm use-dev-tty so piped filter reads keys on macos - ([deda8d0](https://github.com/kevinswiber/ratto/commit/deda8d01737ec88f656aaf4ab032c23575825bdc)) - [@kevinswiber](https://github.com/kevinswiber)
- repair mangled apostrophes in spin help text - ([2038679](https://github.com/kevinswiber/ratto/commit/2038679fbc70ed86523c76a3ea0c04291640bf48)) - [@kevinswiber](https://github.com/kevinswiber)

- - -

## [v0.2.0](https://github.com/kevinswiber/ratto/compare/d9f8258f0d4cc1cbac3d0bfc0b0edfa35a6b176e..v0.2.0) - 2026-07-26
#### Features
- add watch --clear for full-screen dashboards - ([350243d](https://github.com/kevinswiber/ratto/commit/350243d2f5e44a9326cfee429606104dc3d2f83d)) - [@kevinswiber](https://github.com/kevinswiber)
#### Bug Fixes
- keep child stderr from corrupting the watch repaint - ([509f9b6](https://github.com/kevinswiber/ratto/commit/509f9b6aa01c85430265f109d691c08b4b2b4054)) - [@kevinswiber](https://github.com/kevinswiber)
#### Documentation
- switch readme examples to bash and add shell examples - ([c5278fb](https://github.com/kevinswiber/ratto/commit/c5278fb4d952e6d511071d7572b31a993cce7ea0)) - [@kevinswiber](https://github.com/kevinswiber)
- mention watch --clear in the readme - ([b9f03f9](https://github.com/kevinswiber/ratto/commit/b9f03f90d81503b712bf84752a1c8724fab30ade)) - [@kevinswiber](https://github.com/kevinswiber)

- - -

Changelog generated by [cocogitto](https://github.com/cocogitto/cocogitto).