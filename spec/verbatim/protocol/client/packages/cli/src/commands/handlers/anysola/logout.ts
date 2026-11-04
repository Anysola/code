import { intro, log, outro } from "@clack/prompts"
import { Effect } from "effect"
import { Commands } from "../../commands"
import { Runtime } from "../../../framework/runtime"
import { handlePromptErrors } from "../../../ui/prompt"
import { createClient } from "../auth/shared"
import { activeToken, anysolaIntegration, APPS_URL, removeCredential, revoke } from "./account"

export default Runtime.handler(
  Commands.commands.logout,
  Effect.fn("cli.anysola.logout")(() =>
    Effect.gen(function* () {
      intro("Sign out of Anysola")
      const client = yield* createClient()
      const current = yield* activeToken(client, yield* anysolaIntegration(client))
      if (!current?.credentialID) {
        log.info(current ? "Signed in through ANYSOLA_TOKEN; unset it to sign out." : "Not signed in.")
        outro("Done")
        return
      }
      const revoked = yield* revoke(current.token)
      if (!revoked.ok) {
        log.error(`Could not revoke the token: ${revoked.message}`)
        log.info(`The token was kept. Try again, or revoke it at ${APPS_URL}.`)
        process.exitCode = 1
        return
      }
      yield* removeCredential(client, current.credentialID)
      outro("Signed out. The token is revoked and removed from this machine.")
    }).pipe(handlePromptErrors),
  ),
)
