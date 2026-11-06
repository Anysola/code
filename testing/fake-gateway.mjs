// Minimal stand-in for api.anysola.com/v1/code: /config as the gateway builds it with Auto and economy on, and
// the usage limits (/me, /limits, /limits/redeem, /usage, the anysola_usage stream event).
//   LIMIT=1      every model request answers 429 for a full 5-hour window (until a redeem)
//   a prompt containing "limitreached" is refused the same way, once per banked reset spent
import http from "node:http"
const PORT = Number(process.env.PORT ?? 8797)
const efforts = ["low", "medium", "high", "xhigh", "max"]
const economy = { id: "economy", settings: { thinking: { type: "adaptive", display: "summarized" }, effort: "low" }, headers: { "x-anysola-variant": "economy" } }
const base = `http://127.0.0.1:${PORT}/v1/code`
const config = {
  model: "anysola/claude-sonnet-5-5",
  small_model: "anysola/claude-haiku-5-5",
  agents: { title: { model: "anysola/claude-haiku-5-5" } },
  providers: { anysola: { name: "Anysola", package: "@opencode/ai/providers/anthropic-compatible",
    settings: { baseURL: base, apiKey: "{env:ANYSOLA_TOKEN}" }, headers: { "x-anysola-client": "opencode" },
    models: {
      auto: { name: "Auto", description: "Picks Haiku or Sonnet per turn",
        limit: { context: 1000000, output: 32000 }, capabilities: { reasoning: { supported: true, effort: efforts } },
        compatibility: { supportsThinkingBlockBinding: true, supportsEffortUpdates: true }, variants: [economy] },
      "claude-sonnet-5-5": { name: "Claude Sonnet 5.5", description: "Default", limit: { context: 1000000, output: 32000 },
        capabilities: { reasoning: { supported: true, effort: efforts } }, variants: [economy] },
      "claude-haiku-5-5": { name: "Claude Haiku 5.5", description: "Fastest", limit: { context: 200000, output: 32000 } },
    } } },
}
const HOUR = 3600e3
const state = { fiveHour: 42, weekly: 18, bank: 2, spent: 0, full: Boolean(process.env.LIMIT), requests: 3 }
const at = (ms) => new Date(Date.now() + ms).toISOString()
const limits = () => ({
  plan: "team",
  allowed: !state.full,
  blockedBy: state.full ? "five_hour" : null,
  fiveHour: { percent: state.full ? 100 : Math.floor(state.fiveHour), resetsAt: state.fiveHour > 0 || state.full ? at(2 * HOUR) : null },
  weekly: { percent: Math.floor(state.weekly), resetsAt: at(76 * HOUR) },
  bank: Array.from({ length: state.bank }, (_, i) => ({ id: `bank_${state.spent + i + 1}`, expiresAt: at((state.spent + i + 20) * 24 * HOUR), scope: "five_hour" })),
})
const me = () => ({ plan: "team", access: "paid", limits: limits(), usageToday: { requests: state.requests }, event: null })
const day = (offset) => new Date(Date.now() - offset * 864e5).toISOString().slice(0, 10)
const usage = (days) => ({
  days,
  totals: { requests: 42, cacheHitRate: 0.61 },
  byDay: Array.from({ length: days }, (_, i) => ({ date: day(days - 1 - i), requests: i === days - 1 ? state.requests : 0 })),
  byModel: [{ model: "claude-sonnet-5-5", requests: 40 }, { model: "claude-haiku-5-5", requests: 2 }],
  today: { requests: state.requests },
  plan: "team",
  limits: limits(),
})
const refused = () => {
  const banked = state.bank ? ` You have ${state.bank} banked reset${state.bank === 1 ? "" : "s"}.` : ""
  return { type: "error", error: { type: "rate_limit_error", message: `5-hour limit reached. Resets in 2 h.${banked}` } }
}
http.createServer((req, res) => {
  const url = new URL(req.url, base)
  const send = (status, body) => { res.writeHead(status, { "content-type": "application/json" }); res.end(JSON.stringify(body)) }
  if (url.pathname === "/v1/code/config") return send(200, config)
  if (url.pathname === "/v1/code/me") return send(200, me())
  if (url.pathname === "/v1/code/limits") return send(200, limits())
  if (url.pathname === "/v1/code/limits/redeem" && req.method === "POST") {
    if (!state.bank) return send(409, { type: "error", error: { type: "invalid_request_error", message: "No banked resets to spend." } })
    state.bank -= 1
    state.spent += 1
    state.full = false
    state.fiveHour = 0
    return send(200, limits())
  }
  if (url.pathname === "/v1/code/usage") return send(200, usage(Number(url.searchParams.get("days") ?? 30)))
  if (url.pathname === "/v1/code/version") return send(200, { latest: "0.5.0", minimum: "0.2.0" })
  if (url.pathname === "/v1/code/search" && req.method === "POST")
    return send(200, { results: [{ url: "https://bun.com", title: "Bun", content: "Bun 1.4.4" }] })
  if (url.pathname === "/v1/code/messages" && req.method === "POST") {
    let body = ""
    req.on("data", (c) => (body += c))
    req.on("end", () => {
      const parsed = JSON.parse(body)
      const last = parsed.messages.at(-1)
      const afterTool = Array.isArray(last.content) && last.content.some((p) => p.type === "tool_result")
      const wantsSearch = JSON.stringify(parsed.messages.at(-1)).toLowerCase().includes("latest")
      const isTitle = !parsed.tools?.length
      const asksLimit = JSON.stringify(parsed.messages.at(-1)).includes("limitreached") && state.spent === 0
      if (!isTitle && (state.full || asksLimit)) {
        state.full = true
        res.writeHead(429, { "content-type": "application/json", "x-should-retry": "false", "x-anysola-limit": "five_hour" })
        return res.end(JSON.stringify(refused()))
      }
      const events = []
      const ev = (type, data) => events.push(`event: ${type}
data: ${JSON.stringify({ type, ...data })}

`)
      ev("message_start", { message: { id: "msg_fake", type: "message", role: "assistant", model: parsed.model, content: [], stop_reason: null, usage: { input_tokens: 10, output_tokens: 1 } } })
      if (isTitle || afterTool || !wantsSearch) {
        ev("content_block_start", { index: 0, content_block: { type: "text", text: "" } })
        const userText = (m) => (typeof m.content === "string" ? m.content : (m.content ?? []).filter((p) => p.type === "text").map((p) => p.text).join(" "))
        const asked = userText(parsed.messages.findLast((m) => m.role === "user") ?? { content: "" })
        const known = { "latest bun": "Latest Bun version", "failing date parser": "Fix date parser tests", "--json flag": "Add a --json flag", "explain this project": "Explain this project" }
        const hit = Object.entries(known).find(([needle]) => JSON.stringify(parsed.messages).toLowerCase().includes(needle))
        const title = hit ? hit[1] : "New session"
        const answer = afterTool
          ? "The latest stable Bun is **1.4.4**, from bun.com."
          : "This project is a small TypeScript CLI. `src/index.ts` parses the flags and `src/run.ts` does the work; tests live in `test/` and run with `bun test`."
        ev("content_block_delta", { index: 0, delta: { type: "text_delta", text: isTitle ? title : answer } })
        ev("content_block_stop", { index: 0 })
        ev("message_delta", { delta: { stop_reason: "end_turn" }, usage: { output_tokens: 3 } })
      } else {
        ev("content_block_start", { index: 0, content_block: { type: "tool_use", id: "toolu_fake1", name: "websearch", input: {} } })
        ev("content_block_delta", { index: 0, delta: { type: "input_json_delta", partial_json: JSON.stringify({ query: "latest bun" }) } })
        ev("content_block_stop", { index: 0 })
        ev("message_delta", { delta: { stop_reason: "tool_use" }, usage: { output_tokens: 5 } })
      }
      ev("message_stop", {})
      const used = isTitle ? 0 : 1.5
      if (!isTitle) state.requests += 1
      state.fiveHour = Math.min(100, state.fiveHour + used)
      state.weekly = Math.min(100, state.weekly + used / 4)
      const window = limits()
      if (req.headers["x-anysola-usage-event"] === "1") ev("anysola_usage", { fiveHourPercentUsed: used, fiveHour: window.fiveHour, weekly: window.weekly })
      res.writeHead(200, { "content-type": "text/event-stream" })
      res.end(events.join(""))
    })
    return
  }
  send(404, { type: "error", error: { type: "not_found_error", message: "fake" } })
}).listen(PORT, "127.0.0.1", () => console.log("fake on", PORT))
