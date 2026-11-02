import { intro, log, outro } from "@clack/prompts"
import { Effect } from "effect"
import { Commands } from "../../commands"
import { Runtime } from "../../../framework/runtime"
import { handlePromptErrors } from "../../../ui/prompt"
import { createClient } from "../auth/shared"
import { activeToken, anysolaIntegration, APPS_URL, mask, removeCredential, revoke, signIn } from "./account"

export default Runtime.handler(
  Commands.commands.login,
  Effect.fn("cli.anysola.login")((input) =>
    Effect.gen(function* () {
      intro("Sign in to Anysola")
      const client = yield* createClient()
      const integration = yield* anysolaIntegration(client)
      const current = yield* activeToken(client, integration)
      if (current && !input.force) {
        log.info(`Already signed in (${mask(current.token)}). Use \`anysola login --force\` for a new token.`)
        outro("Done")
        return
      }
      if (current?.credentialID) {
        const revoked = yield* revoke(current.token)
        if (!revoked.ok) log.warn(`Could not revoke the old token (${revoked.message}); revoke it at ${APPS_URL}`)
        yield* removeCredential(client, current.credentialID)
      }
      yield* signIn(client, yield* anysolaIntegration(client))
      outro("Signed in")
    }).pipe(handlePromptErrors),
  ),
)
