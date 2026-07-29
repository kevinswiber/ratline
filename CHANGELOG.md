# Changelog
All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

- - -
## [v0.7.0](https://github.com/kevinswiber/ratto/compare/e8b7bcd56d7f2d4070ea8a0f267eef015ccfb049..v0.7.0) - 2026-07-29
#### Features
- drive respawns from fifo and fd trigger readers with an end-of-life notice - ([d6938d8](https://github.com/kevinswiber/ratto/commit/d6938d87d2b72b87f47e448f8f36a17d5140c3f3)) - [@kevinswiber](https://github.com/kevinswiber)
- envelope the tap channel so a trigger can wake the event wait - ([b542942](https://github.com/kevinswiber/ratto/commit/b54294229ef609c6fcca8069fabc15dc41c40ecb)) - [@kevinswiber](https://github.com/kevinswiber)
- refresh watch on file trigger fires through the debounce gate - ([5f3f91a](https://github.com/kevinswiber/ratto/commit/5f3f91a0d609ffc1685b5a3791ca76fc7781c6a8)) - [@kevinswiber](https://github.com/kevinswiber)
- name the trigger mode in the footer and the sources in the key reference - ([acfa08e](https://github.com/kevinswiber/ratto/commit/acfa08e997a35fc7f596b6e337b8515ef82a9d01)) - [@kevinswiber](https://github.com/kevinswiber)
- declare the trigger surface on watch and make the interval optional - ([c964d7d](https://github.com/kevinswiber/ratto/commit/c964d7d6698cc009652fc5f3044c182f946def44)) - [@kevinswiber](https://github.com/kevinswiber)
- watch file trigger paths by mtime fingerprint - ([7334e91](https://github.com/kevinswiber/ratto/commit/7334e91fff9d48b8668406f8af3549bbbbca9a7f)) - [@kevinswiber](https://github.com/kevinswiber)
- gate trigger fires behind an anchored debounce window - ([06c93ac](https://github.com/kevinswiber/ratto/commit/06c93ac23f0acb96af101882fd33581d2478563e)) - [@kevinswiber](https://github.com/kevinswiber)
- parse trigger specs with scheme prefixes and teaching errors - ([d086a4e](https://github.com/kevinswiber/ratto/commit/d086a4e0e99a9134afaee6aa4bd30dd2c9d98b5a)) - [@kevinswiber](https://github.com/kevinswiber)
- give the tick schedule a deadline vocabulary with an optional interval - ([df8cd86](https://github.com/kevinswiber/ratto/commit/df8cd8678c713fce5619566025a9273abe5c0770)) - [@kevinswiber](https://github.com/kevinswiber)
- name the cadence in the live footer and slim the hints - ([d60af02](https://github.com/kevinswiber/ratto/commit/d60af029997581f5db2b32ace6dbecc50df80e2f)) - [@kevinswiber](https://github.com/kevinswiber)
- page a key reference from watch on ? - ([c60ee8f](https://github.com/kevinswiber/ratto/commit/c60ee8fef45e0902775cd97103d44ca6f36f8751)) - [@kevinswiber](https://github.com/kevinswiber)
- rerun the watch child when the terminal theme flips - ([bae33de](https://github.com/kevinswiber/ratto/commit/bae33de106df5062f9dc1cdc4a2a7e40c91ebe9d)) - [@kevinswiber](https://github.com/kevinswiber)
- run the watch child off the loop thread - ([5fbfd55](https://github.com/kevinswiber/ratto/commit/5fbfd55d92e156dcb1c1d0db8c03d528e4080a3a)) - [@kevinswiber](https://github.com/kevinswiber)
- repaint in place on pager return and resume - ([d57fb95](https://github.com/kevinswiber/ratto/commit/d57fb95d481b0fdb431550445fa026cdaca4a634)) - [@kevinswiber](https://github.com/kevinswiber)
- add a killable off-thread child runner for watch - ([cc625a2](https://github.com/kevinswiber/ratto/commit/cc625a238f201e74611c9ab19731a21cba3204f0)) - [@kevinswiber](https://github.com/kevinswiber)
- add a fixed-delay tick schedule with a single-flight guard - ([ae34704](https://github.com/kevinswiber/ratto/commit/ae34704d4d07acd0f082c31e3d71f9a24d201534)) - [@kevinswiber](https://github.com/kevinswiber)
- toggle status-row time display with t - ([3a83b58](https://github.com/kevinswiber/ratto/commit/3a83b5820a4fe78ff29a1398c3728f75ac092b4a)) - [@kevinswiber](https://github.com/kevinswiber)
- highlight changed characters in watch behind the c toggle - ([7be5a9f](https://github.com/kevinswiber/ratto/commit/7be5a9f8f2cc051d0e6a9f263b018749a2d34159)) - [@kevinswiber](https://github.com/kevinswiber)
- splice reverse-video marks onto changed characters - ([2664518](https://github.com/kevinswiber/ratto/commit/2664518729daf6f43dfb7b1626ba786a22e85d55)) - [@kevinswiber](https://github.com/kevinswiber)
- add a change gutter to watch behind the D toggle - ([20d3cf3](https://github.com/kevinswiber/ratto/commit/20d3cf37f596753cfdcc3b386bb5f8934968e4b5)) - [@kevinswiber](https://github.com/kevinswiber)
- stamp change-gutter margin cells onto window rows - ([07e240e](https://github.com/kevinswiber/ratto/commit/07e240e2fea05860aab888c315cea11874ab5b0f)) - [@kevinswiber](https://github.com/kevinswiber)
- compute per-line change marks with a whole-frame char diff - ([1ed6e0e](https://github.com/kevinswiber/ratto/commit/1ed6e0e061c04160e94dea3e28ed631af4bc8c5d)) - [@kevinswiber](https://github.com/kevinswiber)
- make freezing explicit — scroll keys never pause - ([67d122e](https://github.com/kevinswiber/ratto/commit/67d122e0a3d5f46db06319b9098893c2523c3ecb)) - [@kevinswiber](https://github.com/kevinswiber)
- scrub watch history with the transport keys - ([436258a](https://github.com/kevinswiber/ratto/commit/436258a73ca3285dbf4f338ccd95738c975a9763)) - [@kevinswiber](https://github.com/kevinswiber)
- add a byte-capped history ring of distinct frames - ([2de23cf](https://github.com/kevinswiber/ratto/commit/2de23cf66cfb69fdd46f2bc075903865ee168cfe)) - [@kevinswiber](https://github.com/kevinswiber)
- live-scroll stable frames and freeze on shape change - ([fcf7320](https://github.com/kevinswiber/ratto/commit/fcf7320fd99c0188134a3b4743ceff9ab7ee2c03)) - [@kevinswiber](https://github.com/kevinswiber)
- add stability tracking and live-scroll cores - ([f97c836](https://github.com/kevinswiber/ratto/commit/f97c836a7563757268a8b5a4f85f8a8e76dceaa2)) - [@kevinswiber](https://github.com/kevinswiber)
- add F as a resume alias and p as an explicit freeze - ([e190a26](https://github.com/kevinswiber/ratto/commit/e190a2639463b8748113b482fd5e5077082eb5ee)) - [@kevinswiber](https://github.com/kevinswiber)
- count the age of a paused watch frame - ([e0e4832](https://github.com/kevinswiber/ratto/commit/e0e483290ca8c9abb5729acbd36c49baa1e1fff9)) - [@kevinswiber](https://github.com/kevinswiber)
- name the last content change on every live watch frame - ([79ab365](https://github.com/kevinswiber/ratto/commit/79ab365cf7bbfb646a0618b4e7cfd2f5546028fa)) - [@kevinswiber](https://github.com/kevinswiber)
- add bottom-row fast path and self-healing full repaints - ([75f5345](https://github.com/kevinswiber/ratto/commit/75f5345c21f4465f6d58b7bf3890257fb148c110)) - [@kevinswiber](https://github.com/kevinswiber)
- rewrite only changed rows when a repaint is eligible - ([cd8f210](https://github.com/kevinswiber/ratto/commit/cd8f210f9171146bba08308221e6676d2b41b79f)) - [@kevinswiber](https://github.com/kevinswiber)
- retain the painted rows in the inline renderer - ([7430c1a](https://github.com/kevinswiber/ratto/commit/7430c1a575df8dfa5cf1b0bf8bd0eb3397f58374)) - [@kevinswiber](https://github.com/kevinswiber)
- write a watch frame snapshot on S - ([ed1f009](https://github.com/kevinswiber/ratto/commit/ed1f00989531895c4a572fd94ff35f319b50d674)) - [@kevinswiber](https://github.com/kevinswiber)
- add snapshot and wrap flags to watch - ([1730a2c](https://github.com/kevinswiber/ratto/commit/1730a2cffd5f5dab66db2cdcf1a5dfbf87cefe67)) - [@kevinswiber](https://github.com/kevinswiber)
- toggle wrapping and scroll a watch frame horizontally - ([5fb4063](https://github.com/kevinswiber/ratto/commit/5fb40637d006ee188b0cc6c8b3675059391743b3)) - [@kevinswiber](https://github.com/kevinswiber)
- scroll a frozen watch frame with less-style keys - ([3e9379a](https://github.com/kevinswiber/ratto/commit/3e9379a182a0055c4c9144ec4022a115a6e0c9b8)) - [@kevinswiber](https://github.com/kevinswiber)
- resolve a lone escape after a hold of input silence - ([117e544](https://github.com/kevinswiber/ratto/commit/117e54463a31e461836949ec3f00532aa4c67f89)) - [@kevinswiber](https://github.com/kevinswiber)
- decode navigation keys in the tap scanner - ([996df68](https://github.com/kevinswiber/ratto/commit/996df687b2e0310b780964595a63078cdbb58187)) - [@kevinswiber](https://github.com/kevinswiber)
- add an SGR-preserving horizontal chop to measure - ([943ea37](https://github.com/kevinswiber/ratto/commit/943ea372c13f4ed62c1b82ef08668c60e01975b9)) - [@kevinswiber](https://github.com/kevinswiber)
- add snapshot naming, body, and collision-safe writer - ([0ca0e34](https://github.com/kevinswiber/ratto/commit/0ca0e348e9a9e51bdf6643cc2ae02e05496821fb)) - [@kevinswiber](https://github.com/kevinswiber)
- add the scroll window state machine - ([e8b7bcd](https://github.com/kevinswiber/ratto/commit/e8b7bcd56d7f2d4070ea8a0f267eef015ccfb049)) - [@kevinswiber](https://github.com/kevinswiber)
#### Bug Fixes
- size the pager park ack for starved schedulers - ([d344509](https://github.com/kevinswiber/ratto/commit/d344509311241087b67406e8efe9d553ea285251)) - [@kevinswiber](https://github.com/kevinswiber)
- give the pager handoff test CI headroom and appease the windows filter lint - ([891785d](https://github.com/kevinswiber/ratto/commit/891785daf64f006643d3905da28f0eb5897b8c70)) - [@kevinswiber](https://github.com/kevinswiber)
- keep one footer time style across live and paused frames - ([0e28ad3](https://github.com/kevinswiber/ratto/commit/0e28ad361855790036ccc545235a2b05b3aeb40f)) - [@kevinswiber](https://github.com/kevinswiber)
- compile the unix-only respawn request on windows - ([d6769e4](https://github.com/kevinswiber/ratto/commit/d6769e429cea3c068733135f5cc09f5ed6b05bde)) - [@kevinswiber](https://github.com/kevinswiber)
- collapse a live window in place on resume - ([62d159d](https://github.com/kevinswiber/ratto/commit/62d159d639d0462b1f5fc1983643b1f847b0c802)) - [@kevinswiber](https://github.com/kevinswiber)
#### Documentation
- document watch triggers and the two-speed dashboard pattern - ([f736363](https://github.com/kevinswiber/ratto/commit/f736363e32f6e68833ebd5c60c4250fd4f72f05c)) - [@kevinswiber](https://github.com/kevinswiber)
- document the responsive watch loop - ([4f8f97a](https://github.com/kevinswiber/ratto/commit/4f8f97a3b296d5717d309b39777ac4115ce2cbd7)) - [@kevinswiber](https://github.com/kevinswiber)
- document the watch change markers and time toggle - ([76b0af8](https://github.com/kevinswiber/ratto/commit/76b0af8c58b6b1ab786ef3ce558d196d3865bef5)) - [@kevinswiber](https://github.com/kevinswiber)
- scrolling never pauses; freezing is explicit - ([c8ed470](https://github.com/kevinswiber/ratto/commit/c8ed47095b719a6da9e3ddfd9f527c4ece363efd)) - [@kevinswiber](https://github.com/kevinswiber)
- document live scrolling, staleness rows, and time scrub - ([fca079a](https://github.com/kevinswiber/ratto/commit/fca079a28214b1b7da6c268803018bc0dfb10c8c)) - [@kevinswiber](https://github.com/kevinswiber)
- document watch scrollback and snapshots - ([92d343d](https://github.com/kevinswiber/ratto/commit/92d343dccaf571b522d7c87d93e7d4cd541bbfd4)) - [@kevinswiber](https://github.com/kevinswiber)
#### Refactoring
- drive the watch loop from one schedule - ([53bea8e](https://github.com/kevinswiber/ratto/commit/53bea8e9188040494fe3cb79ccd0c810a9eab4ba)) - [@kevinswiber](https://github.com/kevinswiber)
- build the watch child command apart from running it - ([5ae0cf0](https://github.com/kevinswiber/ratto/commit/5ae0cf0a30b7263eeabcf8a4f6e01731c1858b4b)) - [@kevinswiber](https://github.com/kevinswiber)
- single-source the status-row time segments - ([62462fd](https://github.com/kevinswiber/ratto/commit/62462fd349691d8c3beaecd778c24be2929bcefe)) - [@kevinswiber](https://github.com/kevinswiber)
- thread a frame mode through the watch loop - ([53198b6](https://github.com/kevinswiber/ratto/commit/53198b6e2c63697b87441559fb8e31084929daaf)) - [@kevinswiber](https://github.com/kevinswiber)
- consolidate the watch paint sites into one repaint helper - ([b2e1a18](https://github.com/kevinswiber/ratto/commit/b2e1a18b50ee5e2deaf65f0b8095f10ce8684679)) - [@kevinswiber](https://github.com/kevinswiber)
- route watch key dispatch through one binding table - ([7453d7b](https://github.com/kevinswiber/ratto/commit/7453d7b934f799c20d99f47b190b5748e6494add)) - [@kevinswiber](https://github.com/kevinswiber)
- extract the watch frame paint - ([55798f8](https://github.com/kevinswiber/ratto/commit/55798f8b450089bcda696facd55ca8597f8c852e)) - [@kevinswiber](https://github.com/kevinswiber)

- - -

## [v0.6.0](https://github.com/kevinswiber/ratto/compare/bedddc52dd95ce9b57b010643034ba1d08f82d7d..v0.6.0) - 2026-07-27
#### Features
- add cursor and placeholder theme tokens - ([722d8a1](https://github.com/kevinswiber/ratto/commit/722d8a1a97a8be53b4fead598201669874b3825f)) - [@kevinswiber](https://github.com/kevinswiber)
- add selection and match theme tokens - ([e1cb3ab](https://github.com/kevinswiber/ratto/commit/e1cb3ab6ded83f51dcd1dce24b6aa24e8b09231d)) - [@kevinswiber](https://github.com/kevinswiber)
#### Bug Fixes
- pin on-accent to the 256-color cube - ([945ca2f](https://github.com/kevinswiber/ratto/commit/945ca2fb06fcbc56ea2bd42d16af12c6d8722590)) - [@kevinswiber](https://github.com/kevinswiber)
#### Documentation
- document the selection, match, cursor, and placeholder tokens - ([57aa2d1](https://github.com/kevinswiber/ratto/commit/57aa2d17b43bbc5cfea07bed7d5af327e40b4aed)) - [@kevinswiber](https://github.com/kevinswiber)
#### Refactoring
- read the input placeholder and caret from their ui tokens - ([c1a83a1](https://github.com/kevinswiber/ratto/commit/c1a83a18db205ba785142e94bf73c905064d7f19)) - [@kevinswiber](https://github.com/kevinswiber)
- read the filter surface from its ui tokens - ([d0f5b9a](https://github.com/kevinswiber/ratto/commit/d0f5b9a06c709768aae13dc27e72c9488c190bcf)) - [@kevinswiber](https://github.com/kevinswiber)
- read the choose cursor row from the selection token - ([a86c190](https://github.com/kevinswiber/ratto/commit/a86c1904dc36d9662d221de67acce3ae696dc0cb)) - [@kevinswiber](https://github.com/kevinswiber)
- derive the palettes from a reference tier - ([89bc6cc](https://github.com/kevinswiber/ratto/commit/89bc6cc59861477259c3d4c775a507cdcb643f19)) - [@kevinswiber](https://github.com/kevinswiber)

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