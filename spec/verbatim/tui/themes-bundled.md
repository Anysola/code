# Bundled themes, theme format and mode detection (0.5.1)

Paths are relative to `/home/ubuntu/anysola-code/build/fork/`.

## What `/themes` lists

Dialog: `packages/tui/src/component/dialog-theme-list.tsx:7-55`. Options = `Object.keys(themes.all())` sorted with `localeCompare(..., { sensitivity: "base" })`; the option title is the raw key (no display names). Title `Themes`, search in title row, placeholder `Search themes`.

With no custom theme files and the `new-designs` experiment off, exactly these 35 entries, in this order:

```
anysola
aura
ayu
carbonfox
catppuccin
catppuccin-frappe
catppuccin-macchiato
cobalt2
cursor
dracula
everforest
flexoki
github
gruvbox
kanagawa
lucent-orng
material
matrix
mercury
monokai
nightowl
nord
one-dark
opencode
orng
osaka-jade
palenight
rosepine
solarized
synthwave84
system
tokyonight
vercel
vesper
zenburn
```

Origins (`packages/tui/src/theme/index.ts:50-62`, merge priority low to high: defaults < custom files < generated system):

| name(s) | format | file |
|---|---|---|
| 32 legacy themes (all except anysola, opencode, system) | v1 (`defs` + `theme`) | `packages/tui/src/theme/assets/<name>.json`, registered in `packages/tui/src/theme/v1.ts:37-70` (`DEFAULT_THEMES`) |
| `opencode` | v2 | `packages/tui/src/theme/assets/v2/opencode.json` (`index.ts:10,27-32,54`) |
| `anysola` (default) | v2 | `packages/tui/src/theme/assets/v2/anysola.json` (`index.ts:12,34-39,55`); full tokens in `theme-anysola.md` |
| `system` | generated v1 from the terminal palette | `packages/tui/src/theme/system.ts:11-99`; until a palette is known it is an alias of the `anysola` document (`index.ts:60`) |
| `opencode2` | v2 | `packages/tui/src/theme/assets/v2/opencode2.json`; listed first-merged only while `experimental["new-designs"] === true` in the TUI config (`packages/tui/src/context/theme.tsx:291-294`, `packages/tui/src/config/index.tsx:390-394`) |
| any custom name | v1 or v2 | `<dir>/themes/<name>.json`, see below; a custom file can override a bundled name (including `anysola`) |

Not bundled: `packages/tui/src/theme/assets/opencode.json` (v1 opencode) is never imported (only named in a comment at `packages/tui/src/component/error-component.tsx:17`).

The legacy names in `DEFAULT_THEMES` key order (source order, used for `allThemes()` iteration): aura, ayu, catppuccin, catppuccin-frappe, catppuccin-macchiato, cobalt2, cursor, dracula, everforest, flexoki, github, gruvbox, kanagawa, material, matrix, mercury, monokai, nightowl, nord, one-dark, osaka-jade, orng, lucent-orng, palenight, rosepine, solarized, synthwave84, tokyonight, vesper, vercel, zenburn, carbonfox.

## Default theme and selection

- Initial active name = TUI config `theme.name` ?? `"anysola"` (`packages/tui/src/context/theme.tsx:113-118,139`). Settings dialog default also `"anysola"` (`packages/tui/src/component/dialog-config.tsx:29-38`).
- If the active name is not in the available set, `anysola` is used (`theme.tsx:300`).
- If loading a theme throws: the error is emitted (toast, below) and the active theme becomes `opencode` (`theme.tsx:301-313`); if `opencode` itself throws, the TUI crashes into the error screen.
- If custom-theme discovery fails, active is reset to `anysola` (`theme.tsx:160-167`).
- Error toast: variant `error`, title `Failed to load theme: <name>`, message = error message (`packages/tui/src/component/theme-error-toast.tsx:5-19`). Decode errors read `Invalid theme: <name> <value> is an invalid value` (`packages/theme/src/tui/resolve.ts:39-43`).
- Theme choice in the dialog: moving the highlight applies (and persists to config) the hovered theme immediately (`dialog-theme-list.tsx:32-34`); typing a filter previews the first match, an empty filter restores the initial theme (`:44-52`); Enter confirms; closing without confirming restores the initial theme (`:20-22`). `themes.set` writes `theme.name` to the TUI config on every call (`theme.tsx:345-355`).
- Renderer background color = resolved `background.base` (`theme.tsx:322`).

