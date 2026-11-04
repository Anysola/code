# Anysola theme (`anysola`), every token

Source: `packages/tui/src/theme/assets/v2/anysola.json` (verbatim copy: `theme-anysola.json`). Registered in `packages/tui/src/theme/index.ts:12,36-39,55`; default name in `packages/tui/src/context/theme.tsx:116,139,166,300`.

The `$schema` field says `https://opencode.ai/theme.json` (unchanged upstream URL; only metadata).

## How a value resolves (method)

Re-implemented 1:1 in `/tmp/claude-1001/-home-ubuntu/8ca5656e-d1fe-58c1-b843-42c4ccb2771b/scratchpad/themes/resolve.py` from these sources; the tables below are that script's output.

1. **Mode pick** `packages/theme/src/tui/select.ts:8-21`: modes = `["light","dark"]` filtered to the keys present (anysola has both). Requested mode used if present, else the first present. Selected definition = deep `mergeTheme(base, <mode>)` (`packages/theme/src/tui/expand.ts:34-45`: records merge key by key, `undefined` skipped, scalars/arrays replace).
2. **Expansion** `packages/theme/src/tui/expand.ts:11-109`:
   - `text.muted` defaults to `$text.base`; `text.faint` defaults to `$text.muted` (expand.ts:51-52).
   - every action variant that has a `base` gets every missing state (`$disabled $pressed $focused $selected $hovered`) = `$<text|background>.action.<variant>.base` (expand.ts:90-109).
   - `formfield` with a `base` gets every missing state = `$<text|background>.formfield.base` (expand.ts:80-88).
   - `text.feedback.<kind>.muted` defaults to `$text.feedback.<kind>.base` (expand.ts:55-67).
   - `@dialog` (and its optional `@footer`) are expanded the same way (expand.ts:11-24).
3. **Hues** `packages/theme/src/tui/resolve.ts:201-236`: each hue is either a full 100..900 hex scale (all nine steps required, no extra steps) or an alias `"$hue.<other>"` (copied). `accent`, `interactive`, `neutral` are required (`packages/theme/src/tui/schema.ts:51-58`). Anysola: `accent = neutral = grafit`, `interactive = sand`.
4. **References** `packages/theme/src/tui/resolve.ts:238-288`: a string value is `"transparent"` (RGBA 0,0,0,0), a hex (`#rgb #rgba #rrggbb #rrggbbaa`), or `$<dot.path>`; `$hue.<name>.<step>` reads a hue step, any other path is read from the same view's own token tree (so `$background.base` inside the dialog view means the dialog's background). Circular references throw. Keys `$hovered` etc. become `hovered` etc. in the resolved tree.
5. **Views** `packages/theme/src/tui/resolve.ts:55-86`: the base view; a `dialog` surface = `contextualize(base, @dialog)` (deep merge, then per action variant and per state: `surface[$state] ?? surface.base ?? base[$state] ?? base.base`, resolve.ts:100-136); `dialog.footer` = `contextualize(dialog, @dialog.@footer)` or the dialog view when absent (anysola has no `@footer`).
6. **Stateful colors** (`packages/theme/src/tui/resolve.ts:170-175`): `.state(states)` returns the first true state in the order `disabled, pressed, focused, selected, hovered`, else `base`.
7. **Hue step shifting** (`packages/theme/src/tui/resolve.ts:177-199`): `theme.increase(color, n)` / `decrease(color, n)` move a color n steps along its own hue scale (clamped to 100..900). The lookup is by RGBA object identity (a WeakMap from each resolved hue-step object), not by value: a token that came from `$hue.<name>.<step>` shifts within `<name>`; a hex literal (e.g. `#eeba70`) or `transparent` is returned unchanged even if its value equals a hue step. Alias hues (`accent`, `neutral`, `interactive`) are clones, so `$hue.neutral.200` shifts along `neutral`. A Rust port needs a provenance tag (hue name + step) on resolved colors to reproduce this.
8. Renderer background = `background.base` (`packages/tui/src/context/theme.tsx:322`). Dark `#110f0d`, light `#fbfaf8`.

