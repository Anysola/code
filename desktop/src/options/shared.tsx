/**
 * Layout options for the palette, the session budget and forking (paket1 choice round).
 * Throwaway: `?opt=a|b|c` picks a set; without it nothing here renders and the app is as before.
 * The real build keeps one set and folds it into the components it belongs to.
 */
import { GitBranch, Keyboard, MessageSquare, Plus, Settings as SettingsIcon, SquarePen, Wallet } from "lucide-react"
import { useEffect, useState, useSyncExternalStore, type KeyboardEvent, type ReactNode } from "react"

import { MAX_BUDGET, type SessionRow, type SessionUsage } from "../bridge"
import { setReviewing, showShortcuts } from "../chrome"
import { modes } from "../composer"
import {
  chooseAgent,
  currentBridge,
  newSession,
  openSession,
  openSettings,
  setMode,
  useStore,
  type Open,
} from "../store"
import { btn, cn } from "../ui"

export type Opt = "a" | "b" | "c"
export const OPT: Opt | null = (() => {
  const v = new URLSearchParams(location.search).get("opt")
  return v === "a" || v === "b" || v === "c" ? v : null
})()

// ---------------------------------------------------------------------------
// A tiny store for what the app's store doesn't keep yet: usage, forks, the palette
// ---------------------------------------------------------------------------

type Local = {
  usage: Record<string, SessionUsage>
  /** New session id → where it came from (the real build would want `forkOf` on SessionRow). */
  forks: Record<string, { sourceId: string; sourceTitle: string; before: string | null; at: number }>
  palette: boolean
  /** The budget editor, wherever the set shows it. */
  editing: boolean
  /** Set C's palette step. */
  step: "root" | "fork" | "budget"
  dismissed: Record<string, boolean>
}
let local: Local = { usage: {}, forks: {}, palette: false, editing: false, step: "root", dismissed: {} }
const subs = new Set<() => void>()
function put(patch: Partial<Local> | ((l: Local) => Partial<Local>)) {
  local = { ...local, ...(typeof patch === "function" ? patch(local) : patch) }
  for (const s of subs) s()
}
export function useLocal<T>(select: (l: Local) => T): T {
  return useSyncExternalStore(
    (l) => {
      subs.add(l)
      return () => subs.delete(l)
    },
    () => select(local)
  )
}

export function openPalette(step: Local["step"] = "root") {
  put({ palette: true, step })
}
export function closePalette() {
  put({ palette: false, step: "root" })
}
export function setStep(step: Local["step"]) {
  put({ step })
}
export function editBudget(on = true) {
  put({ editing: on })
}
export function dismiss(id: string) {
  put((l) => ({ dismissed: { ...l.dismissed, [id]: true } }))
}

/** The open session's usage, read once and kept as set. */
export function useUsage(open: Open | null): SessionUsage | null {
  const id = open?.sessionId ?? null
  const u = useLocal((l) => (id ? (l.usage[id] ?? null) : null))
  useEffect(() => {
    if (!open?.sessionId || local.usage[open.sessionId]) return
    const sid = open.sessionId
    void currentBridge()
      .sessionUsage(open.directory, sid)
      .then((x) => put((l) => ({ usage: { ...l.usage, [sid]: x } })))
  }, [open?.sessionId, open?.directory])
  return u
}

export async function setBudget(open: Open, budget: number | null) {
  if (!open.sessionId) return
  const sid = open.sessionId
  const u = await currentBridge().setBudget(open.directory, sid, budget)
  put((l) => ({ usage: { ...l.usage, [sid]: u } }))
}

export const atBudget = (u: SessionUsage | null) => !!u && u.budget !== null && u.total >= u.budget

/** 6.4, 12, 0.8: one decimal under 10. */
export function pct(n: number) {
  return n < 10 ? String(Math.round(n * 10) / 10) : String(Math.round(n))
}

export async function fork(open: Open, before: string | null) {
  if (!open.sessionId) return
  const row = await currentBridge().forkSession(open.directory, open.sessionId, before)
  put((l) => ({ forks: { ...l.forks, [row.id]: { sourceId: open.sessionId!, sourceTitle: open.title, before, at: Date.now() } } }))
  closePalette()
  await openSession(row)
}

