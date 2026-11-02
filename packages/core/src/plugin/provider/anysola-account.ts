import { Effect, Option, Scope } from "effect"
import { define } from "@opencode/plugin/effect/plugin"
import type { SessionHttpResponse } from "@opencode/plugin/effect/session"
import { AnysolaRpc } from "@opencode/schema/anysola"
import { Anysola } from "../../anysola.js"
import { KV } from "../../kv.js"
import { cacheKey, call, errorOf } from "./anysola.js"

// Anysola account features that live inside the binary: credits per turn and per session from the gateway's
// `x-credits-*` headers, the out-of-credits message, and proxies for /me, /usage and /tokens so clients (TUI, CLI)
// never handle the token themselves.

export const BILLING_URL = "https://account.anysola.com/billing"

type State = { remaining: number | null; turn: number; total: number; turns: number; open: boolean }

const int = (value: string | null) => {
  if (value === null || value.trim() === "") return undefined
  const parsed = Number(value)
  return Number.isFinite(parsed) ? parsed : undefined
}

export const AnysolaAccountPlugin = define<KV.Service | Scope.Scope>({
  id: "anysola.account",
  effect: Effect.fn(function* (ctx) {
    const kv = yield* KV.Service
    const sessions = new Map<string, State>()

    const token = Effect.fn("AnysolaAccount.token")(function* () {
      const connection = yield* ctx.integration.connection.active(Anysola.integrationID)
      if (!connection) return undefined
      const credential = yield* ctx.integration.connection.resolve(connection).pipe(Effect.orElseSucceed(() => undefined))
      return credential?.type === "oauth" ? credential.access : credential?.type === "key" ? credential.key : undefined
    })
    const proxy = (method: "GET" | "POST", route: string) =>
      Effect.gen(function* () {
        const value = yield* token()
        if (!value)
          return { ok: false, status: 401, body: { error: { message: "Not signed in. Run `anysola login`." } } }
        const reply = yield* call(method, route, { token: value }).pipe(Effect.option)
        if (Option.isNone(reply))
          return { ok: false, status: 0, body: { error: { message: `Cannot reach ${Anysola.apiURL()}` } } }
        const { status, body } = reply.value
        return { ok: status >= 200 && status < 300, status, body: body as unknown }
      }).pipe(
        Effect.catchCause(() =>
          Effect.succeed({ ok: false, status: 0, body: { error: { message: "Anysola request failed" } } }),
        ),
      )

    const snapshot = (sessionID: string, state: State) => ({
      sessionID,
      remaining: state.remaining,
      turn: state.turn,
      total: state.total,
      turns: state.turns,
    })

    const rpc = yield* ctx.rpc
      .register(AnysolaRpc.Definition, {
        session: (input) =>
          Effect.succeed(sessions.has(input.sessionID) ? snapshot(input.sessionID, sessions.get(input.sessionID)!) : null),
        me: () => proxy("GET", "/v1/code/me"),
        usage: (input) => proxy("GET", `/v1/code/usage?days=${input.days === 30 ? 30 : 7}`),
        tokens: () => proxy("GET", "/v1/code/tokens"),
        revoke: (input) => proxy("POST", `/v1/code/tokens/${encodeURIComponent(input.id)}/revoke`),
        estimates: () =>
          kv.get(cacheKey).pipe(
            Effect.map((body) => estimates(body)),
            Effect.orElseSucceed(() => ({})),
          ),
      })
      .pipe(Effect.orDie)

    const publish = (sessionID: string, state: State) =>
      rpc.events
        .emit("credits", snapshot(sessionID, state))
        .pipe(Effect.catchCause((cause) => Effect.logWarning("failed to publish Anysola credits", { cause })))

    const onResponse = (event: SessionHttpResponse) =>
      Effect.gen(function* () {
        const headers = event.response.headers
        const charged = int(headers.get("x-credits-charged"))
        const remaining = int(headers.get("x-credits-remaining"))
        if (charged !== undefined || remaining !== undefined) {
          const state = sessions.get(event.sessionID) ?? { remaining: null, turn: 0, total: 0, turns: 0, open: false }
          if (!state.open) state.turn = 0
          state.open = true
          state.turn += charged ?? 0
          state.total += charged ?? 0
          if (remaining !== undefined) state.remaining = remaining
          sessions.set(event.sessionID, state)
          yield* publish(event.sessionID, state)
        }
        // Out of credits: the gateway's message names the renewal date; add where to top up.
        if (event.response.status === 402) {
          const text = yield* Effect.promise(() => event.response.clone().text().catch(() => ""))
          let message = "Out of Anysola credits."
          try {
            const parsed = errorOf({ status: 402, body: JSON.parse(text) })
            if (parsed.message && parsed.message !== "HTTP 402") message = parsed.message
          } catch {}
          void event.response.body?.cancel()
          event.response = Response.json(
            { type: "error", error: { type: "billing_error", message: `${message}\nUpgrade: ${BILLING_URL}` } },
            { status: 402 },
          )
        }
      })
    yield* ctx.session.hook("http.response", onResponse, { providerID: Anysola.providerID })

    // A turn ends when the Session goes idle; the status line keeps showing that turn until the next one charges.
    yield* ctx.session.hook("experimental.idle", (event) =>
      Effect.gen(function* () {
        const state = sessions.get(event.sessionID)
        if (!state?.open) return
        state.open = false
        if (state.turn > 0) state.turns += 1
        yield* publish(event.sessionID, state)
      }),
    )
  }),
})

function estimates(body: unknown): Record<string, number> {
  const providers = (body as { providers?: Record<string, { models?: Record<string, unknown> }> } | undefined)
    ?.providers
  const models = providers?.[Anysola.providerID]?.models ?? {}
  return Object.fromEntries(
    Object.entries(models).flatMap(([id, model]) => {
      const value = (model as { credits_per_turn_estimate?: unknown } | undefined)?.credits_per_turn_estimate
      return typeof value === "number" && Number.isFinite(value) && value > 0 ? [[id, value]] : []
    }),
  )
}
