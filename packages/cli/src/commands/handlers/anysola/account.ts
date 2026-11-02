// Anysola account commands: `anysola login`, `anysola logout`, `anysola status`, and the sign-in that runs before the
// first TUI start. The device flow itself runs in the server (the `anysola` integration's OAuth method), so the
// token lands in the regular credential store; these commands drive it and talk to the account endpoints.
import { intro, log, outro } from "@clack/prompts"
import { Effect } from "effect"
import type { IntegrationInfo, OpenCodeClient } from "@opencode/client"
import { oauthLogin } from "../auth/login"
import { loadIntegrations, location, request, resolveIntegration } from "../auth/shared"

export const integrationID = "anysola"
const APPS_URL = "https://account.anysola.com/apps"

export function apiURL() {
  return (process.env.ANYSOLA_API_URL || "https://api.anysola.com").replace(/\/+$/, "")
}

export const anysolaIntegration = Effect.fn("cli.anysola.integration")(function* (client: OpenCodeClient) {
  return yield* resolveIntegration(yield* loadIntegrations(client), integrationID)
})

/** The saved credential behind the active Anysola connection, with its token. */
export const activeToken = Effect.fn("cli.anysola.token")(function* (
  client: OpenCodeClient,
  integration: IntegrationInfo,
) {
  const connection = integration.connections[0]
  if (!connection) return undefined
  if (connection.type === "env") return { connection, token: process.env[connection.name] ?? "", credentialID: undefined }
  const response: unknown = yield* request((signal) => client.credential.list({ signal }))
  const list = (Array.isArray(response) ? response : ((response as { data?: unknown[] }).data ?? [])) as Array<{
    id: string
    integrationID: string
    value: { type: string; access?: string; key?: string }
  }>
  const entry = list.find((item) => item.id === connection.id)
  const token = entry?.value.type === "oauth" ? entry.value.access : entry?.value.type === "key" ? entry.value.key : undefined
  return token ? { connection, token, credentialID: connection.id } : undefined
})

export function mask(token: string) {
  return `${token.slice(0, 8)}...${token.slice(-4)}`
}

type Reply = { ok: boolean; status: number; body: Record<string, unknown> }

export const call = (method: "GET" | "POST", route: string, token: string) =>
  Effect.tryPromise({
    try: async (): Promise<Reply> => {
      const response = await fetch(apiURL() + route, {
        method,
        headers: { accept: "application/json", authorization: `Bearer ${token}` },
        signal: AbortSignal.timeout(15_000),
      })
      const text = await response.text()
      let body: Record<string, unknown> = {}
      try {
        const parsed: unknown = text ? JSON.parse(text) : {}
        if (parsed && typeof parsed === "object") body = parsed as Record<string, unknown>
      } catch {}
      return { ok: response.ok, status: response.status, body }
    },
    catch: (cause) => new Error(`Cannot reach ${apiURL()}`, { cause }),
  })

/** Runs the device flow through the server and stores the token as the Anysola credential. */
export const signIn = Effect.fn("cli.anysola.sign-in")(function* (client: OpenCodeClient, integration: IntegrationInfo) {
  const method = integration.methods.find((item) => item.type === "oauth")
  if (!method || method.type !== "oauth") return yield* Effect.fail(new Error("Anysola login is not available"))
  yield* oauthLogin(client, integration, method)
})

/** Revokes a token on the server. A 401 means it is already dead, which counts as success. */
export const revoke = Effect.fn("cli.anysola.revoke")(function* (token: string) {
  return yield* call("POST", "/v1/code/tokens/self/revoke", token).pipe(
    Effect.map((reply) =>
      reply.ok || reply.status === 401
        ? { ok: true as const, message: "" }
        : { ok: false as const, message: `HTTP ${reply.status}` },
    ),
    Effect.catch((error: Error) => Effect.succeed({ ok: false as const, message: error.message })),
  )
})

export const removeCredential = Effect.fn("cli.anysola.remove")(function* (client: OpenCodeClient, credentialID: string) {
  yield* request((signal) => client.credential.remove({ credentialID }, { signal }))
})

/** Before the first TUI start: sign in when no Anysola connection exists and a terminal can show the code. */
export const ensureSignedIn = Effect.fn("cli.anysola.ensure")(function* (client: OpenCodeClient) {
  const integration = yield* anysolaIntegration(client)
  if (integration.connections.length > 0) return
  if (!process.stdin.isTTY || !process.stdout.isTTY) return
  intro("Welcome to Anysola Code")
  log.info("Sign in with your Anysola account to continue.")
  yield* signIn(client, integration)
  outro("Signed in")
})

export { APPS_URL, location }
