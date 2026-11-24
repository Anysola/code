/**
 * Set A, "Header": a centered palette, grouped; the budget sits in the session header; fork is a
 * hover action on a user message, and the new session says once, above the composer, what a fork is.
 */
import { GitBranch, Search, X } from "lucide-react"
import { useMemo, useRef, useState } from "react"

import type { SessionRow } from "../bridge"
import { openSession, useStore, type Open } from "../store"
import { ago, btn, cn, folderName, Kbd, Popover, Tooltip, useEscape } from "../ui"
import {
  atBudget,
  BudgetEditor,
  BudgetStrip,
  closePalette,
  dismiss,
  editBudget,
  FILES_NOTE,
  fork,
  forkTitle,
  matches,
  Meter,
  openPalette,
  PaletteRows,
  pct,
  useCommands,
  useForkable,
  useLocal,
  usePaletteKeys,
  useSessions,
  useUsage,
  type PRow,
} from "./shared"

export function Palette() {
  const shown = useLocal((l) => l.palette)
  if (!shown) return null
  return <PaletteA />
}

function PaletteA() {
  const [q, setQ] = useState("")
  const panel = useRef<HTMLDivElement>(null)
  useEscape(panel, closePalette)
  const sessions = useSessions()
  const commands = useCommands()
  const openId = useStore((s) => s.open?.sessionId)
  const rows = useMemo<PRow[]>(() => {
    const ses = sessions.filter((s) => matches(q, s.title, s.directory)).slice(0, q ? 8 : 5)
    const cmd = commands.filter((c) => matches(q, c.label))
    const run = (f: () => void) => () => {
      closePalette()
      f()
    }
    return [
      ...ses.map((s) => ({
        key: s.id,
        section: q ? "Sessions" : "Recent sessions",
        run: run(() => void openSession(s)),
        node: <SessionLine s={s} current={s.id === openId} />,
      })),
      ...cmd.map((c) => ({
        key: c.id,
        section: "Commands",
        run: run(c.run),
        node: (
          <>
            {c.icon}
            <span className="min-w-0 flex-1 truncate">{c.label}</span>
            {c.hint && c.id === "fork" && <span className="text-2xs text-muted">{c.hint}</span>}
            {c.keys && <Kbd>{c.keys}</Kbd>}
          </>
        ),
      })),
    ]
  }, [q, sessions, commands, openId])
  const { at, setAt, onKeyDown } = usePaletteKeys(rows)
  return (
    <div className="fixed inset-0 z-40 flex items-start justify-center bg-scrim pt-[14vh]" onClick={closePalette}>
      <div
        ref={panel}
        role="dialog"
        aria-label="Search sessions and commands"
        onClick={(e) => e.stopPropagation()}
        className="flex max-h-[60vh] w-full max-w-[36rem] flex-col overflow-hidden rounded-2xl bg-overlay"
      >
        <label className="flex h-12 shrink-0 items-center gap-3 px-4">
          <Search className="size-4 shrink-0 text-muted" strokeWidth={1.75} />
          <input
            autoFocus
            value={q}
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder="Search sessions and commands"
            className="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted"
          />
          <Kbd>Esc</Kbd>
        </label>
        <div className="min-h-0 overflow-y-auto px-1.5 pb-1.5">
          {rows.length === 0 ? (
            <p className="text-dense px-2.5 py-3 text-muted">Nothing matches “{q.trim()}”.</p>
          ) : (
            <PaletteRows rows={rows} at={at} setAt={setAt} />
          )}
        </div>
      </div>
    </div>
  )
}

function SessionLine({ s, current }: { s: SessionRow; current: boolean }) {
  const { base, n } = forkTitle(s.title)
  return (
    <>
      {n !== null && <GitBranch className="size-3.5 shrink-0 text-muted" strokeWidth={1.75} />}
      <span className="min-w-0 truncate">{base}</span>
      <span className="text-2xs min-w-0 flex-1 truncate text-muted">{folderName(s.directory)}</span>
      <span className="text-2xs shrink-0 text-muted tabular-nums">{current ? "Open" : ago(s.updated)}</span>
    </>
  )
}

