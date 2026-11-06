// Stand-in for the Anysola endpoints that are not deployed yet. Plain Node, no deps.
//
//   node scripts/fake-anysola.mjs          # PORT=8796 UPSTREAM=https://api.anysola.com
//
// /v1/code/messages and /config are proxied to the live gateway (real model answers, real token); the gateway's
// own `anysola_usage` stream event passes through. /me, /limits, /limits/redeem, /usage, /tokens and /version are
// synthetic, in the 1.0 usage-limit shapes (packages/contracts/src/code/client-api.ts).
//   USED=4              percent of the 5-hour window each /messages request counts here
//   START=42            starting 5-hour percent; WEEKLY=18 starting weekly percent
//   BANK=2              banked resets
//   EVENT="label"       /me.event label (empty: no event)
//   MINIMUM=0.2.0       /version minimum; LATEST=0.3.1
//   FORCE_LIMIT=1       /messages answers 429 for a full 5-hour window until a banked reset is spent
import http from "node:http"
import { Readable } from "node:stream"

const PORT = Number(process.env.PORT ?? 8796)
const UPSTREAM = (process.env.UPSTREAM ?? "https://api.anysola.com").replace(/\/+$/, "")
const USED = Number(process.env.USED ?? 4)
const HOUR = 3600e3
const state = {
  fiveHour: Number(process.env.START ?? 42),
  weekly: Number(process.env.WEEKLY ?? 18),
  bank: Number(process.env.BANK ?? 2),
  full: Boolean(process.env.FORCE_LIMIT),
  requests: 0,
  revoked: [],
}
const at = (ms) => new Date(Date.now() + ms).toISOString()
const limits = () => {
  const full = state.full || state.fiveHour >= 100
  return {
    plan: "team",
    allowed: !full,
    blockedBy: full ? "five_hour" : null,
    fiveHour: { percent: Math.min(100, Math.floor(state.fiveHour)), resetsAt: state.fiveHour > 0 || full ? at(2 * HOUR) : null },
    weekly: { percent: Math.min(100, Math.floor(state.weekly)), resetsAt: at(76 * HOUR) },
    bank: Array.from({ length: state.bank }, (_, i) => ({ id: `bank_${i + 1}`, expiresAt: at((i + 20) * 24 * HOUR), scope: "five_hour" })),
  }
}
const log = (...args) => console.log(new Date().toISOString().slice(11, 19), ...args)

const forwardHeaders = (req) => {
  const out = {}
  for (const key of ["authorization", "x-api-key", "content-type", "anthropic-version", "anthropic-beta", "x-anysola-client", "x-anysola-usage-event", "user-agent", "accept"])
    if (req.headers[key]) out[key] = req.headers[key]
  return out
}

async function upstream(req, path, body) {
  return fetch(UPSTREAM + path, { method: req.method, headers: forwardHeaders(req), body: body.length ? body : undefined })
}

async function liveMe(req) {
  const response = await fetch(UPSTREAM + "/v1/code/me", { headers: forwardHeaders(req) })
  return response.ok ? response.json() : {}
}

const json = (res, status, data, headers = {}) => {
  res.writeHead(status, { "content-type": "application/json", ...headers })
  res.end(JSON.stringify(data))
}

const day = (offset) => new Date(Date.now() - offset * 864e5).toISOString().slice(0, 10)