## Mode (dark/light)

- Config `theme.mode`: `"system" | "dark" | "light"` (`packages/tui/src/config/index.tsx:70-77`). `dark`/`light` lock the mode; `system` (or unset) follows the terminal.
- Startup mode (`packages/tui/src/app.tsx:313`): terminal handoff mode ?? `renderer.waitForThemeMode(1000)` ?? `"dark"`. Then lock ?? renderer.themeMode ?? that value (`theme.tsx:133-142`).
- Terminal detection (OpenTUI 0.8.0, `node_modules/.bun/@opentui+core@0.8.0+eff4d38d6a4a2cd9/node_modules/@opentui/core/chunk-bun-6kxdhkdc.js:20688-20800`): the native session queries OSC 10/11 (`query-theme-colors`); a reply `ESC ] 11 ; rgb:RRRR/GGGG/BBBB` (or `#rrggbb`) terminated by BEL or ST sets the mode: brightness `(r*299 + g*587 + b*114)/1000 > 128` (0..255 ints) → `light`, else `dark`. Components with more than 2 hex digits are scaled to 8 bits. No reply within 1000 ms → `dark`.
- Live changes: DEC private mode 2031 notifications `CSI ? 997 ; 1 n` / `CSI ? 997 ; 2 n` trigger a fresh OSC 10/11 query (OpenTUI) and a palette refresh for the `system` theme (`theme.tsx:263-268`). A changed mode emits `theme_mode`; when not locked the TUI applies it (`theme.tsx:257-261`).
- If the chosen theme lacks the requested mode, its first available mode is used (order `light`, `dark`) (`theme.tsx:400-405`, `packages/theme/src/tui/select.ts:8-21`).
- Commands (`packages/tui/src/app.tsx:1106-1135`):
  - `theme.switch` "Switch theme", slash `/themes`, opens the dialog.
  - `theme.switch_mode` title "Switch to light mode" when dark, "Switch to dark mode" when light; enabled only if the theme supports the other mode; pins (locks) the new mode and persists `theme.mode` (`theme.tsx:235-244,340-344`). Not in the palette (`palette: undefined`), default key none.
  - `theme.mode.lock` title "Unlock theme mode" / "Lock theme mode"; lock persists the current mode, unlock persists `theme.mode = "system"` and re-applies the terminal mode (`theme.tsx:246-255`). Not in palette, key none.
- Settings dialog row "Color mode", category Appearance, description "Dark or light variant, or follow the system", values `system dark light`, default `system` (`dialog-config.tsx:40-48`).

## `system` theme (generated from the terminal palette)