/** The sidebar's search row opens the palette. */
export function SearchRow() {
  return (
    <button
      onClick={() => openPalette()}
      className="mt-1 flex h-9 w-full items-center gap-2 rounded-lg px-3 text-sm text-muted hover:bg-default/60"
    >
      <Search className="size-4 shrink-0" strokeWidth={1.75} />
      <span className="flex-1 text-left">Search</span>
      <Kbd>Ctrl K</Kbd>
    </button>
  )
}

/** Spent against the budget, by the session's other header controls; nothing without a budget. */
export function HeaderBudget({ open }: { open: Open }) {
  const u = useUsage(open)
  const editing = useLocal((l) => l.editing)
  if (!u || (u.budget === null && !editing)) return null
  const at = atBudget(u)
  return (
    <div className="relative">
      <Tooltip label="Session budget, share of the 5-hour window">
        <button
          onClick={() => editBudget(!editing)}
          className={btn("ghost", "sm", cn("gap-2 text-muted tabular-nums", at && "text-warning"))}
        >
          {u.budget !== null ? (
            <>
              <Meter u={u} className="w-8" />
              {pct(u.total)} of {u.budget}%
            </>
          ) : (
            "Budget"
          )}
        </button>
      </Tooltip>
      <Popover open={editing} onClose={() => editBudget(false)} side="bottom" align="end" label="Session budget">
        <BudgetEditor open={open} onDone={() => editBudget(false)} />
      </Popover>
    </div>
  )
}

/** On hover under a user message, with Copy, Edit and Rewind. */
export function UserAction({ id }: { id: string }) {
  const forkable = useForkable(id)
  const open = useStore((s) => s.open)
  if (!forkable || !open) return null
  return (
    <Tooltip label="Branch from here" side="top">
      <button
        onClick={() => void fork(open, id)}
        aria-label="Branch from here"
        className="flex size-7 items-center justify-center rounded-md text-muted hover:bg-default/60 hover:text-foreground focus-visible:status-focused-field"
      >
        <GitBranch className="size-3.5" />
      </button>
    </Tooltip>
  )
}

/** Above the composer: stopped at the budget, or a fresh fork's one note. */
export function Banner({ open }: { open: Open }) {
  const from = useLocal((l) => (open.sessionId ? l.forks[open.sessionId] : undefined))
  const gone = useLocal((l) => !!open.sessionId && !!l.dismissed[open.sessionId])
  const source = useStore((s) => s.sessions.find((r) => r.id === from?.sourceId) ?? null)
  return (
    <>
      <BudgetStrip open={open} />
      {from && !gone && (
        <div role="status" className="text-dense mb-2 flex min-h-10 items-center gap-2.5 rounded-xl bg-surface/70 py-1 pr-1.5 pl-3.5">
          <GitBranch className="size-3.5 shrink-0 text-muted" />
          <span className="min-w-0 flex-1 py-1">
            <span className="block truncate">
              Branched from <span className="text-foreground">{from.sourceTitle}</span>
            </span>
            <span className="text-2xs block text-muted">{FILES_NOTE}</span>
          </span>
          {source && (
            <button onClick={() => void openSession(source)} className={btn("ghost", "sm", "text-muted")}>
              Open original
            </button>
          )}
          <Tooltip label="Dismiss" side="top">
            <button onClick={() => dismiss(open.sessionId!)} aria-label="Dismiss" className={btn("ghost", "sm", "text-muted", true)}>
              <X className="size-3.5" />
            </button>
          </Tooltip>
        </div>
      )}
    </>
  )
}

/** A fork's row: the branch mark, and the title without the core's ` (fork #n)`. */
export function RowTitle({ title, active }: { title: string; active: boolean }) {
  const { base, n } = forkTitle(title)
  return (
    <span className={cn("flex min-w-0 items-center gap-1.5 text-sm", !active && "text-foreground/85")}>
      {n !== null && <GitBranch className="size-3.5 shrink-0 text-muted" strokeWidth={1.75} aria-label={`Fork ${n} of`} />}
      <span className="truncate">{base}</span>
    </span>
  )
}