## Hue scales

### dark

| hue | 100 | 200 | 300 | 400 | 500 | 600 | 700 | 800 | 900 |
|---|---|---|---|---|---|---|---|---|---|
| grafit | #f8f6f4 | #e8e4df | #c9c3bc | #aba39b | #857f79 | #292622 | #1a1815 | #110f0d | #0a0907 |
| sand | #f3e6d4 | #dcc7ab | #c1ae93 | #a6967e | #897b67 | #6c6152 | #51493e | #3d372f | #2c2823 |
| red | #ecafaa | #e6857e | #c86f68 | #a85b56 | #864541 | #63322e | #42201d | #2b1210 | #180807 |
| orange | #f6d3b8 | #e9b082 | #cd986d | #af815c | #8f6849 | #6f5139 | #503b2a | #3a2a1e | #271d14 |
| gold | #f5e7c4 | #e0c88e | #c5af79 | #aa9667 | #8c7c53 | #6f6243 | #534934 | #3e3727 | #2d281e |
| green | #b9e0c0 | #88ca95 | #72af7f | #60956b | #4b7854 | #3a5c41 | #29412e | #1c2e20 | #121e14 |
| cyan | #b7ddde | #85c4c8 | #6faaae | #5d9193 | #497577 | #38595b | #283f40 | #1b2c2d | #121d1e |
| blue | #b2ceeb | #85b2e2 | #6f99c5 | #5c81a7 | #476687 | #354d65 | #243446 | #17232f | #0c151d |
| purple | #ccc0e4 | #b39fd9 | #9a87bd | #81719f | #665880 | #4c415f | #332c41 | #211c2b | #130f1a |
| accent | alias `$hue.grafit` | | | | | | | | |
| interactive | alias `$hue.sand` | | | | | | | | |
| neutral | alias `$hue.grafit` | | | | | | | | |

### light

| hue | 100 | 200 | 300 | 400 | 500 | 600 | 700 | 800 | 900 |
|---|---|---|---|---|---|---|---|---|---|
| grafit | #0f0d0b | #24211e | #4b4742 | #6e6862 | #908b86 | #e2dfdb | #efece9 | #fbfaf8 | #ffffff |
| sand | #573c1c | #795d3c | #8f7557 | #a68e73 | #bba791 | #d1c1b0 | #e1d5c9 | #ebe3da | #f2eee8 |
| red | #901e21 | #b94642 | #ce655f | #e3827b | #f3a29b | #ffc2bc | #ffdbd6 | #ffece9 | #fff5f2 |
| orange | #8d4a00 | #b36d2a | #c98850 | #dea372 | #f1bf97 | #ffdcbd | #fff3dd | #fffff2 | #fff8ee |
| gold | #7a5700 | #9e7a23 | #b4934b | #cbad6f | #e0c895 | #f6e3bc | #fff7dc | #fffff1 | #fffaee |
| green | #006129 | #33854a | #559b66 | #75b382 | #97c9a0 | #b9e0c0 | #d5f0d9 | #e8fbeb | #f2fef4 |
| cyan | #005f67 | #2e828b | #5299a1 | #73b1b8 | #97c8ce | #bae0e4 | #d7f1f4 | #eafdff | #f1fdfe |
| blue | #184f90 | #3d73b6 | #5b8bc9 | #78a4dd | #99bdec | #bad7fc | #d6eaff | #e8f7ff | #f2fbff |
| purple | #5b3d87 | #7d5fad | #9379c1 | #ab94d5 | #c2b0e6 | #daccf7 | #ece2ff | #f8f1ff | #fcf8ff |
| accent | alias `$hue.grafit` | | | | | | | | |
| interactive | alias `$hue.sand` | | | | | | | | |
| neutral | alias `$hue.grafit` | | | | | | | | |

Categorical order (base): `sand, blue, purple, green, red` (categorical[0] = sand is used for agent extmarks, categorical[1] = blue for skill extmarks, `packages/theme/src/tui/syntax.ts:14-15`).

