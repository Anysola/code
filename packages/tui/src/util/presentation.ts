import { product, wordmark } from "../anysola/wordmark"
import { exitSummary } from "../anysola/store"

const reset = "\x1b[0m"
const bold = "\x1b[1m"
const dim = "\x1b[90m"

// The ANYSOLA wordmark in the terminal's own foreground, with the product name muted under its right edge.
function mark(pad = "") {
  const width = wordmark[0]!.length
  return [...wordmark.map((line) => `${pad}${bold}${line.trimEnd()}${reset}`), `${pad}${" ".repeat(width - product.length)}${dim}${product}${reset}`]
}

export type SessionEpilogue = { readonly title: string; readonly sessionID?: string }

// Printed after the TUI closes.
export function epilogue(input: { readonly session?: SessionEpilogue; readonly updatedVersion?: string }) {
  if (!input.session && !input.updatedVersion) return
  const row = (label: string, value: string) => `  ${dim}${label.padEnd(10, " ")}${reset}${bold}${value}${reset}`
  return [
    ...mark("  "),
    "",
    ...(input.updatedVersion
      ? [`  ${dim}Run ${reset}${bold}anysola${reset}${dim} to use ${reset}${bold}${input.updatedVersion}${reset}`, ""]
      : []),
    ...(input.session
      ? [
          row("Session", input.session.title),
          ...(exitSummary(input.session.sessionID) ? [row("Spent", exitSummary(input.session.sessionID)!)] : []),
          row("Continue", `anysola -s ${input.session.sessionID}`),
          "",
        ]
      : []),
  ].join("\n")
}
