/**
 * Set C, "Palette first": the palette is where session actions live, with steps (pick the message
 * to branch from, set the budget); the budget shows on the open session's sidebar row; a fork
 * nests under its source in the sidebar and names it in the header.
 */
import { ChevronRight, CornerDownRight, GitBranch, Search } from "lucide-react"
import { useMemo, useRef, useState } from "react"

import type { SessionRow } from "../bridge"
import { openSession, useStore, type Open } from "../store"
import { ago, cn, folderName, Kbd, useEscape } from "../ui"
import {
  atBudget,
  BudgetEditor,
  BudgetStrip,
  closePalette,
  FILES_NOTE,
  fork,
  forkTitle,
  matches,
  Meter,
  openPalette,
  PaletteRows,
  pct,
  setStep,
  useCommands,
  useLocal,
  usePaletteKeys,
  useSessions,
  useUsage,
  type PRow,
} from "./shared"

type Filter = "all" | "sessions" | "commands"

export function Palette() {
  const shown = useLocal((l) => l.palette)
  if (!shown) return null
  return <PaletteC />
}

function PaletteC() {
  const step = useLocal((l) => l.step)
  const open = useStore((s) => s.open)
  const panel = useRef<HTMLDivElement>(null)
  useEscape(panel, () => (step === "root" ? closePalette() : setStep("root")))
  return (
    <div className="fixed inset-0 z-40 flex items-start justify-center bg-scrim pt-[12vh]" onClick={closePalette}>
      <div
        ref={panel}
        role="dialog"
        aria-label="Command palette"
        onClick={(e) => e.stopPropagation()}
        className="flex max-h-[64vh] w-full max-w-[40rem] flex-col overflow-hidden rounded-2xl bg-overlay"
      >
        {step === "root" || !open ? <Root /> : step === "fork" ? <ForkStep open={open} /> : <BudgetStep open={open} />}
      </div>
    </div>
  )
}

function Crumb({ label }: { label: string }) {
  const title = useStore((s) => s.open?.title ?? "")
  return (
    <div className="flex h-12 shrink-0 items-center gap-1.5 px-4 text-sm">
      <button onClick={() => setStep("root")} className="max-w-60 truncate text-muted hover:text-foreground">
        {forkTitle(title).base}
      </button>
      <ChevronRight className="size-3.5 shrink-0 text-muted" />
      <span className="min-w-0 flex-1 truncate">{label}</span>
      <Kbd>Esc</Kbd>
    </div>
  )
}

