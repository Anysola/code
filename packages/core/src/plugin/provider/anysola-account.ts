import { Effect, Option, Scope } from "effect"
import { define } from "@opencode/plugin/effect/plugin"
import type { SessionHttpResponse } from "@opencode/plugin/effect/session"
import { AnysolaRpc } from "@opencode/schema/anysola"
import { Anysola } from "../../anysola.js"
import { KV } from "../../kv.js"
import { cacheKey, call, errorOf } from "./anysola.js"
import { creditsStream } from "./anysola-credits-stream.js"
import { AnysolaSearchCredits } from "../websearch/anysola-credits.js"

// Anysola account features that live inside the binary: credits per turn and per session from the gateway's
// `x-credits-*` headers (or, on a stream, its closing `anysola_credits` event), the out-of-credits message, and proxies for /me, /usage and /tokens so clients (TUI, CLI)
// never handle the token themselves. Per-Session totals and budgets (`/budget`) are kept in KV so a resumed
// Session keeps counting against the same budget.

export const BILLING_URL = "https://account.anysola.com/billing"

type State = {
  remaining: number | null
  turn: number
  total: number
  turns: number
  open: boolean
  budget: number | null
}

const sessionKey = (sessionID: string) => `anysola:session:${sessionID}`
const defaultBudgetKey = "anysola:budget:default"

/** A budget is a positive whole number of credits; anything else means none. */
export const budgetOf = (value: unknown) =>
  typeof value === "number" && Number.isFinite(value) && value >= 1 ? Math.floor(value) : null
/**
 * One map for the process: the plugin runs once per location, and a budget set through one location's RPC must
 * be the one the Session's hooks (in another location) count against and save.
 */
const sessions = new Map<string, State>()

const count = (value: unknown) => (typeof value === "number" && Number.isFinite(value) && value > 0 ? value : 0)

const int = (value: string | null) => {
  if (value === null || value.trim() === "") return undefined
  const parsed = Number(value)
  return Number.isFinite(parsed) ? parsed : undefined
}

