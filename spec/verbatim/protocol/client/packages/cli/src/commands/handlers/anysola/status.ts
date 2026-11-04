import { EOL } from "node:os"
import { Effect } from "effect"
import { Commands } from "../../commands"
import { Runtime } from "../../../framework/runtime"
import { errorMessage } from "../../../util/error"
import { createClient } from "../auth/shared"
import { activeToken, anysolaIntegration, call, mask } from "./account"

const str = (value: unknown) => (typeof value === "string" && value ? value : undefined)
const num = (value: unknown) => (typeof value === "number" && Number.isFinite(value) ? value : undefined)
const obj = (value: unknown) => (value && typeof value === "object" ? (value as Record<string, unknown>) : undefined)

/** Formats `GET /v1/code/me`; every field is optional while the shape settles. */
export function statusLines(me: Record<string, unknown>): Array<[string, string]> {
  const lines: Array<[string, string]> = []
  const email = str(me.email) ?? str(obj(me.user)?.email)
  if (email) lines.push(["Account", email])
  const plan = str(me.plan) ?? str(obj(me.plan)?.name)
  if (plan) lines.push(["Plan", plan])
  const credits = obj(me.credits)
  const available = num(me.credits) ?? num(credits?.available)
  if (available !== undefined) {
    const renews = str(credits?.renewsAt)
    lines.push(["Credits", `${available.toLocaleString("en-US")} available${renews ? `, renews ${renews.slice(0, 10)}` : ""}`])
  }
  const free = obj(me.freeQuota) ?? obj(me.freeTier)
  if (free) {
    const used = num(free.usedTurns) ?? num(free.turnsUsed)
    const daily = num(free.dailyTurns) ?? num(free.turnsLimit)
    if (daily !== undefined) lines.push(["Free quota", `${used ?? 0}/${daily} turns today`])
  }
  const model = str(me.defaultModel) ?? str(me.model)
  if (model) lines.push(["Model", model])
  const event = str(obj(me.event)?.label)
  if (event) lines.push(["Event", event])
  return lines
}

export default Runtime.handler(
  Commands.commands.status,
  Effect.fn("cli.anysola.status")(
    function* () {
      const client = yield* createClient()
      const current = yield* activeToken(client, yield* anysolaIntegration(client))
      if (!current) {
        process.stderr.write("Not signed in. Run `anysola login`." + EOL)
        process.exitCode = 1
        return
      }
      const reply = yield* call("GET", "/v1/code/me", current.token)
      if (reply.status === 401) {
        process.stderr.write("The token was revoked or expired. Run `anysola login --force`." + EOL)
        process.exitCode = 1
        return
      }
      const lines = reply.ok ? statusLines(reply.body) : []
      if (!reply.ok) process.stderr.write(`Could not load status: HTTP ${reply.status}` + EOL)
      lines.push(["Token", mask(current.token)])
      const width = Math.max(...lines.map(([key]) => key.length)) + 2
      process.stdout.write(lines.map(([key, value]) => key.padEnd(width) + value).join(EOL) + EOL)
    },
    Effect.catch((error) =>
      Effect.sync(() => {
        process.stderr.write(errorMessage(error) + EOL)
        process.exitCode = 1
      }),
    ),
  ),
)