`packages/tui/src/theme/system.ts`, wiring `packages/tui/src/context/theme.tsx:131,177-220`:
- Palette = OpenTUI `renderer.getPalette({ size: 16 })` (OSC 4 colors 0..15 plus default fg/bg), queried only while the active theme is `system`, after first paint. Cached in TUI storage key `system-theme` (`colors`), so the next start paints with the cached palette before probing. Usable only if a background (default bg or palette[0]) and a foreground (default fg or palette[7]) exist.
- Mode from palette: `0.299 r + 0.587 g + 0.114 b > 0.5` (0..1 floats) → light (`system.ts:4-9`).
- Generated v1 theme (`system.ts:11-99`): primary/accent/info = ANSI cyan(6), secondary = magenta(5), error = red(1), warning = yellow(3), success = green(2); text = default fg; background = default bg with alpha 0 (transparent: the terminal's own background shows); backgroundPanel = gray[2], backgroundElement/backgroundMenu = gray[3], borderSubtle gray[6], border gray[7], borderActive gray[8]; diff colors tinted 22% (dark) or 14% (light) of green/red over bg; markdown/syntax mapped to ANSI hues (see file). Gray scale (`system.ts:101-133`): 12 steps; near-black dark bg (luminance < 10) uses `floor(i/12*0.4*255)` grays, near-white light bg (> 245) the mirror, otherwise scales bg toward white (dark) or black (light) by up to 40%. Muted text (`system.ts:135-144`): dark → 180 if luminance < 10 else `min(floor(160 + lum*0.3), 200)`; light → 75 if luminance > 245 else `max(floor(100 - (255-lum)*0.2), 60)`.
- Missing palette entries fall back to the xterm 256-color table (`packages/tui/src/theme/color.ts:3-41`).
- The generated v1 theme is migrated to v2 like any v1 theme (next section).

## Theme JSON format

### v2 (`base` + `light`/`dark`) — `packages/theme/src/tui/schema.ts`

```
{
  "$schema": string?,                 // ignored
  "base":  BaseThemeDefinition,       // required
  "light": ModeDefinition?,           // at least one of light/dark is required
  "dark":  ModeDefinition?
}
BaseThemeDefinition = {
  categorical: [hueName, ...] (min 1),
  text: { base, muted, faint?, action: {primary, secondary, destructive}, formfield, feedback: {error, warning, success, info: {base, muted?}} },
  background: { base, raised: {base, high, max}, action: {...}, formfield, feedback: {error|warning|success|info: {base}} },
  border: {base}, scrollbar: {base},
  diff: { text: {added, removed, context, hunkHeader}, background: {added, removed, context},
          highlight: {added, removed}, lineNumber: {text, background: {added, removed}} },
  syntax:   {comment, keyword, function, variable, string, number, type, operator, punctuation},
  markdown: {text, heading, link, linkText, code, blockQuote, emphasis, strong, horizontalRule, listItem,
             listEnumeration, image, imageText, codeBlock},
  "@dialog"?: tokens subset + "@footer"?: tokens subset
}
ModeDefinition = { hue: { <name>: {100..900 hex} | "$hue.<name>" , accent, interactive, neutral required },
                   categorical?, any token subset, "@dialog"? }
stateful color = { base, $hovered?, $focused?, $pressed?, $selected?, $disabled? }
color value    = "#rgb" | "#rgba" | "#rrggbb" | "#rrggbbaa" | "transparent" | "$<path>"
syntax/markdown values must be hex or "$hue.<name>.<step>"
```

Schema at `packages/theme/src/tui/schema.ts:1-310`; resolution rules in `theme-anysola.md` ("How a value resolves"). Every mode present is fully resolved at parse time, so a broken mode fails the whole theme (`packages/theme/src/tui/resolve.ts:45-53`).

### v1 (`defs` + `theme`) — `packages/theme/src/tui/v1.ts`

```
{ "$schema"?: string, "defs"?: { name: "#hex" | otherName },
  "theme": { <ThemeColor>: "#hex" | defName | themeKey | ansiNumber | {"dark": value, "light": value}, ...,
             "selectedListItemText"?, "backgroundMenu"?, "thinkingOpacity"?: number } }
ThemeColor = primary secondary accent error warning success info text textMuted selectedListItemText background
  backgroundPanel backgroundElement backgroundMenu border borderActive borderSubtle diffAdded diffRemoved diffContext
  diffHunkHeader diffHighlightAdded diffHighlightRemoved diffAddedBg diffRemovedBg diffContextBg diffLineNumber
  diffAddedLineNumberBg diffRemovedLineNumberBg markdownText markdownHeading markdownLink markdownLinkText markdownCode
  markdownBlockQuote markdownEmph markdownStrong markdownHorizontalRule markdownListItem markdownListEnumeration
  markdownImage markdownImageText markdownCodeBlock syntaxComment syntaxKeyword syntaxFunction syntaxVariable
  syntaxString syntaxNumber syntaxType syntaxOperator syntaxPunctuation
```

`"transparent"`/`"none"` = alpha 0; numbers are xterm-256 indexes; defaults: `selectedListItemText` = background, `backgroundMenu` = backgroundElement, `thinkingOpacity` = 0.6 (`packages/theme/src/tui/v1-migrate.ts:352-388`).

A source is a v1 theme when it has a `theme` key, otherwise v2 when it has `base` (`packages/tui/src/theme/index.ts:73-88`). v1 themes are converted at load time by `migrateV1` (`packages/theme/src/tui/v1-migrate.ts:52-74,86-227`), or `migrateV1NewDesigns` while the `new-designs` experiment is on (`packages/theme/src/tui/v1-migrate-new-designs.ts`):
- resolve the v1 theme for both modes; if both backgrounds are opaque and equal and both detect the same mode (text luminance > background luminance → dark), emit a single-mode document;
- infer chromatic hues (red orange yellow green cyan blue purple) for accent/success/warning/primary/error/info/secondary by OKLCH hue distance to reference swatches (light swatches when L ≥ 0.6), ignoring transparent or chroma < 0.03 colors; accent and primary always win their hue (`v1-migrate.ts:288-350`);
- each inferred hue gets a 9-step scale anchored at step 200 = the original color, interpolated in OKLCH toward L 0.97 (light) or 0.18 (dark), chroma reduced up to 50% (`v1-migrate.ts:398-415`); neutral (`gray`) scale is interpolated from background (800), backgroundPanel (700), backgroundElement (600), textMuted (400), text (200) (`v1-migrate.ts:417-465`);
- token mapping is fixed (`v1-migrate.ts:86-227`): text.base `$hue.neutral.200`, muted 400, background 800, raised 700/600/500, primary interactive.200, border = v1 border, scrollbar = v1 borderActive, feedback = v1 error/warning/success/info, diff/syntax/markdown 1:1; any literal that equals a hue step is rewritten to `$hue.<name>.<step>` (`referenceHues`, `v1-migrate.ts:239-286`); dialog background = backgroundPanel when opaque, else the first visible surface made opaque (`v1-migrate.ts:229-237`).
- The Rust port must reproduce this algorithm exactly (OKLCH math in `packages/theme/src/tui/color.ts`) for the 32 legacy themes and for `system` to look identical.

## Custom theme discovery

- Directories (`packages/tui/src/util/config-directories.ts:3-5`, used by `packages/tui/src/context/theme.tsx:68-76`): `<XDG_CONFIG_HOME or ~/.config>/opencode`, then `<ancestor>/.opencode` for every ancestor of the launch cwd from `/` down to cwd (deduplicated). The global config dir name is `opencode` (`packages/util/src/global.ts:12-13`, `packages/util/src/global-roots.ts:7-17`), not `anysola`.
- In each: `themes/*.json` (regular files or symlinks), sorted by path; theme name = file basename without `.json`; later (deeper) directories override earlier ones (`packages/tui/src/theme/discovery.ts:4-20`). A missing `themes` dir is fine; other read errors make discovery fail (active theme resets to `anysola`). A file whose JSON is not an object with `theme` or `base` is silently ignored (`index.ts:95-100`); a JSON syntax error fails discovery.
- Reload: `SIGUSR2` re-probes the palette at 250 ms and 1000 ms and rescans custom themes at 1000 ms (`theme.tsx:72-75,78,270-280`). Discovery runs once after mount; the first paint uses bundled themes only.

## Syntax highlighting style

`generateSyntax` (`packages/theme/src/tui/syntax.ts:4-86`) turns the resolved tokens into OpenTUI `SyntaxStyle` rules for tree-sitter capture names (and prompt extmarks). Full rule table with anysola values in `theme-anysola.md`.
