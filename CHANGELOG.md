# Changelog
All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

- - -
## [v0.17.0](https://github.com/kevinswiber/ratto/compare/99f0a6016defd672b98c977b4c4afeea8d3ae5c8..v0.17.0) - 2026-08-06
#### Features
- the zoomed badge carries its place in the cycle - ([e6cb2b7](https://github.com/kevinswiber/ratto/commit/e6cb2b7542eff7d6b7baeb2fe93abbcedbb5fc8e)) - [@kevinswiber](https://github.com/kevinswiber)
- esc drops the focus before it drops the frame scroll - ([a382c80](https://github.com/kevinswiber/ratto/commit/a382c806bae567db8e3c1e6f46a47659a93cc3b6)) - [@kevinswiber](https://github.com/kevinswiber)
- tab carries the zoom along the reading order - ([55b5274](https://github.com/kevinswiber/ratto/commit/55b52744b23efa158d657985fca7117dd62292a7)) - [@kevinswiber](https://github.com/kevinswiber)
- enter zooms a focused pane and pages it once zoomed - ([2ab974f](https://github.com/kevinswiber/ratto/commit/2ab974f9ae706e5a0e5aa3d15aa6b4ddcaa7948e)) - [@kevinswiber](https://github.com/kevinswiber)
- focus works from the scrolled frame and the viewport follows it - ([44c90c9](https://github.com/kevinswiber/ratto/commit/44c90c996e4b76e016b3810a61a9817d00a5d54f)) - [@kevinswiber](https://github.com/kevinswiber)
- space collapses the focused pane to one row that keeps its child - ([43ca239](https://github.com/kevinswiber/ratto/commit/43ca2399566c90459854fd3155af09290bdcc056)) - [@kevinswiber](https://github.com/kevinswiber)
- a collapsed pane renders as one row naming itself - ([0bc7f57](https://github.com/kevinswiber/ratto/commit/0bc7f574d0ba505642007fdf2f3cb55bb0a97584)) - [@kevinswiber](https://github.com/kevinswiber)
- a zoomed pane keeps its viewport and wears the badge - ([31d54bc](https://github.com/kevinswiber/ratto/commit/31d54bc0975bc5fa2fa62ee66af5b061322b2bef)) - [@kevinswiber](https://github.com/kevinswiber)
- a zoomed batch pane earns an honest width within one debounced run - ([bda7113](https://github.com/kevinswiber/ratto/commit/bda711396ec114276df419d6cf3b41595abee81b)) - [@kevinswiber](https://github.com/kevinswiber)
- z zooms the focused pane to the frame and back - ([6edb6bf](https://github.com/kevinswiber/ratto/commit/6edb6bf1994058bf5e0190a253b82609a7cb36bc)) - [@kevinswiber](https://github.com/kevinswiber)
- the geometry derivation answers a zoom without reading as a resize - ([ab441da](https://github.com/kevinswiber/ratto/commit/ab441dac4e2b32749f805834ffbb38e996b96099)) - [@kevinswiber](https://github.com/kevinswiber)
- v pages the focused pane's whole retained body - ([d00c94f](https://github.com/kevinswiber/ratto/commit/d00c94f160773322e9ae5b49630358edd35f9d52)) - [@kevinswiber](https://github.com/kevinswiber)
- a scrolled pane says where its window is - ([a68b34c](https://github.com/kevinswiber/ratto/commit/a68b34cdd0a99d1fbd7e27d44176cde3a8697901)) - [@kevinswiber](https://github.com/kevinswiber)
- the scroll keys drive the focused pane's own window - ([4c33c9f](https://github.com/kevinswiber/ratto/commit/4c33c9fb54e845a8e9aa7f96521f52374e2f60d1)) - [@kevinswiber](https://github.com/kevinswiber)
- a pane renders through a viewport its caller hands it - ([34a82f8](https://github.com/kevinswiber/ratto/commit/34a82f81162e66a28a5bc341172bb041bc345e86)) - [@kevinswiber](https://github.com/kevinswiber)
- a pane window knows its pin and its declared rest - ([8487d55](https://github.com/kevinswiber/ratto/commit/8487d55af0a6b27c4d0f547f61ec0b6f567ea835)) - [@kevinswiber](https://github.com/kevinswiber)
- the focused pane wears the accent border and the footer names it - ([ad99437](https://github.com/kevinswiber/ratto/commit/ad99437be342d1d7b45934dd3f4af9b4ede81059)) - [@kevinswiber](https://github.com/kevinswiber)
- tab cycles and alt-hjkl moves a pane focus the loop now holds - ([e1a3105](https://github.com/kevinswiber/ratto/commit/e1a3105caff1ac9f1e256bfa34f81be9e2f55ed3)) - [@kevinswiber](https://github.com/kevinswiber)
- one geometry derivation, and the repaint gate sees the per-pane view - ([1148557](https://github.com/kevinswiber/ratto/commit/1148557b31fb0c963998023eb012b2ea1dce3c80)) - [@kevinswiber](https://github.com/kevinswiber)
- the layout tree answers where each pane lands and in what order - ([2e128dc](https://github.com/kevinswiber/ratto/commit/2e128dc85466cad44a28610b33d18e47c88cf82d)) - [@kevinswiber](https://github.com/kevinswiber)
- the unix scanner decodes tab, backtab, space, and the meta encoding - ([a4a400d](https://github.com/kevinswiber/ratto/commit/a4a400d021ea9775cebf89af9ef45e11e5dccfda)) - [@kevinswiber](https://github.com/kevinswiber)
- alt-modified printables arrive as their own key - ([9abd7ff](https://github.com/kevinswiber/ratto/commit/9abd7ff85da00f207aa60a938b313aa75b736ded)) - [@kevinswiber](https://github.com/kevinswiber)
#### Bug Fixes
- a horizontal shift is inert on a pane board - ([6793506](https://github.com/kevinswiber/ratto/commit/67935069d681a3fa6d57ced50e749becb75fc295)) - [@kevinswiber](https://github.com/kevinswiber)
- expand tabs before dashboard layout - ([5b9a7f0](https://github.com/kevinswiber/ratto/commit/5b9a7f06e6c04f7dff2c1bdeac2a56e39106f465)) - [@kevinswiber](https://github.com/kevinswiber)
- a quoted word in a platform-shell body reaches cmd verbatim - ([99f0a60](https://github.com/kevinswiber/ratto/commit/99f0a6016defd672b98c977b4c4afeea8d3ae5c8)) - [@kevinswiber](https://github.com/kevinswiber)
#### Documentation
- the esc ladder, the enter ladder, and the zoom carry reach the help - ([8f3823a](https://github.com/kevinswiber/ratto/commit/8f3823aa13b170897071ffe4dd9a46386f72e7c6)) - [@kevinswiber](https://github.com/kevinswiber)
- the pane gestures reach the key reference, the readme, and the examples - ([04397f9](https://github.com/kevinswiber/ratto/commit/04397f9609148f70501f61b48342d548b668d69f)) - [@kevinswiber](https://github.com/kevinswiber)

- - -

## [v0.16.0](https://github.com/kevinswiber/ratto/compare/ecca9dcada7ab1f88aa5d4478f6f88597dcec5b7..v0.16.0) - 2026-08-05
#### Features
- script bodies resolve with their own rules - ([b734017](https://github.com/kevinswiber/ratto/commit/b7340178b1489e630a6ca61c4c919e604d6c9ad9)) - [@kevinswiber](https://github.com/kevinswiber)
- the script key joins the pane grammar - ([764a86d](https://github.com/kevinswiber/ratto/commit/764a86d006776a1c12a7691594a87a56fddd6301)) - [@kevinswiber](https://github.com/kevinswiber)
- shebang bodies materialize once into a private per-run directory - ([38a3285](https://github.com/kevinswiber/ratto/commit/38a32858aa092db191ad9b2e3f1836842368276c)) - [@kevinswiber](https://github.com/kevinswiber)
- sources carry a program, argv or script body - ([b2c7587](https://github.com/kevinswiber/ratto/commit/b2c7587aab1aa6e7b7e3ca72effd5f36a166af40)) - [@kevinswiber](https://github.com/kevinswiber)
- the interpreter arm's tables answer name, flags, extension, bytes - ([65fc75a](https://github.com/kevinswiber/ratto/commit/65fc75a7b8ef691927ebf79ed1bd94babba9b4a2)) - [@kevinswiber](https://github.com/kevinswiber)
- a shebang parser decides a body's route - ([ecca9dc](https://github.com/kevinswiber/ratto/commit/ecca9dcada7ab1f88aa5d4478f6f88597dcec5b7)) - [@kevinswiber](https://github.com/kevinswiber)
#### Bug Fixes
- a long pane id no longer overflows the script file name - ([7cb5eca](https://github.com/kevinswiber/ratto/commit/7cb5eca32dd6644296882a49c71858c46f69e282)) - [@kevinswiber](https://github.com/kevinswiber)
#### Documentation
- the script-body story, dedent rule included - ([fa0a1b7](https://github.com/kevinswiber/ratto/commit/fa0a1b7dd1ab25c70a494669215b7f19a5566bee)) - [@kevinswiber](https://github.com/kevinswiber)

- - -

## [v0.15.0](https://github.com/kevinswiber/ratto/compare/1c4bfbf011ebc84b659fa7ede959970ae3a1c9b4..v0.15.0) - 2026-08-05
#### Features
- the inherit-command guard covers shell dialect changes - ([1b939bb](https://github.com/kevinswiber/ratto/commit/1b939bbfa0da9c93e76fc639246bba184b8d3354)) - [@kevinswiber](https://github.com/kevinswiber)
- the dashboard shell key takes a shell name - ([1f7c34b](https://github.com/kevinswiber/ratto/commit/1f7c34b3475a5a72cdc3aaed398de25a8802c443)) - [@kevinswiber](https://github.com/kevinswiber)
- --shell=NAME selects the shell the script runs through - ([7ae7457](https://github.com/kevinswiber/ratto/commit/7ae7457464e2ba8ec9571f50f9c999f2067c029c)) - [@kevinswiber](https://github.com/kevinswiber)
- named shells get their dialect's command flags - ([f97fcb6](https://github.com/kevinswiber/ratto/commit/f97fcb6c60b83733e5dfd29303e4c9ee3d5d671e)) - [@kevinswiber](https://github.com/kevinswiber)
#### Bug Fixes
- a spawn error under a shell names the shell, not the script - ([331d008](https://github.com/kevinswiber/ratto/commit/331d008d6e2e927afdea8a79a1a187d1184f7d4d)) - [@kevinswiber](https://github.com/kevinswiber)
#### Documentation
- the shell selection story, dialect table included - ([291bb7b](https://github.com/kevinswiber/ratto/commit/291bb7b79c3e3b21356693c894045b28386523d0)) - [@kevinswiber](https://github.com/kevinswiber)
#### Refactoring
- sources carry a shell mode, not a bool - ([1c4bfbf](https://github.com/kevinswiber/ratto/commit/1c4bfbf011ebc84b659fa7ede959970ae3a1c9b4)) - [@kevinswiber](https://github.com/kevinswiber)

- - -

## [v0.14.0](https://github.com/kevinswiber/ratto/compare/15e6b3afff91561cfc9d3e315ad031b5558415b6..v0.14.0) - 2026-08-04
#### Features
- append mode speaks the child's exit only when it changes - ([49cf9a7](https://github.com/kevinswiber/ratto/commit/49cf9a71ce53898c96f2ba6c0579590fd37c7576)) - [@kevinswiber](https://github.com/kevinswiber)
- append mode speaks chrome events as their own rows - ([147740c](https://github.com/kevinswiber/ratto/commit/147740c35d6395207400228641f6cf106840363c)) - [@kevinswiber](https://github.com/kevinswiber)
- append mode answers four keys and one banner replaces the footer - ([1f5f095](https://github.com/kevinswiber/ratto/commit/1f5f095db25daa158d7fed583cd9f68b1b00445e)) - [@kevinswiber](https://github.com/kevinswiber)
- --append streams distinct frames to the scrollback - ([990bd3b](https://github.com/kevinswiber/ratto/commit/990bd3b2aeb00f4266c9f8d735278b54500fa6d7)) - [@kevinswiber](https://github.com/kevinswiber)
- watch accepts --append and refuses the flags it cannot serve - ([15e6b3a](https://github.com/kevinswiber/ratto/commit/15e6b3afff91561cfc9d3e315ad031b5558415b6)) - [@kevinswiber](https://github.com/kevinswiber)
#### Documentation
- the third screen contract — --append and the scrollback bargain - ([8c3bbfd](https://github.com/kevinswiber/ratto/commit/8c3bbfd49147dc4bb995d56291c4651eaa406625)) - [@kevinswiber](https://github.com/kevinswiber)

- - -

## [v0.13.0](https://github.com/kevinswiber/ratto/compare/bfd8ba38b90964a4a0002fcd7694ada480956d1c..v0.13.0) - 2026-08-04
#### Features
- a legacy-codepage child renders clean on Windows - ([dee3243](https://github.com/kevinswiber/ratto/commit/dee324369e15c638df4a3da800021f30f01cbcb7)) - [@kevinswiber](https://github.com/kevinswiber)
- the supersede's force-kill arrives on a generation-guarded deadline - ([6ce21a8](https://github.com/kevinswiber/ratto/commit/6ce21a82a3dbcaa1ca3050114a9d04434fc9c37a)) - [@kevinswiber](https://github.com/kevinswiber)
- a superseded live child is asked before it is killed - ([bfd8ba3](https://github.com/kevinswiber/ratto/commit/bfd8ba38b90964a4a0002fcd7694ada480956d1c)) - [@kevinswiber](https://github.com/kevinswiber)
#### Documentation
- the Windows section says legacy-codepage child output decodes - ([a394afe](https://github.com/kevinswiber/ratto/commit/a394afe91fb8c2da185fe1701dfaadf388a17c5c)) - [@kevinswiber](https://github.com/kevinswiber)
- the live-pane trigger restart asks before it kills - ([f4e737c](https://github.com/kevinswiber/ratto/commit/f4e737c44b6edd15802e3f548c57d43b0f3941ae)) - [@kevinswiber](https://github.com/kevinswiber)
#### Refactoring
- one display-side decode for child output - ([c51650c](https://github.com/kevinswiber/ratto/commit/c51650c6f3c5c6555943f1ed21431845304216e4)) - [@kevinswiber](https://github.com/kevinswiber)

- - -

## [v0.12.0](https://github.com/kevinswiber/ratto/compare/f966dede69b7f8656b28cbf41c2555da05771345..v0.12.0) - 2026-08-02
#### Features
- the wheel scrolls a captured frame - ([1b9e446](https://github.com/kevinswiber/ratto/commit/1b9e44626268c4271ee8cfbed0b08cebbbfdc310)) - [@kevinswiber](https://github.com/kevinswiber)
- rat watch and dashboard learn --fullscreen - ([17ae581](https://github.com/kevinswiber/ratto/commit/17ae581952ea677387d3e69dc6ab2f69efb19f01)) - [@kevinswiber](https://github.com/kevinswiber)
- the scrolled row carries the time and cadence - ([444e3c7](https://github.com/kevinswiber/ratto/commit/444e3c7174cd6decb5536db5ff6051a32dc5203f)) - [@kevinswiber](https://github.com/kevinswiber)
- a step past the newest frame returns to the live view - ([fb09a32](https://github.com/kevinswiber/ratto/commit/fb09a32e1e6abb81cbe275fa15fec1443381716e)) - [@kevinswiber](https://github.com/kevinswiber)
#### Bug Fixes
- windows mouse capture goes through the console mode - ([eaff696](https://github.com/kevinswiber/ratto/commit/eaff696bb1e5d9205b19a1d11db33c28601b3a31)) - [@kevinswiber](https://github.com/kevinswiber)
- the gutter takes its columns from the layout, not the border - ([0141f25](https://github.com/kevinswiber/ratto/commit/0141f25c212ceebdc853dbe51a78640136455520)) - [@kevinswiber](https://github.com/kevinswiber)
- a changed bar cell recolors its ink instead of inverting it - ([91c6bc0](https://github.com/kevinswiber/ratto/commit/91c6bc09a80fc335d2c8796db40a85b615f30d4f)) - [@kevinswiber](https://github.com/kevinswiber)
- a highlight clips at the pane's own edge - ([808fee8](https://github.com/kevinswiber/ratto/commit/808fee89d150a089ed2096430805cf958c80798b)) - [@kevinswiber](https://github.com/kevinswiber)
- a pane's padding and border keep rat's own colors - ([de94f25](https://github.com/kevinswiber/ratto/commit/de94f2588f3cee4162fc266492493d3ecfffd8dc)) - [@kevinswiber](https://github.com/kevinswiber)
- a row closes what it opens before the chrome paints - ([29dd1db](https://github.com/kevinswiber/ratto/commit/29dd1db7b75cd1dfc27f37059b60caea306f5685)) - [@kevinswiber](https://github.com/kevinswiber)

- - -

## [v0.11.0](https://github.com/kevinswiber/ratto/compare/7ba9007d68f818278c54a73bf1867a134a6733df..v0.11.0) - 2026-08-02
#### Features
- the caret is the terminal's own cursor - ([738fef3](https://github.com/kevinswiber/ratto/commit/738fef3897ff2c5d1f86f15ea07afb0bb5ea4a64)) - [@kevinswiber](https://github.com/kevinswiber)
- the hardware cursor rests on the caret - ([6eef28f](https://github.com/kevinswiber/ratto/commit/6eef28f3e5507df1c9f7c0c2d41c1c3b66e38e8d)) - [@kevinswiber](https://github.com/kevinswiber)
- rat style learns --reverse - ([54bfb42](https://github.com/kevinswiber/ratto/commit/54bfb42f6a1f3c3d3a48e6cfd449b1009da2c8c9)) - [@kevinswiber](https://github.com/kevinswiber)
#### Bug Fixes
- the confirm buttons are spaced in cells, not bytes - ([192b879](https://github.com/kevinswiber/ratto/commit/192b8792c387b5645aa133749c5cb86107370791)) - [@kevinswiber](https://github.com/kevinswiber)
- filter highlights land on the cell they mean - ([1fe5729](https://github.com/kevinswiber/ratto/commit/1fe57297a64511aecca1281eb89656c42bad6ccc)) - [@kevinswiber](https://github.com/kevinswiber)
- a wide glyph no longer shifts the line it sits in - ([446bb5f](https://github.com/kevinswiber/ratto/commit/446bb5f6e2d3d5903e21f723f6ff0b5e4f03c5fb)) - [@kevinswiber](https://github.com/kevinswiber)
- the input field scrolls instead of pinning the caret - ([4065a65](https://github.com/kevinswiber/ratto/commit/4065a65862bbffd97d718654e8c7a58281c78a75)) - [@kevinswiber](https://github.com/kevinswiber)
- caret columns count terminal cells, not chars - ([3098c8b](https://github.com/kevinswiber/ratto/commit/3098c8b6ea2591a16d87eb553a651fc984fbb070)) - [@kevinswiber](https://github.com/kevinswiber)
- a resize drops the park instead of trusting it - ([85cdbaf](https://github.com/kevinswiber/ratto/commit/85cdbaf2345cc4e7685b48e9c22032c87be20071)) - [@kevinswiber](https://github.com/kevinswiber)
- the input caret reaches the terminal - ([7ba9007](https://github.com/kevinswiber/ratto/commit/7ba9007d68f818278c54a73bf1867a134a6733df)) - [@kevinswiber](https://github.com/kevinswiber)
#### Documentation
- the caret comments catch up with the bare cursor - ([fc49b9f](https://github.com/kevinswiber/ratto/commit/fc49b9f7d8ce2716f46d9dd3cb8dc5ad86bf3d7a)) - [@kevinswiber](https://github.com/kevinswiber)

- - -

## [v0.10.0](https://github.com/kevinswiber/ratto/compare/744c209062259b108dd40ce5971513dee68e7beb..v0.10.0) - 2026-08-02
#### Features
- the terminal tab carries the dashboard's title - ([6d53659](https://github.com/kevinswiber/ratto/commit/6d5365938c401d99302d1761d3df3bcdfe2aadcc)) - [@kevinswiber](https://github.com/kevinswiber)
- the title role reads as plain text - ([f65955f](https://github.com/kevinswiber/ratto/commit/f65955f323a548f18044c3a84979199630a6c130)) - [@kevinswiber](https://github.com/kevinswiber)
- a title may come from a pane, by fragment reference - ([7404c7a](https://github.com/kevinswiber/ratto/commit/7404c7ae3609d7c7a89493aa958da4fe728ea7e4)) - [@kevinswiber](https://github.com/kevinswiber)
- the ? reference grows a diagnostics section - ([f4c93b9](https://github.com/kevinswiber/ratto/commit/f4c93b988c9683b3e5e2cd651b7b01b6d16b1733)) - [@kevinswiber](https://github.com/kevinswiber)
- duplicate ids are first-win diagnostics, never load failures - ([7907adf](https://github.com/kevinswiber/ratto/commit/7907adf76e46b77d0a9a634f96a84e9053135973)) - [@kevinswiber](https://github.com/kevinswiber)
- a pane's identity is an id, and an id is a URI fragment - ([744c209](https://github.com/kevinswiber/ratto/commit/744c209062259b108dd40ce5971513dee68e7beb)) - [@kevinswiber](https://github.com/kevinswiber)
#### Documentation
- the last invariant guard speaks id, not name - ([3324fde](https://github.com/kevinswiber/ratto/commit/3324fde02ca9de3ab66e3e9192e3fca9ed5c9232)) - [@kevinswiber](https://github.com/kevinswiber)
- ids, fragment refs, the pane-sourced title, and the tab - ([3bb6b3b](https://github.com/kevinswiber/ratto/commit/3bb6b3bf486ed5d1b60c5373559eb16a0f74df28)) - [@kevinswiber](https://github.com/kevinswiber)

- - -

## [v0.9.0](https://github.com/kevinswiber/ratto/compare/5213b013acd128b5ce39ab5a9c410be2d76b73eb..v0.9.0) - 2026-08-02
#### Features
- a dashboard-level title, one bold line above the panes - ([56dc5f5](https://github.com/kevinswiber/ratto/commit/56dc5f5cacb07e88d65e10c94aa463c3a8330c8a)) - [@kevinswiber](https://github.com/kevinswiber)
- color the syntax-error snippet when the terminal can take it - ([70c1725](https://github.com/kevinswiber/ratto/commit/70c1725a6c6ed77ff3cee6b88c23904073976898)) - [@kevinswiber](https://github.com/kevinswiber)
- point a KDL syntax error into the source, rustc-style - ([6e88883](https://github.com/kevinswiber/ratto/commit/6e88883923f35f7656d553316852532f2428e13a)) - [@kevinswiber](https://github.com/kevinswiber)
- an opt-in --once-timeout bounds the wait with exit 124 - ([9823aed](https://github.com/kevinswiber/ratto/commit/9823aede5e69c8627c1fc9f9158e371b6c8b018f)) - [@kevinswiber](https://github.com/kevinswiber)
- let a timeout carry what it waited on - ([2c81847](https://github.com/kevinswiber/ratto/commit/2c8184760c9d58c032aad00d40aaa2e125b0bd4c)) - [@kevinswiber](https://github.com/kevinswiber)
- a quiet --once dashboard says which pane it is waiting on - ([3b156e7](https://github.com/kevinswiber/ratto/commit/3b156e7a2bfd167159936917ffa9b0a3d15828cf)) - [@kevinswiber](https://github.com/kevinswiber)
- compose the notice for a --once run that has gone quiet - ([562baac](https://github.com/kevinswiber/ratto/commit/562baacf31efbfa5446f09b196f3902aa04365d9)) - [@kevinswiber](https://github.com/kevinswiber)
- a bare setting or pane key on a container gets a real answer - ([e9f3e52](https://github.com/kevinswiber/ratto/commit/e9f3e520aff59043662baca81e3096c30a7d3c1b)) - [@kevinswiber](https://github.com/kevinswiber)
- a bare key after a pane's name teaches the property spelling - ([327351b](https://github.com/kevinswiber/ratto/commit/327351bbc7358431e5d6a8ec5fdaa561504f7fa8)) - [@kevinswiber](https://github.com/kevinswiber)
- a bare key on defaults teaches the property spelling - ([f59263c](https://github.com/kevinswiber/ratto/commit/f59263cf84829ae4beee3a212206b953b17672cb)) - [@kevinswiber](https://github.com/kevinswiber)
- find the pane key or setting a bare argument names - ([a479b18](https://github.com/kevinswiber/ratto/commit/a479b18ed0cba44af263b3d6d96e61b73cdaa287)) - [@kevinswiber](https://github.com/kevinswiber)
- a KDL syntax error names its line and column - ([41c8043](https://github.com/kevinswiber/ratto/commit/41c8043c882e647116013d2a9fd6c5e73c397f3a)) - [@kevinswiber](https://github.com/kevinswiber)
- frame a placed syntax error as one line - ([4b87be6](https://github.com/kevinswiber/ratto/commit/4b87be62fe731b432d5b3a2176a9572c08d06a8a)) - [@kevinswiber](https://github.com/kevinswiber)
- place a byte offset into 1-based line and column - ([bb03093](https://github.com/kevinswiber/ratto/commit/bb030939262fc1c02a8f374ab868daef7526b6bf)) - [@kevinswiber](https://github.com/kevinswiber)
- let a trigger restart a live child through a revocable kill - ([b84875b](https://github.com/kevinswiber/ratto/commit/b84875ba7b4aa11b3c3752e7be3dd23a383d3d37)) - [@kevinswiber](https://github.com/kevinswiber)
- stop a live pane's chrome claiming a cadence - ([f6f1174](https://github.com/kevinswiber/ratto/commit/f6f11742810b70e182eaa4edfac7dfaba7b51562)) - [@kevinswiber](https://github.com/kevinswiber)
- paint a pane when its long-lived child emits, not when it exits - ([e113baf](https://github.com/kevinswiber/ratto/commit/e113bafd8d1e15787ffeaf4f11ed480fbf9d3081)) - [@kevinswiber](https://github.com/kevinswiber)
- let a long-lived source offer its output before it exits - ([33a269e](https://github.com/kevinswiber/ratto/commit/33a269e999bb50c8cd377e5b9ae058af996813ea)) - [@kevinswiber](https://github.com/kevinswiber)
- let a pane declare that its child is long-lived - ([c8301c8](https://github.com/kevinswiber/ratto/commit/c8301c8a68df467b41eb8b9fca9dcaf36bdfeb46)) - [@kevinswiber](https://github.com/kevinswiber)
- give a live source bounded buffers and a one-slot outbox - ([323d9b0](https://github.com/kevinswiber/ratto/commit/323d9b0296a6c5a1434f578a91208377150c7af2)) - [@kevinswiber](https://github.com/kevinswiber)
- let a line cap be read without being consumed - ([619cbfd](https://github.com/kevinswiber/ratto/commit/619cbfdf9a17c0e5a3b99407c3b75d1fdf96a977)) - [@kevinswiber](https://github.com/kevinswiber)
#### Bug Fixes
- never echo an unwritable value into a taught spelling - ([aea434e](https://github.com/kevinswiber/ratto/commit/aea434ef6d147b14c0ca1e108bff011c4bd09755)) - [@kevinswiber](https://github.com/kevinswiber)
- lead a pane's spawn error with the reason, path last - ([b20a581](https://github.com/kevinswiber/ratto/commit/b20a581d1a8da2313886b4ed8b5f315b10543445)) - [@kevinswiber](https://github.com/kevinswiber)
- recognise a CRLF terminator whole, so Windows panes paint - ([1bea77a](https://github.com/kevinswiber/ratto/commit/1bea77a50141ca040fce6170fa5bb6d045a92d09)) - [@kevinswiber](https://github.com/kevinswiber)
- bound what spin retains from its child - ([1075070](https://github.com/kevinswiber/ratto/commit/1075070234bd858e9c8e7d55abc4a45ee9be5a39)) - [@kevinswiber](https://github.com/kevinswiber)
#### Documentation
- keep a comment's rationale without its private reference - ([3d94124](https://github.com/kevinswiber/ratto/commit/3d9412436bcdf764c605f8d8c942a90feb1746a8)) - [@kevinswiber](https://github.com/kevinswiber)
- the head-line comment describes the head, not the old rule - ([de2463e](https://github.com/kevinswiber/ratto/commit/de2463eacca96ab9f5d78098f4a8024e00176bf7)) - [@kevinswiber](https://github.com/kevinswiber)
- the --once diagnostic and --once-timeout, where users read - ([c6df825](https://github.com/kevinswiber/ratto/commit/c6df825ced6c4753e48fa3f31b51c1ecfeb3682b)) - [@kevinswiber](https://github.com/kevinswiber)
- add a self-feeding tail example, and its cmd.exe spelling - ([033489f](https://github.com/kevinswiber/ratto/commit/033489f738c3a8d67cd3be33c45ce275b903c46f)) - [@kevinswiber](https://github.com/kevinswiber)
- document the live pane, and stop teaching TOML - ([55291e7](https://github.com/kevinswiber/ratto/commit/55291e715002dea11d860270e7af1502c65cf834)) - [@kevinswiber](https://github.com/kevinswiber)
- say what a line bound costs in bytes - ([e0c9346](https://github.com/kevinswiber/ratto/commit/e0c934630f301a13dfff8ed53ecb76b9655a5e9e)) - [@kevinswiber](https://github.com/kevinswiber)
#### Refactoring
- keep an exit status off anything but a completion - ([093dfb9](https://github.com/kevinswiber/ratto/commit/093dfb9be0b4752ea84e966e7e5848e5233ede30)) - [@kevinswiber](https://github.com/kevinswiber)
- give the retention bound its own home - ([6c448e9](https://github.com/kevinswiber/ratto/commit/6c448e970d337e6e96c0717cdf50314a4640dd8c)) - [@kevinswiber](https://github.com/kevinswiber)

- - -

## [v0.8.0](https://github.com/kevinswiber/ratto/compare/cecb3f54647e0b5005a9e73b3744ba877df2a08c..v0.8.0) - 2026-08-01
#### Features
- fence either side of a bracket, so an interval can be placed inside it - ([2c201e9](https://github.com/kevinswiber/ratto/commit/2c201e9a0ec190d107dd37e1e4dbcb66a05394ac)) - [@kevinswiber](https://github.com/kevinswiber)
- give the loop a way to ask a reader for a fresh proof of emptiness - ([de30d3d](https://github.com/kevinswiber/ratto/commit/de30d3d2059ce596083ab566832eec90d5343703)) - [@kevinswiber](https://github.com/kevinswiber)
- measure the bracket a write must be placed inside - ([388abdc](https://github.com/kevinswiber/ratto/commit/388abdc97b51779b7370d29c6cb73cc92c3655dc)) - [@kevinswiber](https://github.com/kevinswiber)
- say by how much, and on which side, a vetoing arrival missed - ([345c9bd](https://github.com/kevinswiber/ratto/commit/345c9bd3b96e0dcf36ec4ee35f7e304fb137bf18)) - [@kevinswiber](https://github.com/kevinswiber)
- record how the suspicion test answered, when asked - ([8e2b8f6](https://github.com/kevinswiber/ratto/commit/8e2b8f6549ae4363b67be1102bf7229b40587a3c)) - [@kevinswiber](https://github.com/kevinswiber)
- carry the reader's whole interval out to the loop - ([b816153](https://github.com/kevinswiber/ratto/commit/b81615334f4e52aa37ccf3c50ede50851181f754)) - [@kevinswiber](https://github.com/kevinswiber)
- let the trigger reader prove when its descriptor was empty - ([af60d18](https://github.com/kevinswiber/ratto/commit/af60d184c7b476dbdfcb55d421454479c69ba9e2)) - [@kevinswiber](https://github.com/kevinswiber)
- classify when a write could have happened, not who wrote it - ([47687c2](https://github.com/kevinswiber/ratto/commit/47687c224a9509b2e12cd6791ca03f425a406ea4)) - [@kevinswiber](https://github.com/kevinswiber)
- say so when a command's output outran what is kept - ([c32b02d](https://github.com/kevinswiber/ratto/commit/c32b02dce99f9908d58251b89997a31d92f3d9c9)) - [@kevinswiber](https://github.com/kevinswiber)
- retain the end of a pane's output that the pane keeps - ([3e5aed6](https://github.com/kevinswiber/ratto/commit/3e5aed63778a1e28caa997df24642a3abe1683fb)) - [@kevinswiber](https://github.com/kevinswiber)
- carry a retention policy to the child and its losses back - ([e4ae21b](https://github.com/kevinswiber/ratto/commit/e4ae21ba46d82808afa53f5b9bf06283fe0eb797)) - [@kevinswiber](https://github.com/kevinswiber)
- bound what a watch child's reader retains - ([6489bbd](https://github.com/kevinswiber/ratto/commit/6489bbd43d1d37db08797a08abdc8ff93d0c3c22)) - [@kevinswiber](https://github.com/kevinswiber)
- bound a single line so one endless line cannot exhaust memory - ([6f529d0](https://github.com/kevinswiber/ratto/commit/6f529d08369ad97d4cf4569ff2d1509f79a6c478)) - [@kevinswiber](https://github.com/kevinswiber)
- join a line a pipe split across two reads - ([7892910](https://github.com/kevinswiber/ratto/commit/7892910c80cbee79affb18b46f6fed7b95106db8)) - [@kevinswiber](https://github.com/kevinswiber)
- add a bounded accumulator for a child's output lines - ([1ffe601](https://github.com/kevinswiber/ratto/commit/1ffe6015d3273c8e4b6150b9bb46434d9dee873a)) - [@kevinswiber](https://github.com/kevinswiber)
- hand a reader's arrivals to the window, keyed by the trigger - ([8024cde](https://github.com/kevinswiber/ratto/commit/8024cdef3876fa911d1efe44287cf805ea33d029)) - [@kevinswiber](https://github.com/kevinswiber)
- record when a fifo or fd trigger arrived, and say so when one is lost - ([df00e8d](https://github.com/kevinswiber/ratto/commit/df00e8d55b3e66496c84a18b0ab3b8dd7d57e0ca)) - [@kevinswiber](https://github.com/kevinswiber)
- teach the looping badge where a user checks what a pane watches - ([35658f6](https://github.com/kevinswiber/ratto/commit/35658f6d56eb964445176ab7fc297895967e588b)) - [@kevinswiber](https://github.com/kevinswiber)
- name a suspected trigger loop in a one-shot notice row - ([230a1e5](https://github.com/kevinswiber/ratto/commit/230a1e599ce45be88fbc4d8da7566c120e45eda0)) - [@kevinswiber](https://github.com/kevinswiber)
- report a looping pane on its chrome row - ([a313a16](https://github.com/kevinswiber/ratto/commit/a313a167f1213a120ae4f28e06d4d82f40b924b3)) - [@kevinswiber](https://github.com/kevinswiber)
- evaluate the suspicion once per iteration into per-pane state - ([3ca96ca](https://github.com/kevinswiber/ratto/commit/3ca96ca96e662ca6bba0e5d266e3b59b0acf0eac)) - [@kevinswiber](https://github.com/kevinswiber)
- feed the loop's observations into the window - ([6d69901](https://github.com/kevinswiber/ratto/commit/6d6990112a64916216971f12660e829d1f4dba1a)) - [@kevinswiber](https://github.com/kevinswiber)
- stamp the watched union on the worker when a child exits - ([e90378e](https://github.com/kevinswiber/ratto/commit/e90378ea5d3d1a1377018c9e51a354916cec3dcc)) - [@kevinswiber](https://github.com/kevinswiber)
- decide which panes are feeding each other, and when to decline - ([ddf03f1](https://github.com/kevinswiber/ratto/commit/ddf03f13a97f4eaaad1de51a1386d5bb6aebdea2)) - [@kevinswiber](https://github.com/kevinswiber)
- hold every windowed quantity the loop suspicion test reads - ([0ef91c1](https://github.com/kevinswiber/ratto/commit/0ef91c1b887321ef87f5f9504aaa45db268750dc)) - [@kevinswiber](https://github.com/kevinswiber)
- observe whether a watched path changes while the dashboard is idle - ([829c080](https://github.com/kevinswiber/ratto/commit/829c08090c0172e6675577cdfedbf975c158afc1)) - [@kevinswiber](https://github.com/kevinswiber)
- replace the layout block with the tree that declares the panes - ([7c8a0ad](https://github.com/kevinswiber/ratto/commit/7c8a0ad03edd039451ea8c7dfa93fcf1b6eb9473)) - [@kevinswiber](https://github.com/kevinswiber)
- declare a pane inside the row or column that places it - ([1af3eb8](https://github.com/kevinswiber/ratto/commit/1af3eb8b12dd5c1bce1b1ca97b1d8ede8cc14c01)) - [@kevinswiber](https://github.com/kevinswiber)
- let a pane's scalar keys be written as properties - ([410e773](https://github.com/kevinswiber/ratto/commit/410e77324d13de606cc4b7d8a2786e1aceb56fac)) - [@kevinswiber](https://github.com/kevinswiber)
- settle the dashboard declaration format on KDL - ([d3c0553](https://github.com/kevinswiber/ratto/commit/d3c055396d7c6481ebbc187d52bb92e7c0c50169)) - [@kevinswiber](https://github.com/kevinswiber)
- nest rows and columns in the layout grammar - ([9c1c68d](https://github.com/kevinswiber/ratto/commit/9c1c68d02a7e565acd10ee36e0d77d568150132d)) - [@kevinswiber](https://github.com/kevinswiber)
- let piped frames size themselves from the handed-down geometry - ([de59190](https://github.com/kevinswiber/ratto/commit/de59190f2a72ee5bcbff2c7df8a45a67866f4e79)) - [@kevinswiber](https://github.com/kevinswiber)
- keep a pane's change marks alive at its own cadence - ([ad2768c](https://github.com/kevinswiber/ratto/commit/ad2768c39dbbbc7cf37a58f62a8fb8c0b4c3aafe)) - [@kevinswiber](https://github.com/kevinswiber)
- reflow a dashboard on resize and respawn every pane once it settles - ([328e7a3](https://github.com/kevinswiber/ratto/commit/328e7a3e6ce8564d447e2ac127dd5d6eed704a5b)) - [@kevinswiber](https://github.com/kevinswiber)
- give every pane its own trigger runtime - ([0ef461d](https://github.com/kevinswiber/ratto/commit/0ef461d4b3932d49b0ecd49c689e9c7c3569a74e)) - [@kevinswiber](https://github.com/kevinswiber)
- fail a pane inside its own box with an exit badge - ([195f8cf](https://github.com/kevinswiber/ratto/commit/195f8cf1377075d8620b01e128faedb12bcb18f9)) - [@kevinswiber](https://github.com/kevinswiber)
- add the rat dashboard subcommand and the composed-panes frame - ([8a8b1eb](https://github.com/kevinswiber/ratto/commit/8a8b1ebaab1bde88edd8b8468b5081edea88645b)) - [@kevinswiber](https://github.com/kevinswiber)
- add the TOML and KDL dashboard constructors - ([58b903e](https://github.com/kevinswiber/ratto/commit/58b903e879c23630ec01d3f46bc6b0077430eb0e)) - [@kevinswiber](https://github.com/kevinswiber)
- add the format-agnostic dashboard declaration and its one validation path - ([5d0d9b1](https://github.com/kevinswiber/ratto/commit/5d0d9b1ada368420e6359d9de004636b253dbe6f)) - [@kevinswiber](https://github.com/kevinswiber)
- make every gesture whole-dashboard and free the key reference - ([766fc7e](https://github.com/kevinswiber/ratto/commit/766fc7ecab4dd5b9dde95b0bfdbed6a719660023)) - [@kevinswiber](https://github.com/kevinswiber)
- drive the watch loop from a source registry with a drain - ([9970574](https://github.com/kevinswiber/ratto/commit/99705742e688f9332699b3c4adde1a927524e84f)) - [@kevinswiber](https://github.com/kevinswiber)
- tag every tick outcome with its source and exit status - ([b3d170d](https://github.com/kevinswiber/ratto/commit/b3d170d4bd66e807e79638172ad021062b072fbc)) - [@kevinswiber](https://github.com/kevinswiber)
- compose pane blocks and their marks in one layout walk - ([9babf50](https://github.com/kevinswiber/ratto/commit/9babf5004c1415ade91c5c14ae14f50c00e36242)) - [@kevinswiber](https://github.com/kevinswiber)
- pin a pane's output into its declared box - ([cbdcf14](https://github.com/kevinswiber/ratto/commit/cbdcf14b57eefcd819b8c5f517c990d05ffc28c4)) - [@kevinswiber](https://github.com/kevinswiber)
- add the pure source/pane registry with validation and geometry - ([cecb3f5](https://github.com/kevinswiber/ratto/commit/cecb3f54647e0b5005a9e73b3744ba877df2a08c)) - [@kevinswiber](https://github.com/kevinswiber)
#### Bug Fixes
- crediting requires coverage proved past the end, not up to it - ([8846daf](https://github.com/kevinswiber/ratto/commit/8846daff655ec40ff6011801f2ee75a7ee74483a)) - [@kevinswiber](https://github.com/kevinswiber)
- a tie at a bracket edge proves nothing, so it must not veto - ([c975e6c](https://github.com/kevinswiber/ratto/commit/c975e6c4ba565488cce56dba6bacf94f60d29f1b)) - [@kevinswiber](https://github.com/kevinswiber)
- a still-open bracket covers a zero-width interval - ([9780624](https://github.com/kevinswiber/ratto/commit/97806244b28f24d25603eb7c4c9984587f24702a)) - [@kevinswiber](https://github.com/kevinswiber)
- keep every arrival in the credit rule's denominator - ([89970a4](https://github.com/kevinswiber/ratto/commit/89970a4e507c0c0ef777af8b684a7ed9cbd9b8dc)) - [@kevinswiber](https://github.com/kevinswiber)
- say "I could not tell" instead of "there is no loop" - ([f7661f6](https://github.com/kevinswiber/ratto/commit/f7661f68a8141bb42f0bda11b77ec32d071e6976)) - [@kevinswiber](https://github.com/kevinswiber)
- stop reading an unplaceable arrival as proof of an outside writer - ([29b25b0](https://github.com/kevinswiber/ratto/commit/29b25b00a38865659ecbaecde84aff00239f4aca)) - [@kevinswiber](https://github.com/kevinswiber)
- stop accusing a producer and a consumer that merely run together - ([7d51a26](https://github.com/kevinswiber/ratto/commit/7d51a26858efe04adedb415a7471a2c21fe33d4e)) - [@kevinswiber](https://github.com/kevinswiber)
- keep the graph matrix inside the test timeout, and actually de-phase it - ([7e90ad4](https://github.com/kevinswiber/ratto/commit/7e90ad40d92ab2dbc668a1dd83692f8db8191203)) - [@kevinswiber](https://github.com/kevinswiber)
- collect trigger evidence for a dashboard whose triggers are all readers - ([6cdb73a](https://github.com/kevinswiber/ratto/commit/6cdb73ac123c80245e482eac3e80f2c841340c3b)) - [@kevinswiber](https://github.com/kevinswiber)
- open a bracket before its child, and close it when the child exits - ([78be6d3](https://github.com/kevinswiber/ratto/commit/78be6d38c60bdba5ccee1944339e1d6da30561fe)) - [@kevinswiber](https://github.com/kevinswiber)
- keep a still-running child in a change's coverage - ([f106938](https://github.com/kevinswiber/ratto/commit/f1069380eac2306fdfaa6d82022b685b8c4c36c4)) - [@kevinswiber](https://github.com/kevinswiber)
- credit a path's change to every child that could have written it - ([be31b4d](https://github.com/kevinswiber/ratto/commit/be31b4d96e3c60b95f486274274b42cde03d74fa)) - [@kevinswiber](https://github.com/kevinswiber)
- refuse an empty block where a key holds no block - ([4e5b33a](https://github.com/kevinswiber/ratto/commit/4e5b33a793873342e7fc3f133d629fa9dba3996e)) - [@kevinswiber](https://github.com/kevinswiber)
- refuse a type annotation on a container or a pane's name - ([35d4d4d](https://github.com/kevinswiber/ratto/commit/35d4d4db10e66ad6851427441ea38f42415576ef)) - [@kevinswiber](https://github.com/kevinswiber)
- refuse the tokens a key node itself has no room for - ([cd45680](https://github.com/kevinswiber/ratto/commit/cd4568060da2692df620e244b1bfae767bd0a419)) - [@kevinswiber](https://github.com/kevinswiber)
- locate every structural mistake in a dashboard's tree - ([32bd5b7](https://github.com/kevinswiber/ratto/commit/32bd5b738380dd1530b576dd20c5c09420dfac52)) - [@kevinswiber](https://github.com/kevinswiber)
- refuse every dashboard token that has no effect - ([3e78b83](https://github.com/kevinswiber/ratto/commit/3e78b832ff2e0665d1a5444d5f5f1f1c93c20c32)) - [@kevinswiber](https://github.com/kevinswiber)
- close the first review round's three findings - ([a948172](https://github.com/kevinswiber/ratto/commit/a9481726bf6e8f1599a07c233639cd19685d8e90)) - [@kevinswiber](https://github.com/kevinswiber)
- allow the ended-notice helper to be unix-only on the check leg - ([6639741](https://github.com/kevinswiber/ratto/commit/66397419039328adaf1406e2647c8b0c172fc511)) - [@kevinswiber](https://github.com/kevinswiber)
#### Documentation
- say what the absence of the looping badge does not mean - ([8f13b35](https://github.com/kevinswiber/ratto/commit/8f13b35e408d68a15c552ef58871fa7032a20dd5)) - [@kevinswiber](https://github.com/kevinswiber)
- bring the prose in line with what the code can prove - ([e16a1da](https://github.com/kevinswiber/ratto/commit/e16a1da93098bcde2865f2c4b505624b28d17660)) - [@kevinswiber](https://github.com/kevinswiber)
- state the reader's measured failure mode, not the predicted one - ([c2372ca](https://github.com/kevinswiber/ratto/commit/c2372cac92e7b993c6bb9dd74200df051fb48258)) - [@kevinswiber](https://github.com/kevinswiber)
- say what a flooding command costs and what survives it - ([635d052](https://github.com/kevinswiber/ratto/commit/635d0520f376d679163a5ad9da6b1305b84548eb)) - [@kevinswiber](https://github.com/kevinswiber)
- say what the looping report is, and what it is not - ([2611643](https://github.com/kevinswiber/ratto/commit/26116439f8401673b41ced59d6869eb6407f2462)) - [@kevinswiber](https://github.com/kevinswiber)
- warn that a pane's side effect on a watched path is a loop - ([e2939cd](https://github.com/kevinswiber/ratto/commit/e2939cd639ec35eb6a2acb9f8604ea10f4857a02)) - [@kevinswiber](https://github.com/kevinswiber)
- say plainly that a pane's command runs more often than its interval - ([659cadc](https://github.com/kevinswiber/ratto/commit/659cadce41ab895d47665ef37b0191e0475cdf6a)) - [@kevinswiber](https://github.com/kevinswiber)
- point at raw strings for commands that carry backslashes - ([aec4a7d](https://github.com/kevinswiber/ratto/commit/aec4a7d9235681e6dac3ff91452a035dcfb31f73)) - [@kevinswiber](https://github.com/kevinswiber)
- state the rule a dashboard's two spellings follow - ([864e3dd](https://github.com/kevinswiber/ratto/commit/864e3dd740154592b3c761769ad7f23cc75748ba)) - [@kevinswiber](https://github.com/kevinswiber)
- write every dashboard in the spelling that places its panes - ([97d0806](https://github.com/kevinswiber/ratto/commit/97d0806179f44ed0796febf8bd62f21d1e26b255)) - [@kevinswiber](https://github.com/kevinswiber)
- show nested layouts and a dashboard-in-a-dashboard together - ([54ca0ed](https://github.com/kevinswiber/ratto/commit/54ca0ed65147abb5e05c8550c261cb4a19e391ba)) - [@kevinswiber](https://github.com/kevinswiber)
- document rat dashboard and ship runnable pane declarations - ([ad82781](https://github.com/kevinswiber/ratto/commit/ad827816d46e1ee6446eed643add76978f6725e7)) - [@kevinswiber](https://github.com/kevinswiber)
#### Refactoring
- drop the measurement buffer, keep the trace - ([a1c6389](https://github.com/kevinswiber/ratto/commit/a1c6389924bd92331114afb4c9a0f81170d23321)) - [@kevinswiber](https://github.com/kevinswiber)
- dispatch pane keys from one table - ([9cd24d3](https://github.com/kevinswiber/ratto/commit/9cd24d3a064144128bdadb3d8c912ab500ffbc7a)) - [@kevinswiber](https://github.com/kevinswiber)

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