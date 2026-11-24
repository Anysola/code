/**
 * The session budget: a share of the 5-hour window the session may use. Shown in the header only
 * when set (spent against it, with a thin meter); set, changed or removed in a popover that the
 * header's ⋯ menu and the palette open. When it is spent the turn stops (see `Banner`).
 */
import { useEffect, useState } from "react"

import { MAX_BUDGET, type SessionUsage } from "./bridge"
import { editBudget, useChrome } from "./chrome"
import { setBudget, useStore, type Open } from "./store"
import { btn, cn, Popover, Tooltip } from "./ui"

/** The open session's usage and budget, as last read or sent; null before the first read. */
export function useUsage(sessionId: string | null): SessionUsage | null {
  return useStore((s) => (sessionId ? (s.usage[sessionId] ?? null) : null))
}

export const atBudget = (u: SessionUsage | null): u is SessionUsage & { budget: number } =>
  !!u && u.budget !== null && u.total >= u.budget

/** What "Raise to" offers: double, and always above what was spent (a turn can end a little
 * over); null when no higher budget is left. */
export function raiseTo(u: SessionUsage & { budget: number }): number | null {
  const next = Math.min(MAX_BUDGET, Math.max(u.budget * 2, Math.floor(u.total) + 1))
  return next > u.budget && next > u.total ? next : null
}

/** 6.4, 12, 0.8: one decimal under 10. */
export function pct(n: number) {
  return n < 10 ? String(Math.round(n * 10) / 10) : String(Math.round(n))
}

const PRESETS = [5, 10, 25, 50]

function BudgetEditor({ open, usage, onDone }: { open: Open; usage: SessionUsage | null; onDone: () => void }) {
  const [value, setValue] = useState(String(usage?.budget ?? 10))
  /** What is typed in Other, kept as typed: 5 then 0 makes 50, not a preset that empties it. */
  const [other, setOther] = useState(() => (PRESETS.includes(usage?.budget ?? 10) ? "" : String(usage?.budget)))
  const [saving, setSaving] = useState(false)
  const n = Number(value)
  const valid = value !== "" && Number.isInteger(n) && n >= 1 && n <= MAX_BUDGET
  const apply = async (budget: number | null) => {
    if (!open.sessionId) return
    setSaving(true)
    const done = await setBudget(open.directory, open.sessionId, budget)
    setSaving(false)
    if (done) onDone()
  }
  return (
    <form
      className="flex w-72 flex-col gap-3 p-2"
      onSubmit={(e) => {
        e.preventDefault()
        if (valid) void apply(n)
      }}
    >
      <div className="flex flex-col gap-0.5">
        <span className="text-sm font-medium">Session budget</span>
        <span className="text-2xs text-muted">
          Share of the 5-hour window this session and its subagents may use; work stops there. Used so far:{" "}
          {pct(usage?.total ?? 0)}%
        </span>
      </div>
      <div className="flex items-center gap-1">
        {PRESETS.map((p) => (
          <button
            key={p}
            type="button"
            onClick={() => {
              setValue(String(p))
              setOther("")
            }}
            aria-pressed={n === p && other === ""}
            className={cn(
              "text-dense h-8 flex-1 rounded-lg tabular-nums transition-colors focus-visible:status-focused-field",
              n === p && other === "" ? "bg-default-hover text-foreground" : "bg-default/50 text-muted hover:text-foreground"
            )}
          >
            {p}%
          </button>
        ))}
        <label className="text-dense flex h-8 w-16 items-center rounded-lg bg-default/50 px-2 focus-within:bg-default-hover">
          <input
            value={other}
            placeholder="Other"
            inputMode="numeric"
            onChange={(e) => {
              const typed = e.target.value.replace(/[^0-9]/g, "").slice(0, 3)
              setOther(typed)
              setValue(typed)
            }}
            aria-label="Budget, percent of the 5-hour window"
            className="w-full min-w-0 bg-transparent text-right tabular-nums outline-none placeholder:text-muted"
          />
          <span className="pl-0.5 text-muted">%</span>
        </label>
      </div>
      <div className="flex items-center gap-2">
        {usage?.budget != null && (
          <button type="button" onClick={() => void apply(null)} disabled={saving} className={btn("ghost", "sm", "text-muted")}>
            Remove budget
          </button>
        )}
        <button type="submit" disabled={!valid || saving} className={btn("primary", "sm", "ml-auto")}>
          {usage?.budget != null ? "Save" : "Set budget"}
        </button>
      </div>
    </form>
  )
}

/** A thin meter, spent against the budget. */
function Meter({ u }: { u: SessionUsage & { budget: number } }) {
  const w = Math.min(100, (u.total / u.budget) * 100)
  return (
    <span className="block h-1 w-8 overflow-hidden rounded-full bg-default">
      <span className={cn("block h-full rounded-full", atBudget(u) ? "bg-warning" : "bg-foreground/60")} style={{ width: `${w}%` }} />
    </span>
  )
}

/** Spent against the budget, by the header's other controls; nothing without one (unless the
 * editor was asked for, which then opens from here). */
export function HeaderBudget({ open }: { open: Open }) {
  const usage = useUsage(open.sessionId)
  const editing = useChrome((c) => c.budgeting)
  // The editor belongs to the session it opened for.
  useEffect(() => () => editBudget(false), [open.sessionId])
  if (!open.sessionId || (usage?.budget == null && !editing)) return null
  const spent = usage && usage.budget !== null ? (usage as SessionUsage & { budget: number }) : null
  return (
    <div className="relative">
      <Tooltip label="Session budget, share of the 5-hour window">
        <button
          onClick={() => editBudget(!editing)}
          aria-label="Session budget"
          className={btn("ghost", "sm", cn("gap-2 text-muted tabular-nums", atBudget(spent) && "text-warning"))}
        >
          {spent ? (
            <>
              <Meter u={spent} />
              {pct(spent.total)} of {spent.budget}%
            </>
          ) : (
            "Budget"
          )}
        </button>
      </Tooltip>
      <Popover open={editing} onClose={() => editBudget(false)} side="bottom" align="end" label="Session budget">
        <BudgetEditor open={open} usage={usage} onDone={() => editBudget(false)} />
      </Popover>
    </div>
  )
}