## Base view tokens (after expansion), dark and light

`expr` is the expression after merge and expansion; the next column is the final color.

| token | dark expr | dark | light expr | light |
|---|---|---|---|---|
| `text.base` | `$hue.neutral.200` | #e8e4df | `$hue.neutral.200` | #24211e |
| `text.muted` | `$hue.neutral.400` | #aba39b | `$hue.neutral.400` | #6e6862 |
| `text.faint` | `$hue.neutral.500` | #857f79 | `$hue.neutral.500` | #908b86 |
| `text.action.primary.base` | `$text.base` | #e8e4df | `$text.base` | #24211e |
| `text.action.primary.disabled` | `$hue.neutral.500` | #857f79 | `$hue.neutral.500` | #908b86 |
| `text.action.primary.focused` | `$hue.neutral.800` | #110f0d | `$hue.neutral.900` | #ffffff |
| `text.action.primary.selected` | `$hue.interactive.200` | #dcc7ab | `$hue.interactive.200` | #795d3c |
| `text.action.primary.pressed` | `$text.action.primary.base` | #e8e4df | `$text.action.primary.base` | #24211e |
| `text.action.primary.hovered` | `$text.action.primary.base` | #e8e4df | `$text.action.primary.base` | #24211e |
| `text.action.secondary.base` | `$text.muted` | #aba39b | `$text.muted` | #6e6862 |
| `text.action.secondary.hovered` | `$text.base` | #e8e4df | `$text.base` | #24211e |
| `text.action.secondary.disabled` | `$text.action.secondary.base` | #aba39b | `$text.action.secondary.base` | #6e6862 |
| `text.action.secondary.pressed` | `$text.action.secondary.base` | #aba39b | `$text.action.secondary.base` | #6e6862 |
| `text.action.secondary.focused` | `$text.action.secondary.base` | #aba39b | `$text.action.secondary.base` | #6e6862 |
| `text.action.secondary.selected` | `$text.action.secondary.base` | #aba39b | `$text.action.secondary.base` | #6e6862 |
| `text.action.destructive.base` | `$hue.neutral.800` | #110f0d | `$hue.neutral.900` | #ffffff |
| `text.action.destructive.disabled` | `$hue.neutral.500` | #857f79 | `$hue.neutral.500` | #908b86 |
| `text.action.destructive.pressed` | `$text.action.destructive.base` | #110f0d | `$text.action.destructive.base` | #ffffff |
| `text.action.destructive.focused` | `$text.action.destructive.base` | #110f0d | `$text.action.destructive.base` | #ffffff |
| `text.action.destructive.selected` | `$text.action.destructive.base` | #110f0d | `$text.action.destructive.base` | #ffffff |
| `text.action.destructive.hovered` | `$text.action.destructive.base` | #110f0d | `$text.action.destructive.base` | #ffffff |
| `text.formfield.base` | `$hue.neutral.200` | #e8e4df | `$hue.neutral.200` | #24211e |
| `text.formfield.hovered` | `$hue.interactive.200` | #dcc7ab | `$hue.interactive.200` | #795d3c |
| `text.formfield.focused` | `$hue.interactive.200` | #dcc7ab | `$hue.interactive.200` | #795d3c |
| `text.formfield.pressed` | `$hue.interactive.200` | #dcc7ab | `$hue.interactive.200` | #795d3c |
| `text.formfield.disabled` | `$hue.neutral.500` | #857f79 | `$hue.neutral.500` | #908b86 |
| `text.formfield.selected` | `$hue.interactive.200` | #dcc7ab | `$hue.interactive.200` | #795d3c |
| `text.feedback.error.base` | `$hue.red.200` | #e6857e | `$hue.red.200` | #b94642 |
| `text.feedback.error.muted` | `$text.feedback.error.base` | #e6857e | `$text.feedback.error.base` | #b94642 |
| `text.feedback.warning.base` | `#eeba70` | #eeba70 | `#aa691b` | #aa691b |
| `text.feedback.warning.muted` | `$text.feedback.warning.base` | #eeba70 | `$text.feedback.warning.base` | #aa691b |
| `text.feedback.success.base` | `$hue.green.200` | #88ca95 | `$hue.green.200` | #33854a |
| `text.feedback.success.muted` | `$text.feedback.success.base` | #88ca95 | `$text.feedback.success.base` | #33854a |
| `text.feedback.info.base` | `$hue.blue.200` | #85b2e2 | `$hue.blue.200` | #3d73b6 |
| `text.feedback.info.muted` | `$text.feedback.info.base` | #85b2e2 | `$text.feedback.info.base` | #3d73b6 |
| `background.base` | `$hue.neutral.800` | #110f0d | `$hue.neutral.800` | #fbfaf8 |
| `background.raised.base` | `$hue.neutral.700` | #1a1815 | `$hue.neutral.700` | #efece9 |
| `background.raised.high` | `$hue.neutral.600` | #292622 | `$hue.neutral.600` | #e2dfdb |
| `background.raised.max` | `#33302b` | #33302b | `#d7d4cf` | #d7d4cf |
| `background.action.primary.base` | `transparent` | transparent | `transparent` | transparent |
| `background.action.primary.hovered` | `$hue.neutral.700` | #1a1815 | `$hue.neutral.700` | #efece9 |
| `background.action.primary.focused` | `$hue.interactive.200` | #dcc7ab | `$hue.interactive.200` | #795d3c |
| `background.action.primary.selected` | `transparent` | transparent | `transparent` | transparent |
| `background.action.primary.disabled` | `$background.action.primary.base` | transparent | `$background.action.primary.base` | transparent |
| `background.action.primary.pressed` | `$background.action.primary.base` | transparent | `$background.action.primary.base` | transparent |
| `background.action.secondary.base` | `transparent` | transparent | `transparent` | transparent |
| `background.action.secondary.disabled` | `$background.action.secondary.base` | transparent | `$background.action.secondary.base` | transparent |
| `background.action.secondary.pressed` | `$background.action.secondary.base` | transparent | `$background.action.secondary.base` | transparent |
| `background.action.secondary.focused` | `$background.action.secondary.base` | transparent | `$background.action.secondary.base` | transparent |
| `background.action.secondary.selected` | `$background.action.secondary.base` | transparent | `$background.action.secondary.base` | transparent |
| `background.action.secondary.hovered` | `$background.action.secondary.base` | transparent | `$background.action.secondary.base` | transparent |
| `background.action.destructive.base` | `$hue.red.200` | #e6857e | `$hue.red.200` | #b94642 |
| `background.action.destructive.disabled` | `$background.action.destructive.base` | #e6857e | `$background.action.destructive.base` | #b94642 |
| `background.action.destructive.pressed` | `$background.action.destructive.base` | #e6857e | `$background.action.destructive.base` | #b94642 |
| `background.action.destructive.focused` | `$background.action.destructive.base` | #e6857e | `$background.action.destructive.base` | #b94642 |
| `background.action.destructive.selected` | `$background.action.destructive.base` | #e6857e | `$background.action.destructive.base` | #b94642 |
| `background.action.destructive.hovered` | `$background.action.destructive.base` | #e6857e | `$background.action.destructive.base` | #b94642 |
| `background.formfield.base` | `$background.base` | #110f0d | `$background.base` | #fbfaf8 |
| `background.formfield.disabled` | `$background.formfield.base` | #110f0d | `$background.formfield.base` | #fbfaf8 |
| `background.formfield.pressed` | `$background.formfield.base` | #110f0d | `$background.formfield.base` | #fbfaf8 |
| `background.formfield.focused` | `$background.formfield.base` | #110f0d | `$background.formfield.base` | #fbfaf8 |
| `background.formfield.selected` | `$background.formfield.base` | #110f0d | `$background.formfield.base` | #fbfaf8 |
| `background.formfield.hovered` | `$background.formfield.base` | #110f0d | `$background.formfield.base` | #fbfaf8 |
| `background.feedback.error.base` | `$background.base` | #110f0d | `$background.base` | #fbfaf8 |
| `background.feedback.warning.base` | `$background.base` | #110f0d | `$background.base` | #fbfaf8 |
| `background.feedback.success.base` | `$background.base` | #110f0d | `$background.base` | #fbfaf8 |
| `background.feedback.info.base` | `$background.base` | #110f0d | `$background.base` | #fbfaf8 |
| `border.base` | `#2e2b27` | #2e2b27 | `#d4d0cc` | #d4d0cc |
| `scrollbar.base` | `#403c38` | #403c38 | `#bab7b2` | #bab7b2 |
| `diff.text.added` | `$hue.green.200` | #88ca95 | `$hue.green.200` | #33854a |
| `diff.text.removed` | `$hue.red.200` | #e6857e | `$hue.red.200` | #b94642 |
| `diff.text.context` | `$hue.neutral.400` | #aba39b | `$hue.neutral.400` | #6e6862 |
| `diff.text.hunkHeader` | `$hue.neutral.400` | #aba39b | `$hue.neutral.400` | #6e6862 |
| `diff.background.added` | `#112315` | #112315 | `#d6f0da` | #d6f0da |
| `diff.background.removed` | `#2f1614` | #2f1614 | `#ffdedb` | #ffdedb |
| `diff.background.context` | `$hue.neutral.700` | #1a1815 | `$hue.neutral.700` | #efece9 |
| `diff.highlight.added` | `$hue.green.100` | #b9e0c0 | `$hue.green.100` | #006129 |
| `diff.highlight.removed` | `$hue.red.100` | #ecafaa | `$hue.red.100` | #901e21 |
| `diff.lineNumber.text` | `$hue.neutral.500` | #857f79 | `$hue.neutral.500` | #908b86 |
| `diff.lineNumber.background.added` | `#0e1c11` | #0e1c11 | `#cae7cf` | #cae7cf |
| `diff.lineNumber.background.removed` | `#261210` | #261210 | `#fbd3cf` | #fbd3cf |
| `syntax.comment` | `$hue.neutral.500` | #857f79 | `$hue.neutral.500` | #908b86 |
| `syntax.keyword` | `$hue.purple.200` | #b39fd9 | `$hue.purple.200` | #7d5fad |
| `syntax.function` | `$hue.blue.200` | #85b2e2 | `$hue.blue.200` | #3d73b6 |
| `syntax.variable` | `$hue.red.200` | #e6857e | `$hue.red.200` | #b94642 |
| `syntax.string` | `$hue.green.200` | #88ca95 | `$hue.green.200` | #33854a |
| `syntax.number` | `$hue.orange.200` | #e9b082 | `$hue.orange.200` | #b36d2a |
| `syntax.type` | `$hue.gold.200` | #e0c88e | `$hue.gold.200` | #9e7a23 |
| `syntax.operator` | `$hue.cyan.200` | #85c4c8 | `$hue.cyan.200` | #2e828b |
| `syntax.punctuation` | `$hue.neutral.300` | #c9c3bc | `$hue.neutral.300` | #4b4742 |
| `markdown.text` | `$hue.neutral.200` | #e8e4df | `$hue.neutral.200` | #24211e |
| `markdown.heading` | `$hue.neutral.100` | #f8f6f4 | `$hue.neutral.100` | #0f0d0b |
| `markdown.link` | `$hue.interactive.200` | #dcc7ab | `$hue.interactive.200` | #795d3c |
| `markdown.linkText` | `$hue.blue.200` | #85b2e2 | `$hue.blue.200` | #3d73b6 |
| `markdown.code` | `$hue.green.200` | #88ca95 | `$hue.green.200` | #33854a |
| `markdown.blockQuote` | `$hue.neutral.300` | #c9c3bc | `$hue.neutral.300` | #4b4742 |
| `markdown.emphasis` | `$hue.neutral.100` | #f8f6f4 | `$hue.neutral.100` | #0f0d0b |
| `markdown.strong` | `$hue.neutral.100` | #f8f6f4 | `$hue.neutral.100` | #0f0d0b |
| `markdown.horizontalRule` | `$hue.neutral.500` | #857f79 | `$hue.neutral.500` | #908b86 |
| `markdown.listItem` | `$hue.neutral.400` | #aba39b | `$hue.neutral.400` | #6e6862 |
| `markdown.listEnumeration` | `$hue.neutral.400` | #aba39b | `$hue.neutral.400` | #6e6862 |
| `markdown.image` | `$hue.interactive.200` | #dcc7ab | `$hue.interactive.200` | #795d3c |
| `markdown.imageText` | `$hue.blue.200` | #85b2e2 | `$hue.blue.200` | #3d73b6 |
| `markdown.codeBlock` | `$hue.neutral.200` | #e8e4df | `$hue.neutral.200` | #24211e |

