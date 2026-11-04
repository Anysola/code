// Credits of Anysola web searches, handed from the search provider (which knows the charge but not the Session) to
// the account plugin's websearch `execute.after` hook (which knows the Session), keyed by query.

const pending = new Map<string, number[]>()
// Searches that never reach the hook (the tool failed after the search) must not pile up.
const LIMIT = 64

export function note(query: string, credits: number) {
  if (!Number.isFinite(credits) || credits <= 0) return
  if (pending.size >= LIMIT) pending.delete(pending.keys().next().value!)
  pending.set(query, [...(pending.get(query) ?? []), credits])
}

/** The credits of the oldest search for `query` not taken yet. */
export function take(query: string) {
  const list = pending.get(query)
  if (!list?.length) return undefined
  const credits = list.shift()
  if (list.length === 0) pending.delete(query)
  return credits
}

export * as AnysolaSearchCredits from "./anysola-credits.js"
