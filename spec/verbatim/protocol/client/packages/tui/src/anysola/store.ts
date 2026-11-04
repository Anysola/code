import type { AnysolaRpc } from "@opencode/schema/anysola"
import { createSignal } from "solid-js"
import { createStore, reconcile } from "solid-js/store"
import { formatCredits } from "./format"

export { formatCredits, formatDate } from "./format"

// Anysola account state shared by the status line, banners, dialogs, the model picker and the exit summary.
// The `anysola` TUI plugin fills it from the server's `anysola.account` RPC.

export type Me = {
  plan?: string
  access?: "paid" | "free"
  credits?: { available?: number; renewsAt?: string } | null
  freeQuota?: { dailyUsd?: number; dailyTurns?: number; usedUsd?: number; usedTurns?: number; resetsAt?: string } | null
  event?: { mode?: string; label?: string; until?: string | null } | null
  usageToday?: { requests?: number; credits?: number }
  lowBalance?: boolean
}

/** Below this many credits the status line warns even when the server did not flag a low balance. */
export const LOW_BALANCE = 100
export const BILLING_URL = "https://account.anysola.com/billing"

const [sessions, setSessions] = createStore<Record<string, AnysolaRpc.SessionCredits>>({})
const [me, setMe] = createSignal<Me>()
const [estimates, setEstimates] = createSignal<Record<string, number>>({})
const [notes, setNotes] = createSignal<Record<string, string>>({})
let budgetGate: ((sessionID: string) => Promise<boolean>) | undefined

export const Account = {
  sessions,
  me,
  estimates,
  setSession(value: AnysolaRpc.SessionCredits) {
    setSessions(value.sessionID, reconcile(value))
  },
  setMe(value: Me | undefined) {
    setMe(value)
  },
  setEstimates(value: Record<string, number>) {
    setEstimates(value)
  },
  /** What each Anysola model is for, from the remote config. */
  notes,
  setNotes(value: Record<string, string>) {
    setNotes(value)
  },
  /** Latest balance: the gateway headers of this process, else /me. */
  remaining(sessionID?: string) {
    const fromSession = sessionID ? sessions[sessionID]?.remaining : undefined
    if (typeof fromSession === "number") return fromSession
    const latest = Object.values(sessions).find((item) => typeof item.remaining === "number")?.remaining
    if (typeof latest === "number") return latest
    const available = me()?.credits?.available
    return typeof available === "number" && Number.isFinite(available) ? available : undefined
  },
  /** The Session has spent its whole `/budget`. */
  overBudget(sessionID?: string) {
    const session = sessionID ? sessions[sessionID] : undefined
    return typeof session?.budget === "number" && session.total >= session.budget
  },
  /** Installed by the `anysola` plugin: asks what to do when a Session is over budget. */
  setBudgetGate(gate: typeof budgetGate) {
    budgetGate = gate
  },
  /** Whether the composer may start another turn; a failing gate never blocks the prompt. */
  allowTurn(sessionID: string) {
    if (!budgetGate) return Promise.resolve(true)
    return budgetGate(sessionID).catch(() => true)
  },
  low(sessionID?: string) {
    const remaining = Account.remaining(sessionID)
    if (me()?.access === "free") return false
    return me()?.lowBalance === true || (typeof remaining === "number" && remaining < LOW_BALANCE)
  },
}

/** "835 credits · this turn 12 · session 40" ("session 40/50" with a budget), or the free quota for free accounts. */
export function statusText(sessionID: string | undefined) {
  const account = me()
  const session = sessionID ? sessions[sessionID] : undefined
  if (account?.access === "free" && account.freeQuota) {
    const quota = account.freeQuota
    return `today ${quota.usedTurns ?? 0}/${quota.dailyTurns ?? 0}`
  }
  const remaining = Account.remaining(sessionID)
  if (remaining === undefined) return undefined
  const parts = [`${formatCredits(remaining)} credits`]
  if (session && session.turn > 0) parts.push(`this turn ${formatCredits(session.turn)}`)
  if (session && (session.total > 0 || typeof session.budget === "number"))
    parts.push(
      `session ${formatCredits(session.total)}${typeof session.budget === "number" ? `/${formatCredits(session.budget)}` : ""}`,
    )
  return parts.join(" · ")
}

/** Printed after the TUI closes: turns and credits spent in a Session. */
export function exitSummary(sessionID: string | undefined) {
  const session = sessionID ? sessions[sessionID] : undefined
  if (!session || session.total <= 0) return undefined
  return `${session.turns} turn${session.turns === 1 ? "" : "s"} · ${formatCredits(session.total)} credits`
}