## Dialog surface (`@dialog`) tokens that differ from the base view

Used by every dialog (`ThemeContextProvider context="dialog"`); `dialog.footer` is identical to `dialog` in this theme.

| token | dark expr | dark | light expr | light |
|---|---|---|---|---|
| `background.base` | `$background.raised.base` | #1a1815 | `$background.raised.base` | #efece9 |
| `background.action.primary.hovered` | `$background.raised.high` | #292622 | `$background.raised.high` | #e2dfdb |
| `background.formfield.base` | `$background.base` | #1a1815 | `$background.base` | #efece9 |
| `background.formfield.disabled` | `$background.formfield.base` | #1a1815 | `$background.formfield.base` | #efece9 |
| `background.formfield.pressed` | `$background.formfield.base` | #1a1815 | `$background.formfield.base` | #efece9 |
| `background.formfield.focused` | `$background.formfield.base` | #1a1815 | `$background.formfield.base` | #efece9 |
| `background.formfield.selected` | `$background.formfield.base` | #1a1815 | `$background.formfield.base` | #efece9 |
| `background.formfield.hovered` | `$background.formfield.base` | #1a1815 | `$background.formfield.base` | #efece9 |
| `background.feedback.error.base` | `$background.base` | #1a1815 | `$background.base` | #efece9 |
| `background.feedback.warning.base` | `$background.base` | #1a1815 | `$background.base` | #efece9 |
| `background.feedback.success.base` | `$background.base` | #1a1815 | `$background.base` | #efece9 |
| `background.feedback.info.base` | `$background.base` | #1a1815 | `$background.base` | #efece9 |