function Root() {
  const [q, setQ] = useState("")
  const [filter, setFilter] = useState<Filter>("all")
  const sessions = useSessions()
  const base = useCommands()
  const openId = useStore((s) => s.open?.sessionId)
  const rows = useMemo<PRow[]>(() => {
    const run = (f: () => void) => () => {
      closePalette()
      f()
    }
    // This session's commands step into the palette instead of opening something elsewhere.
    const commands = base.flatMap((c) =>
      c.id === "fork"
        ? [
            { ...c, id: "fork-pick", label: "Branch from a message…", step: "fork" as const },
            { ...c, label: "Branch whole session" },
          ]
        : c.id === "budget"
          ? [{ ...c, step: "budget" as const }]
          : [c]
    ) as (ReturnType<typeof useCommands>[number] & { step?: "fork" | "budget" })[]
    const cmd = filter === "sessions" ? [] : commands.filter((c) => matches(q, c.label))
    const ses = filter === "commands" ? [] : sessions.filter((s) => matches(q, s.title, s.directory)).slice(0, q || filter === "sessions" ? 12 : 4)
    const line = (c: (typeof commands)[number]) => (
      <>
        {c.icon}
        <span className="min-w-0 flex-1 truncate">{c.label}</span>
        {c.step ? <ChevronRight className="size-3.5 text-muted" /> : c.keys && <Kbd>{c.keys}</Kbd>}
      </>
    )
    const cRow = (c: (typeof commands)[number], section: string): PRow => ({
      key: c.id,
      section,
      run: c.step ? () => setStep(c.step!) : run(c.run),
      node: line(c),
    })
    return [
      ...cmd.filter((c) => c.group === "session").map((c) => cRow(c, "This session")),
      ...ses.map((s) => ({ key: s.id, section: "Sessions", run: run(() => void openSession(s)), node: <SessionLine s={s} current={s.id === openId} /> })),
      ...cmd.filter((c) => c.group === "app").map((c) => cRow(c, "App")),
    ]
  }, [q, filter, sessions, base, openId])
  const { at, setAt, onKeyDown } = usePaletteKeys(rows)
  return (
    <>
      <div className="flex h-12 shrink-0 items-center gap-3 pr-2 pl-4">
        <Search className="size-4 shrink-0 text-muted" strokeWidth={1.75} />
        <input
          autoFocus
          value={q}
          onChange={(e) => setQ(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder="Search or run a command"
          className="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted"
        />
        <div className="flex h-8 items-center rounded-full bg-default/60 p-0.5" role="radiogroup" aria-label="Show">
          {(["all", "sessions", "commands"] as const).map((f) => (
            <button
              key={f}
              role="radio"
              aria-checked={filter === f}
              onClick={() => setFilter(f)}
              className={cn(
                "text-2xs h-7 rounded-full px-2.5 capitalize transition-colors",
                filter === f ? "bg-default-hover text-foreground" : "text-muted hover:text-foreground"
              )}
            >
              {f}
            </button>
          ))}
        </div>
      </div>
      <div className="min-h-0 overflow-y-auto px-1.5 pb-1.5">
        {rows.length === 0 ? <p className="text-dense px-2.5 py-3 text-muted">Nothing matches “{q.trim()}”.</p> : <PaletteRows rows={rows} at={at} setAt={setAt} />}
      </div>
    </>
  )
}

function SessionLine({ s, current }: { s: SessionRow; current: boolean }) {
  const { base, n } = forkTitle(s.title)
  return (
    <>
      <span className="min-w-0 flex-1 truncate">
        {base}
        {n !== null && <span className="text-muted"> · fork</span>}
      </span>
      <span className="text-2xs shrink-0 text-muted">{folderName(s.directory)}</span>
      <span className="text-2xs w-14 shrink-0 text-right text-muted tabular-nums">{current ? "Open" : ago(s.updated)}</span>
    </>
  )
}

/** Pick the message to branch before; the first can't be (nothing comes before it). */
function ForkStep({ open }: { open: Open }) {
  const users = open.items.filter((i) => i.kind === "user")
  const rows: PRow[] = [
    ...users.slice(1).reverse().map((u) => ({
      key: u.id,
      section: "Before a message",
      run: () => void fork(open, u.id),
      node: (
        <>
          <CornerDownRight className="size-3.5 shrink-0 text-muted" />
          <span className="min-w-0 flex-1 truncate">{u.kind === "user" ? u.text : ""}</span>
          <span className="text-2xs shrink-0 text-muted tabular-nums">{u.kind === "user" && u.time ? ago(u.time) : ""}</span>
        </>
      ),
    })),
    {
      key: "whole",
      section: "Everything",
      run: () => void fork(open, null),
      node: (
        <>
          <GitBranch className="size-3.5 shrink-0 text-muted" />
          <span className="min-w-0 flex-1 truncate">The whole conversation</span>
        </>
      ),
    },
  ]
  const { at, setAt, onKeyDown } = usePaletteKeys(rows)
  return (
    <div tabIndex={-1} autoFocus onKeyDown={onKeyDown} ref={(el) => el?.focus()} className="flex min-h-0 flex-col outline-none">
      <Crumb label="Branch from…" />
      <div className="min-h-0 overflow-y-auto px-1.5">
        <PaletteRows rows={rows} at={at} setAt={setAt} />
      </div>
      <p className="text-2xs flex h-10 shrink-0 items-center px-4 text-muted">
        Starts a new session with the conversation up to there. {FILES_NOTE}
      </p>
    </div>
  )
}

function BudgetStep({ open }: { open: Open }) {
  return (
    <>
      <Crumb label="Session budget" />
      <div className="px-2 pb-2">
        <BudgetEditor open={open} onDone={closePalette} className="w-full" />
      </div>
    </>
  )
}

export function SearchRow() {
  return (
    <button
      onClick={() => openPalette()}
      className="mt-1 flex h-9 w-full items-center gap-2 rounded-lg px-3 text-sm text-muted hover:bg-default/60"
    >
      <Search className="size-4 shrink-0" strokeWidth={1.75} />
      <span className="flex-1 text-left">Search or run</span>
      <Kbd>Ctrl K</Kbd>
    </button>
  )
}

/** The open session's row carries its budget on a second line; a fork shows nested (see sidebar). */
export function RowTitle({ title, active, id }: { title: string; active: boolean; id: string }) {
  const { base, n } = forkTitle(title)
  const open = useStore((s) => (s.open?.sessionId === id ? s.open : null))
  const u = useUsage(open)
  const show = active && u && u.budget !== null
  return (
    <span className="flex min-w-0 flex-col">
      <span className={cn("flex min-w-0 items-center gap-1.5 text-sm", !active && "text-foreground/85")}>
        <span className="truncate">{base}</span>
        {n !== null && <span className="text-2xs shrink-0 text-muted">fork</span>}
      </span>
      {show && (
        <button
          onClick={(e) => {
            e.stopPropagation()
            openPalette("budget")
          }}
          className={cn("text-2xs mt-1 flex w-full items-center gap-2 text-muted tabular-nums", atBudget(u) && "text-warning")}
        >
          <Meter u={u} className="flex-1" />
          <span className="shrink-0">
            {pct(u.total)} of {u.budget}%
          </span>
        </button>
      )}
    </span>
  )
}

/** Under the title in the header: where a fork came from. */
export function HeaderNote({ open }: { open: Open }) {
  const from = useLocal((l) => (open.sessionId ? l.forks[open.sessionId] : undefined))
  const source = useStore((s) => s.sessions.find((r) => r.id === from?.sourceId) ?? null)
  if (!from) return null
  return (
    <span className="text-2xs flex min-w-0 shrink items-center gap-1 text-muted">
      <GitBranch className="size-3 shrink-0" strokeWidth={1.75} />
      <span className="truncate">
        from{" "}
        <button onClick={() => source && void openSession(source)} className="hover:text-foreground">
          {from.sourceTitle}
        </button>
      </span>
    </span>
  )
}

export function Banner({ open }: { open: Open }) {
  return <BudgetStrip open={open} />
}

/** Forks under their source (by title, as the bridge has no `forkOf` yet). */
export function nest(rows: SessionRow[]): (SessionRow & { nested?: boolean })[] {
  const forks = rows.filter((r) => forkTitle(r.title).n !== null)
  const out: (SessionRow & { nested?: boolean })[] = []
  for (const r of rows) {
    if (forkTitle(r.title).n !== null && rows.some((s) => s.title === forkTitle(r.title).base)) continue
    out.push(r)
    for (const f of forks) if (forkTitle(f.title).base === r.title) out.push({ ...f, nested: true })
  }
  return out
}