http
  .createServer(async (req, res) => {
    const url = new URL(req.url ?? "/", "http://x")
    const chunks = []
    for await (const chunk of req) chunks.push(chunk)
    const body = Buffer.concat(chunks)
    try {
      if (url.pathname === "/v1/code/version")
        return json(res, 200, {
          latest: process.env.LATEST ?? "0.3.1",
          minimum: process.env.MINIMUM ?? "0.2.0",
          npmPackage: "@anysola/code",
          notes: "Fake release notes.",
        })

      if (url.pathname === "/v1/code/me") {
        const me = await liveMe(req)
        log("me", state.fiveHour)
        return json(res, 200, {
          plan: me.plan ?? "team",
          access: me.access ?? "paid",
          limits: limits(),
          usageToday: { requests: state.requests },
          event: process.env.EVENT ? { mode: "multiplier", label: process.env.EVENT, until: new Date(Date.now() + 3 * 864e5).toISOString() } : null,
        })
      }

      if (url.pathname === "/v1/code/limits") return json(res, 200, limits())

      if (url.pathname === "/v1/code/limits/redeem" && req.method === "POST") {
        if (!state.bank)
          return json(res, 409, { type: "error", error: { type: "invalid_request_error", message: "No banked resets to spend." } })
        state.bank -= 1
        state.full = false
        state.fiveHour = 0
        log("redeem", state.bank)
        return json(res, 200, limits())
      }

      if (url.pathname === "/v1/code/usage") {
        const days = Number(url.searchParams.get("days") ?? 7) === 30 ? 30 : 7
        const byDay = Array.from({ length: days }, (_, index) => ({ date: day(days - 1 - index), requests: index * 3 }))
        byDay[byDay.length - 1] = { date: day(0), requests: state.requests }
        const requests = byDay.reduce((sum, item) => sum + item.requests, 0)
        return json(res, 200, {
          days,
          totals: { requests, cacheHitRate: 0.6 },
          byDay,
          byModel: [
            { model: "claude-sonnet-5-5", requests: Math.ceil(requests * 0.7) },
            { model: "claude-haiku-5-5", requests: Math.floor(requests * 0.3) },
          ],
          today: { requests: state.requests },
          plan: "team",
          limits: limits(),
        })
      }

      if (url.pathname === "/v1/code/tokens")
        return json(res, 200, {
          tokens: [
            { id: "tok_current", name: "agim-206806", client: "anysola-code/0.3.0", createdAt: day(2), lastUsedAt: new Date().toISOString(), expiresAt: "2027-02-01T00:00:00Z", current: true },
            ...(state.revoked.includes("tok_laptop") ? [] : [{ id: "tok_laptop", name: "macbook", client: "anysola-code/0.2.0", createdAt: day(9), lastUsedAt: day(1), expiresAt: "2027-01-20T00:00:00Z", current: false }]),
          ],
        })

      const revoke = url.pathname.match(/^\/v1\/code\/tokens\/([^/]+)\/revoke$/)
      if (revoke && req.method === "POST") {
        state.revoked.push(decodeURIComponent(revoke[1]))
        return json(res, 200, { revoked: true })
      }

      if (url.pathname === "/v1/code/config") {
        const response = await upstream(req, url.pathname + url.search, body)
        const config = await response.json()
        const models = config.providers?.anysola?.models ?? {}
        for (const model of Object.values(models)) delete model.credits_per_turn_estimate
        // Route the model calls through this proxy so they count toward the windows here.
        if (config.providers?.anysola?.settings) config.providers.anysola.settings.baseURL = `http://127.0.0.1:${PORT}/v1/code`
        return json(res, response.status, config)
      }

      if (url.pathname === "/v1/code/messages" && req.method === "POST") {
        if (!limits().allowed) {
          const banked = state.bank ? ` You have ${state.bank} banked reset${state.bank === 1 ? "" : "s"}.` : ""
          return json(
            res,
            429,
            { type: "error", error: { type: "rate_limit_error", message: `5-hour limit reached. Resets in 2 h.${banked}` } },
            { "x-should-retry": "false", "x-anysola-limit": "five_hour" },
          )
        }
        const response = await upstream(req, url.pathname, body)
        const headers = Object.fromEntries([...response.headers].filter(([key]) => ["content-type", "request-id", "x-request-id"].includes(key)))
        if (response.ok) {
          state.requests += 1
          state.fiveHour += USED
          state.weekly += USED / 4
        }
        log("messages", response.status, state.fiveHour)
        res.writeHead(response.status, headers)
        if (response.body) Readable.fromWeb(response.body).pipe(res)
        else res.end()
        return
      }

      const response = await upstream(req, url.pathname + url.search, body)
      res.writeHead(response.status, { "content-type": response.headers.get("content-type") ?? "application/json" })
      res.end(Buffer.from(await response.arrayBuffer()))
    } catch (error) {
      log("error", error)
      json(res, 502, { type: "error", error: { type: "api_error", message: String(error) } })
    }
  })
  .listen(PORT, "127.0.0.1", () => log(`fake anysola on http://127.0.0.1:${PORT} -> ${UPSTREAM}`))