/** ` (fork #2)` off a title, as the core names forks. */
export function forkTitle(title: string): { base: string; n: number | null } {
  const m = / \(fork #(\d+)\)$/.exec(title)
  return m ? { base: title.slice(0, m.index), n: Number(m[1]) } : { base: title, n: null }
}

/** The first user message can't be forked from: nothing comes before it. */
export function useForkable(id: string) {
  return useStore((s) => {
    const first = s.open?.items.find((i) => i.kind === "user")
    return !!first && first.id !== id
  })
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

export type Command = { id: string; label: string; hint?: string; icon: ReactNode; keys?: string; run: () => void; group: "session" | "app" }

export function useCommands(budgetLabel = "Set budget…"): Command[] {
  const open = useStore((s) => s.open)
  const mode = useStore((s) => s.mode)
  const agent = open?.info?.agent
  const shown = agent === "plan" ? "plan" : mode
  const i = "size-4 text-muted"
  const list: Command[] = [{ id: "new", label: "New session", icon: <Plus className={i} />, keys: "Ctrl N", run: () => void newSession(), group: "app" }]
  if (open?.sessionId) {
    list.push(
      { id: "fork", label: "Branch this session", hint: "Whole conversation", icon: <GitBranch className={i} />, run: () => void fork(open, null), group: "session" },
      { id: "budget", label: budgetLabel, icon: <Wallet className={i} />, run: () => editBudget(), group: "session" },
      { id: "review", label: "Review changes", icon: <SquarePen className={i} />, keys: "Ctrl Shift D", run: () => setReviewing(true), group: "session" }
    )
    for (const m of modes)
      if (m.id !== shown)
        list.push({
          id: `mode-${m.id}`,
          label: `Switch to ${m.label}`,
          hint: m.hint,
          icon: <MessageSquare className={i} />,
          run: () => (m.id === "plan" ? void chooseAgent("plan") : setMode(m.id)),
          group: "session",
        })
  }
  list.push(
    { id: "settings", label: "Settings", icon: <SettingsIcon className={i} />, keys: "Ctrl ,", run: () => openSettings(), group: "app" },
    { id: "shortcuts", label: "Keyboard shortcuts", icon: <Keyboard className={i} />, keys: "Ctrl /", run: () => showShortcuts(true), group: "app" }
  )
  return list
}

export function matches(q: string, ...texts: string[]) {
  const s = q.trim().toLowerCase()
  return !s || texts.some((t) => t.toLowerCase().includes(s))
}

export function useSessions(): SessionRow[] {
  return useStore((s) => s.sessions)
}

// ---------------------------------------------------------------------------
// The budget editor (every set uses it; only where it opens differs)
// ---------------------------------------------------------------------------

const PRESETS = [5, 10, 25, 50]

export function BudgetEditor({ open, onDone, className }: { open: Open; onDone: () => void; className?: string }) {
  const u = useUsage(open)
  const [value, setValue] = useState(String(u?.budget ?? 10))
  const n = Number(value)
  const valid = Number.isInteger(n) && n >= 1 && n <= MAX_BUDGET
  const apply = async (b: number | null) => {
    await setBudget(open, b)
    onDone()
  }
  return (
    <div className={cn("flex w-72 flex-col gap-3 p-2", className)}>
      <div className="flex flex-col gap-0.5">
        <span className="text-sm font-medium">Session budget</span>
        <span className="text-2xs text-muted">
          Share of the 5-hour window this session may use. Used so far: {pct(u?.total ?? 0)}%
        </span>
      </div>
      <div className="flex items-center gap-1">
        {PRESETS.map((p) => (
          <button
            key={p}
            onClick={() => setValue(String(p))}
            className={cn(
              "text-dense h-8 flex-1 rounded-lg tabular-nums transition-colors",
              n === p ? "bg-default-hover text-foreground" : "bg-default/50 text-muted hover:text-foreground"
            )}
          >
            {p}%
          </button>
        ))}
        <label className="text-dense flex h-8 w-16 items-center rounded-lg bg-default/50 px-2 focus-within:bg-default-hover">
          <input
            value={PRESETS.includes(n) ? "" : value}
            placeholder="Other"
            onChange={(e) => setValue(e.target.value.replace(/[^0-9]/g, ""))}
            aria-label="Budget, percent of the 5-hour window"
            className="w-full min-w-0 bg-transparent text-right tabular-nums outline-none placeholder:text-muted"
          />
          <span className="pl-0.5 text-muted">%</span>
        </label>
      </div>
      <div className="flex items-center gap-2">
        {u?.budget != null && (
          <button onClick={() => void apply(null)} className={btn("ghost", "sm", "text-muted")}>
            Remove budget
          </button>
        )}
        <button onClick={() => void apply(n)} disabled={!valid} className={btn("primary", "sm", "ml-auto")}>
          {u?.budget != null ? "Save" : "Set budget"}
        </button>
      </div>
    </div>
  )
}

/** A thin meter, spent against the budget. */
export function Meter({ u, className }: { u: SessionUsage; className?: string }) {
  if (u.budget === null) return null
  const w = Math.min(100, (u.total / u.budget) * 100)
  return (
    <span className={cn("block h-1 overflow-hidden rounded-full bg-default", className)}>
      <span className={cn("block h-full rounded-full", atBudget(u) ? "bg-warning" : "bg-foreground/60")} style={{ width: `${w}%` }} />
    </span>
  )
}

// ---------------------------------------------------------------------------
// Stopped at the budget: one answer for every set (the Banner strip the limit already uses)
// ---------------------------------------------------------------------------

export function BudgetStrip({ open }: { open: Open }) {
  const u = useUsage(open)
  if (!u || !atBudget(u) || u.budget === null) return null
  const raise = Math.min(MAX_BUDGET, u.budget * 2)
  return (
    <div role="status" className="text-dense mb-2 flex min-h-10 items-center gap-2.5 rounded-xl bg-surface/70 py-1 pr-1.5 pl-3.5">
      <Wallet className="size-3.5 shrink-0 text-warning" />
      <span className="min-w-0 flex-1 py-1">
        <span className="block">
          Budget reached <span className="text-muted">· {u.budget}% of the 5-hour window</span>
        </span>
        <span className="text-2xs block text-muted">Raise it or remove it to keep going.</span>
      </span>
      <span className="flex shrink-0 items-center gap-1">
        <button onClick={() => void setBudget(open, raise)} className={btn("secondary", "sm")}>
          Raise to {raise}%
        </button>
        <button onClick={() => void setBudget(open, null)} className={btn("ghost", "sm", "text-muted")}>
          Remove budget
        </button>
      </span>
    </div>
  )
}

/** The fork's one line about files, worded once for every set. */
export const FILES_NOTE = "Same folder. Files stay as they are now; nothing is rewound."

// ---------------------------------------------------------------------------
// A keyboard list for the palettes
// ---------------------------------------------------------------------------

export type PRow = { key: string; section?: string; run: () => void; node: ReactNode }

/** ↑/↓ over the rows, Enter runs the highlighted one; the input keeps focus. */
export function usePaletteKeys(rows: PRow[]) {
  const [at, setAt] = useState(0)
  useEffect(() => setAt(0), [rows.length])
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault()
      setAt((a) => Math.min(rows.length - 1, a + 1))
    } else if (e.key === "ArrowUp") {
      e.preventDefault()
      setAt((a) => Math.max(0, a - 1))
    } else if (e.key === "Enter") {
      e.preventDefault()
      rows[at]?.run()
    }
  }
  return { at, setAt, onKeyDown }
}

export function PaletteRows({
  rows,
  at,
  setAt,
  sectionClass = "text-2xs flex h-7 items-end px-2.5 pb-1 font-medium tracking-wide text-muted",
  rowClass = "h-9",
}: {
  rows: PRow[]
  at: number
  setAt: (n: number) => void
  sectionClass?: string
  rowClass?: string
}) {
  let last: string | undefined
  return (
    <div role="listbox" className="flex flex-col">
      {rows.map((r, i) => {
        const head = r.section && r.section !== last ? r.section : null
        last = r.section
        return (
          <div key={r.key}>
            {head && <div className={cn(sectionClass, i > 0 && "mt-1.5")}>{head}</div>}
            <button
              role="option"
              aria-selected={i === at}
              tabIndex={-1}
              onMouseMove={() => setAt(i)}
              onClick={r.run}
              className={cn(
                "text-dense flex w-full items-center gap-2.5 rounded-lg px-2.5 text-left outline-none",
                rowClass,
                i === at ? "bg-default-hover" : ""
              )}
            >
              {r.node}
            </button>
          </div>
        )
      })}
    </div>
  )
}
