// Stand-in for the round-2 Anysola endpoints that are not deployed yet. Plain Node, no deps.
//
//   node scripts/fake-anysola.mjs          # PORT=8796 UPSTREAM=https://api.anysola.com
//
// /v1/code/messages and /config are proxied to the live gateway (real model answers, real token), with the
// `x-credits-charged` / `x-credits-remaining` headers the gateway will add. /me gains the new fields; /usage,
// /tokens and /version are synthetic, in the shapes agreed with the server agent.
//   CHARGE=9            credits charged per /messages request
//   START=0             starting balance (default: live /me balance)
//   EVENT="label"       /me.event label (empty: no event)
//   MINIMUM=0.2.0       /version minimum; LATEST=0.3.1
//   FORCE_402=1         /messages answers 402 with a gateway-style message
import http from "node:http"
import { Readable } from "node:stream"

const PORT = Number(process.env.PORT ?? 8796)
const UPSTREAM = (process.env.UPSTREAM ?? "https://api.anysola.com").replace(/\/+$/, "")
const CHARGE = Number(process.env.CHARGE ?? 9)
const state = { remaining: process.env.START ? Number(process.env.START) : undefined, requests: 0, charged: 0, revoked: [] }
const log = (...args) => console.log(new Date().toISOString().slice(11, 19), ...args)

const forwardHeaders = (req) => {
  const out = {}
  for (const key of ["authorization", "x-api-key", "content-type", "anthropic-version", "anthropic-beta", "x-anysola-client", "user-agent", "accept"])
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
        if (state.remaining === undefined) state.remaining = me.credits?.available ?? 1000
        log("me", state.remaining)
        return json(res, 200, {
          ...me,
          credits: me.credits ? { ...me.credits, available: state.remaining } : me.credits,
          event: process.env.EVENT ? { mode: "bonus", label: process.env.EVENT, until: new Date(Date.now() + 3 * 864e5).toISOString() } : null,
          usageToday: { requests: state.requests, credits: state.charged },
          lowBalance: state.remaining < 50,
        })
      }

      if (url.pathname === "/v1/code/usage") {
        const days = Number(url.searchParams.get("days") ?? 7) === 30 ? 30 : 7
        const byDay = Array.from({ length: days }, (_, index) => ({ date: day(days - 1 - index), requests: index * 3, credits: index * 27 }))
        byDay[byDay.length - 1] = { date: day(0), requests: state.requests, credits: state.charged }
        const totals = byDay.reduce((sum, item) => ({ requests: sum.requests + item.requests, credits: sum.credits + item.credits, costUsd: 0 }), { requests: 0, credits: 0, costUsd: 0 })
        totals.costUsd = Number((totals.credits * 0.01).toFixed(2))
        return json(res, 200, {
          days,
          totals,
          byDay,
          byModel: [
            { model: "claude-sonnet-5-5", requests: Math.ceil(totals.requests * 0.7), credits: Math.ceil(totals.credits * 0.8) },
            { model: "claude-haiku-5-5", requests: Math.floor(totals.requests * 0.3), credits: Math.floor(totals.credits * 0.2) },
          ],
          today: { requests: state.requests, credits: state.charged },
          freeQuota: null,
          plan: "team",
          creditsAvailable: state.remaining ?? 0,
          renewsAt: "2026-11-23T13:27:46.340Z",
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
        const estimates = { "claude-haiku-5-5": 3, "claude-sonnet-5-5": 9, "claude-opus-5-5": 15 }
        for (const [id, model] of Object.entries(models)) if (estimates[id]) model.credits_per_turn_estimate = estimates[id]
        // Route the model calls through this proxy so they carry the credit headers.
        if (config.providers?.anysola?.settings) config.providers.anysola.settings.baseURL = `http://127.0.0.1:${PORT}/v1/code`
        return json(res, response.status, config)
      }

      if (url.pathname === "/v1/code/messages" && req.method === "POST") {
        if (process.env.FORCE_402)
          return json(res, 402, { type: "error", error: { type: "billing_error", message: "You are out of credits. They renew on Nov 23." } })
        const response = await upstream(req, url.pathname, body)
        if (state.remaining === undefined) state.remaining = 1000
        const headers = Object.fromEntries([...response.headers].filter(([key]) => ["content-type", "request-id", "x-request-id"].includes(key)))
        if (response.ok) {
          state.requests += 1
          state.charged += CHARGE
          state.remaining -= CHARGE
          headers["x-credits-charged"] = String(CHARGE)
          headers["x-credits-remaining"] = String(state.remaining)
        }
        log("messages", response.status, headers["x-credits-remaining"] ?? "")
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
