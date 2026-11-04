import { EOL } from "node:os"
import { Effect } from "effect"
import { errorMessage } from "../../../util/error"
import { createClient } from "../auth/shared"
import { activeToken, anysolaIntegration, call } from "./account"

/** Calls an Anysola account endpoint with the signed-in token; prints the error and sets the exit code on failure. */
export const accountCall = Effect.fn("cli.anysola.call")(function* (method: "GET" | "POST", route: string) {
  const client = yield* createClient()
  const current = yield* activeToken(client, yield* anysolaIntegration(client))
  if (!current) return yield* Effect.fail(new Error("Not signed in. Run `anysola login`."))
  const reply = yield* call(method, route, current.token)
  if (reply.status === 401) return yield* Effect.fail(new Error("The token was revoked or expired. Run `anysola login --force`."))
  if (reply.status === 404) return yield* Effect.fail(new Error("This Anysola server does not offer this yet."))
  if (!reply.ok) {
    const error = reply.body.error as { message?: string } | undefined
    return yield* Effect.fail(new Error(error?.message ?? `Request failed (HTTP ${reply.status})`))
  }
  return reply.body
})

export const reportErrors = Effect.catch((error: unknown) =>
  Effect.sync(() => {
    process.stderr.write(errorMessage(error) + EOL)
    process.exitCode = 1
  }),
)
