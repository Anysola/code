// The ANYSOLA wordmark for the terminal: wide capitals, and both A's drawn as the brand symbol (two diagonal strokes,
// the right one detached below the apex, no crossbar). Quadrant blocks give the strokes half-cell steps.
// Cells are about twice as tall as wide, so three rows match the logo's proportions at 60 columns.
// Source: ~/anysola-logo/anysola-yazi-beyaz.svg, drawn by hand on a 120x6 subcell grid.
export const wordmark = [
  "   ▄█▜▖  ▐█▙▖  ▐█ ▝▜▙▖▗▟▛▘ ▄▛▀▀▀▀▘ ▗▟▀▀▀▀▜▄ ▐█         ▄█▜▖ ",
  " ▗▟▛▘▐█▖ ▐█▝█▄ ▐█   ▝██▘    ▀▀▀▀▜▙ ▐█    ▐█ ▐█       ▗▟▛▘▐█▖",
  "▗█▀   ▜█ ▐█  ▀▙▟█    ██    ▗▄▄▄▄▟▀ ▝▜▄▄▄▄▟▀ ▐█▄▄▄▄▖ ▗█▀   ▜█",
]

/** The A alone: the Anysola symbol, for terminals too narrow for the wordmark. */
export const symbol = ["   ▄█▜▖ ", " ▗▟▛▘▐█▖", "▗█▀   ▜█"]

/** Product name set under the mark, muted, as on the web ("ANYSOLA Cloud", "ANYSOLA Admin"). */
export const product = "Code"
