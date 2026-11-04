import { EOL } from "node:os"
import { Effect } from "effect"
import { Commands } from "../../commands"
import { Runtime } from "../../../framework/runtime"
import { accountCall, reportErrors } from "./shared"

export default Runtime.handler(
  Commands.commands.devices.commands.revoke,
  Effect.fn("cli.anysola.devices.revoke")(
    function* (input) {
      yield* accountCall("POST", `/v1/code/tokens/${encodeURIComponent(input.id)}/revoke`)
      process.stdout.write(`Device ${input.id} is signed out.` + EOL)
    },
    (effect) => effect.pipe(reportErrors),
  ),
)