## Syntax style rules (tree-sitter capture scopes)

From `packages/theme/src/tui/syntax.ts:4-86` (`generateSyntax`, fed to OpenTUI `SyntaxStyle.fromTheme`; used by code blocks, markdown, diffs and the prompt's extmarks). Rules listed in source order.

| scopes | token | style | dark | light |
|---|---|---|---|---|
| `default` | `text.base` |  | #e8e4df | #24211e |
| `prompt` | `hue.accent.200` |  | #e8e4df | #24211e |
| `extmark.file` | `text.feedback.warning.base` | bold | #eeba70 | #aa691b |
| `extmark.agent` | `categorical[0].200` | bold | #dcc7ab | #795d3c |
| `extmark.skill` | `categorical[1].200` | bold | #85b2e2 | #3d73b6 |
| `extmark.paste` | `text.action.primary.focused` | bold, bg=text.feedback.warning.base (#eeba70 / #aa691b) | #110f0d | #ffffff |
| `comment comment.documentation` | `syntax.comment` | italic | #857f79 | #908b86 |
| `string symbol character.special character` | `syntax.string` |  | #88ca95 | #33854a |
| `number boolean constant float` | `syntax.number` |  | #e9b082 | #b36d2a |
| `keyword.return keyword.conditional keyword.repeat keyword.coroutine` | `syntax.keyword` | italic | #b39fd9 | #7d5fad |
| `keyword.type` | `syntax.type` | bold, italic | #e0c88e | #9e7a23 |
| `keyword.function function.method` | `syntax.function` |  | #85b2e2 | #3d73b6 |
| `keyword` | `syntax.keyword` | italic | #b39fd9 | #7d5fad |
| `keyword.import string.escape string.regexp tag.attribute keyword.export` | `syntax.keyword` |  | #b39fd9 | #7d5fad |
| `operator keyword.operator punctuation.delimiter keyword.conditional.ternary` | `syntax.operator` |  | #85c4c8 | #2e828b |
| `variable variable.parameter function.method.call function.call property parameter field` | `syntax.variable` |  | #e6857e | #b94642 |
| `variable.member function constructor` | `syntax.function` |  | #85b2e2 | #3d73b6 |
| `type module class namespace` | `syntax.type` |  | #e0c88e | #9e7a23 |
| `type.definition` | `syntax.type` | bold | #e0c88e | #9e7a23 |
| `punctuation punctuation.bracket` | `syntax.punctuation` |  | #c9c3bc | #4b4742 |
| `variable.builtin type.builtin function.builtin module.builtin constant.builtin variable.super` | `text.feedback.error.base` |  | #e6857e | #b94642 |
| `keyword.directive keyword.modifier keyword.exception` | `syntax.keyword` | italic | #b39fd9 | #7d5fad |
| `punctuation.special tag.delimiter` | `syntax.operator` |  | #85c4c8 | #2e828b |
| `markup.heading markup.heading.2 markup.heading.3 markup.heading.4 markup.heading.5 markup.heading.6` | `markdown.heading` | bold | #f8f6f4 | #0f0d0b |
| `markup.heading.1` | `markdown.heading` | bold, underline | #f8f6f4 | #0f0d0b |
| `markup.bold markup.strong` | `markdown.strong` | bold | #f8f6f4 | #0f0d0b |
| `markup.italic` | `markdown.emphasis` | italic | #f8f6f4 | #0f0d0b |
| `markup.list` | `markdown.listItem` |  | #aba39b | #6e6862 |
| `markup.quote` | `markdown.blockQuote` | italic | #c9c3bc | #4b4742 |
| `markup.raw markup.raw.block` | `markdown.code` |  | #88ca95 | #33854a |
| `markup.raw.inline` | `markdown.code` | bg=background.base (#110f0d / #fbfaf8) | #88ca95 | #33854a |
| `markup.link markup.link.url string.special string.special.url` | `markdown.link` | underline | #dcc7ab | #795d3c |
| `markup.link.label` | `markdown.linkText` | underline | #85b2e2 | #3d73b6 |
| `label` | `markdown.linkText` |  | #85b2e2 | #3d73b6 |
| `spell nospell` | `text.base` |  | #e8e4df | #24211e |
| `markup.underline` | `text.base` | underline | #e8e4df | #24211e |
| `comment.error` | `text.feedback.error.base` | italic, bold | #e6857e | #b94642 |
| `comment.warning` | `text.feedback.warning.base` | italic, bold | #eeba70 | #aa691b |
| `comment.todo comment.note` | `text.feedback.info.base` | italic, bold | #85b2e2 | #3d73b6 |
| `attribute annotation` | `text.feedback.warning.base` |  | #eeba70 | #aa691b |
| `tag` | `text.feedback.error.base` |  | #e6857e | #b94642 |
| `markup.strikethrough markup.list.unchecked debug` | `text.muted` |  | #aba39b | #6e6862 |
| `markup.list.checked` | `text.feedback.success.base` |  | #88ca95 | #33854a |
| `diff.plus` | `diff.text.added` | bg=diff.background.added (#112315 / #d6f0da) | #88ca95 | #33854a |
| `diff.minus` | `diff.text.removed` | bg=diff.background.removed (#2f1614 / #ffdedb) | #e6857e | #b94642 |
| `diff.delta` | `diff.text.context` | bg=diff.background.context (#1a1815 / #efece9) | #aba39b | #6e6862 |
| `error` | `text.feedback.error.base` | bold | #e6857e | #b94642 |
| `warning` | `text.feedback.warning.base` | bold | #eeba70 | #aa691b |
| `info` | `text.feedback.info.base` |  | #85b2e2 | #3d73b6 |

