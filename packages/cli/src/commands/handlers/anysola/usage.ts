import { EOL } from "node:os"
import { Effect } from "effect"
import { usageLines } from "@opencode/tui/anysola/format"
import { Commands } from "../../commands"
import { Runtime } from "../../../framework/runtime"
import { accountCall, reportErrors } from "./shared"

export default Runtime.handler(
  Commands.commands.usage,
  Effect.fn("cli.anysola.usage")(
    function* (input) {
      const body = yield* accountCall("GET", `/v1/code/usage?days=${input.days}`)
      if (input.format === "json") {
        process.stdout.write(JSON.stringify(body, null, 2) + EOL)
        return
      }
      process.stdout.write([`Usage, last ${input.days} days`, "", ...usageLines(body)].join(EOL) + EOL)
    },
    (effect) => effect.pipe(reportErrors),
  ),
)
