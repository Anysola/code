import { EOL } from "node:os"
import { Effect } from "effect"
import { formatDate } from "@opencode/tui/anysola/format"
import { Commands } from "../../commands"
import { Runtime } from "../../../framework/runtime"
import { accountCall, reportErrors } from "./shared"

const str = (value: unknown) => (typeof value === "string" ? value : "")

export default Runtime.handler(
  Commands.commands.devices,
  Effect.fn("cli.anysola.devices")(
    function* () {
      const body = yield* accountCall("GET", "/v1/code/tokens")
      const tokens = Array.isArray(body.tokens) ? (body.tokens as Array<Record<string, unknown>>) : []
      if (tokens.length === 0) {
        process.stdout.write("No devices are signed in." + EOL)
        return
      }
      const rows = tokens.map((token) => [
        `${token.current === true ? "* " : "  "}${str(token.id)}`,
        str(token.name) || "Unnamed device",
        str(token.client),
        formatDate(str(token.lastUsedAt)) ?? "never",
        formatDate(str(token.expiresAt)) ?? "",
      ])
      const all = [["  id", "name", "client", "last used", "expires"], ...rows]
      const widths = all[0]!.map((_, index) => Math.max(...all.map((row) => row[index]!.length)))
      const lines = all.map((row) => row.map((cell, index) => cell.padEnd(widths[index]!)).join("   ").trimEnd())
      process.stdout.write(lines.join(EOL) + EOL + EOL + "* this device. Sign one out with `anysola devices revoke <id>`." + EOL)
    },
    (effect) => effect.pipe(reportErrors),
  ),
)