export const AnysolaAccountPlugin = define<KV.Service | Scope.Scope>({
  id: "anysola.account",
  effect: Effect.fn(function* (ctx) {
    const kv = yield* KV.Service
    const runFork = Effect.runForkWith(yield* Effect.context())

    const defaultBudget = () =>
      kv.get(defaultBudgetKey).pipe(
        Effect.map(budgetOf),
        Effect.orElseSucceed(() => null),
      )
    /** The Session's state: this process's copy, else the stored one, else a fresh one with the default budget. */
    const load = Effect.fn("AnysolaAccount.load")(function* (sessionID: string, create: boolean) {
      const known = sessions.get(sessionID)
      if (known) return known
      const stored = yield* kv.get(sessionKey(sessionID)).pipe(Effect.orElseSucceed(() => undefined))
      if (!stored && !create) return undefined
      const saved =
        stored && typeof stored === "object" && !Array.isArray(stored) ? (stored as Record<string, unknown>) : undefined
      const state: State = {
        remaining: null,
        turn: 0,
        total: count(saved?.total),
        turns: count(saved?.turns),
        open: false,
        budget: saved ? budgetOf(saved.budget) : yield* defaultBudget(),
      }
      // Another call may have loaded it while this one read KV.
      const raced = sessions.get(sessionID)
      if (raced) return raced
      sessions.set(sessionID, state)
      return state
    })
    const save = (sessionID: string, state: State) =>
      kv
        .set(sessionKey(sessionID), { total: state.total, turns: state.turns, budget: state.budget })
        .pipe(Effect.catchCause((cause) => Effect.logWarning("failed to save Anysola session credits", { cause })))

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
      budget: state.budget,
    })

    const rpc = yield* ctx.rpc
      .register(AnysolaRpc.Definition, {
        session: (input) =>
          load(input.sessionID, false).pipe(Effect.map((state) => (state ? snapshot(input.sessionID, state) : null))),
        budget: (input) =>
          Effect.gen(function* () {
            const state = (yield* load(input.sessionID, true))!
            state.budget = budgetOf(input.limit)
            yield* save(input.sessionID, state)
            // The caller gets the new state back; the `credits` event keeps reporting charges.
            return snapshot(input.sessionID, state)
          }),
        budgetDefault: (input) =>
          Effect.gen(function* () {
            if (input.limit === undefined) return { limit: yield* defaultBudget() }
            const limit = budgetOf(input.limit)
            yield* (limit === null ? kv.remove(defaultBudgetKey) : kv.set(defaultBudgetKey, limit)).pipe(Effect.ignore)
            return { limit }
          }),
        me: () => proxy("GET", "/v1/code/me"),
        usage: (input) => proxy("GET", `/v1/code/usage?days=${input.days === 30 ? 30 : 7}`),
        tokens: () => proxy("GET", "/v1/code/tokens"),
        revoke: (input) => proxy("POST", `/v1/code/tokens/${encodeURIComponent(input.id)}/revoke`),
        estimates: () =>
          kv.get(cacheKey).pipe(
            Effect.map((body) => estimates(body)),
            Effect.orElseSucceed(() => ({})),
          ),
        notes: () =>
          kv.get(cacheKey).pipe(
            Effect.map((body) => notes(body)),
            Effect.orElseSucceed(() => ({})),
          ),
      })
      .pipe(Effect.orDie)

    const publish = (sessionID: string, state: State) =>
      rpc.events
        .emit("credits", snapshot(sessionID, state))
        .pipe(Effect.catchCause((cause) => Effect.logWarning("failed to publish Anysola credits", { cause })))

    const record = Effect.fn("AnysolaAccount.record")(function* (
      sessionID: string,
      charged: number | undefined,
      remaining: number | undefined,
    ) {
      const state = (yield* load(sessionID, true))!
      if (!state.open) state.turn = 0
      state.open = true
      state.turn += charged ?? 0
      state.total += charged ?? 0
      if (remaining !== undefined) state.remaining = remaining
      if (charged) yield* save(sessionID, state)
      yield* publish(sessionID, state)
    })

    const onResponse = (event: SessionHttpResponse) =>
      Effect.gen(function* () {
        const headers = event.response.headers
        const charged = int(headers.get("x-credits-charged"))
        const remaining = int(headers.get("x-credits-remaining"))
        if (charged !== undefined || remaining !== undefined) yield* record(event.sessionID, charged, remaining)
        else if (event.response.ok && event.response.body && headers.get("content-type")?.includes("text/event-stream")) {
          // A stream's charge is known only at its end: the gateway appends it as an event.
          const sessionID = event.sessionID
          const before = int(headers.get("x-credits-available-before"))
          event.response = new Response(
            event.response.body.pipeThrough(
              creditsStream(
                (value) =>
                  void runFork(
                    record(sessionID, value, before === undefined ? undefined : Math.max(0, before - value)).pipe(
                      Effect.ignore,
                    ),
                  ),
              ),
            ),
            { status: event.response.status, statusText: event.response.statusText, headers },
          )
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

    // A web search is charged by its own request; its credits count toward the Session that ran the tool.
    yield* ctx.tool.hook("execute.after", (event) =>
      Effect.gen(function* () {
        if (event.tool !== "websearch" || event.status !== "completed") return
        const query = (event.input as { query?: unknown } | undefined)?.query
        const credits = typeof query === "string" ? AnysolaSearchCredits.take(query) : undefined
        if (credits) yield* record(event.sessionID, credits, undefined)
      }).pipe(Effect.catchCause((cause) => Effect.logWarning("failed to count Anysola search credits", { cause }))),
    )

    // A turn ends when the Session goes idle; the status line keeps showing that turn until the next one charges.
    yield* ctx.session.hook("experimental.idle", (event) =>
      Effect.gen(function* () {
        const state = sessions.get(event.sessionID)
        if (!state?.open) return
        state.open = false
        if (state.turn > 0) {
          state.turns += 1
          yield* save(event.sessionID, state)
        }
        yield* publish(event.sessionID, state)
      }),
    )
  }),
})

function notes(body: unknown): Record<string, string> {
  const providers = (body as { providers?: Record<string, { models?: Record<string, unknown> }> } | undefined)
    ?.providers
  const models = providers?.[Anysola.providerID]?.models ?? {}
  return Object.fromEntries(
    Object.entries(models).flatMap(([id, model]) => {
      const value = (model as { description?: unknown } | undefined)?.description
      return typeof value === "string" && value.trim() ? [[id, value.trim().slice(0, 160)]] : []
    }),
  )
}

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
