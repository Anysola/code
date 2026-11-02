import { spawnSync } from "node:child_process"
import { intro, log, outro } from "@clack/prompts"
import { Effect } from "effect"
import { Commands } from "../commands"
import { Runtime } from "../../framework/runtime"
import { AnysolaVersion } from "../../services/anysola-version"
import { handlePromptErrors } from "../../ui/prompt"
import { ANYSOLA_VERSION } from "../../version"

// Anysola Code: `anysola update` installs the latest @anysola/code from npm (`--print` only shows the command).
export default Runtime.handler(
  Commands.commands.update,
  Effect.fn("cli.update")(
    function* (input) {
      intro("Update Anysola Code")
      const info = yield* Effect.promise(() => AnysolaVersion.fetchInfo())
      const command = AnysolaVersion.UPDATE_COMMAND.join(" ")
      if (info) {
        log.info(`Installed ${ANYSOLA_VERSION}, latest ${info.latest}`)
        if (info.notes) log.message(info.notes)
        if (AnysolaVersion.released() && AnysolaVersion.compare(info.latest, ANYSOLA_VERSION) <= 0 && !input.force) {
          outro("Already up to date")
          return
        }
      }
      if (input.print) {
        log.message(command)
        outro("Run the command above to update")
        return
      }
      log.step(command)
      const [bin, ...args] = AnysolaVersion.UPDATE_COMMAND
      const result = spawnSync(bin, args, { stdio: "inherit", shell: process.platform === "win32" })
      if (result.error || result.status !== 0) {
        log.error(`npm failed${result.error ? `: ${result.error.message}` : ` with exit code ${result.status}`}`)
        log.info(`Run it yourself: ${command}`)
        process.exitCode = 1
        return
      }
      outro("Updated. Restart Anysola Code to use the new version.")
    },
    (effect) => handlePromptErrors(effect),
  ),
)
