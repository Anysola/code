import { TextAttributes } from "@opentui/core"
import { For, Show } from "solid-js"
import { useTerminalDimensions } from "@opentui/solid"
import { useTheme } from "../context/theme"
import { product, symbol, wordmark } from "../anysola/wordmark"

// Anysola Code: the ANYSOLA wordmark with the product name under its right edge; the A symbol alone on narrow
// terminals, nothing on short ones.
export function Logo() {
  const theme = useTheme()
  const dimensions = useTerminalDimensions()
  const art = () => (dimensions().width < wordmark[0]!.length + 4 ? symbol : wordmark)

  return (
    <Show when={dimensions().height >= 12}>
      <box alignItems={art() === symbol ? "center" : "flex-end"}>
        <box>
          <For each={art()}>
            {(line) => (
              <text fg={theme.text.base} attributes={TextAttributes.BOLD} wrapMode="none" selectable={false}>
                {line}
              </text>
            )}
          </For>
        </box>
        <text fg={theme.text.muted} wrapMode="none" selectable={false}>
          {art() === symbol ? `Anysola ${product}` : product}
        </text>
      </box>
    </Show>
  )
}
