// Minimal stand-in for api.anysola.com/v1/code: /config as the gateway builds it with Auto and economy on.
import http from "node:http"
const PORT = 8797
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
        capabilities: { reasoning: { supported: true, effort: efforts } }, variants: [economy], credits_per_turn_estimate: 6 },
      "claude-haiku-5-5": { name: "Claude Haiku 5.5", description: "Fastest", limit: { context: 200000, output: 32000 }, credits_per_turn_estimate: 1 },
    } } },
}
const me = { plan: "team", access: "paid", credits: { available: 731, renewsAt: "2026-11-23T00:00:00Z" }, lowBalance: false, usageToday: { requests: 3, credits: 9 } }
http.createServer((req, res) => {
  const url = new URL(req.url, base)
  const send = (status, body) => { res.writeHead(status, { "content-type": "application/json" }); res.end(JSON.stringify(body)) }
  if (url.pathname === "/v1/code/config") return send(200, config)
  if (url.pathname === "/v1/code/me") return send(200, me)
  if (url.pathname === "/v1/code/version") return send(200, { latest: "0.5.0", minimum: "0.2.0" })
  if (url.pathname === "/v1/code/search" && req.method === "POST")
    return send(200, { results: [{ url: "https://bun.com", title: "Bun", content: "Bun 1.4.4" }], credits: 2 })
  if (url.pathname === "/v1/code/messages" && req.method === "POST") {
    let body = ""
    req.on("data", (c) => (body += c))
    req.on("end", () => {
      const parsed = JSON.parse(body)
      const last = parsed.messages.at(-1)
      const afterTool = Array.isArray(last.content) && last.content.some((p) => p.type === "tool_result")
      const wantsSearch = JSON.stringify(parsed.messages.at(-1)).toLowerCase().includes("latest")
      const isTitle = !parsed.tools?.length
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
      if (req.headers["x-anysola-credits-event"] === "1") ev("anysola_credits", { charged: isTitle ? 0 : 1 })
      res.writeHead(200, { "content-type": "text/event-stream", "x-credits-available-before": "731" })
      res.end(events.join(""))
    })
    return
  }
  send(404, { type: "error", error: { type: "not_found_error", message: "fake" } })
}).listen(PORT, "127.0.0.1", () => console.log("fake on", PORT))
