/**
 * Set B, "Composer": the palette drops from the header as one flat list; the budget sits in the
 * composer's footer by Context; fork lives in a message menu that says what a fork is, and the new
 * session starts with a marker line in its transcript.
 */
import { Ellipsis, GitBranch, Search } from "lucide-react"
import { useMemo, useRef, useState } from "react"

import { useChrome } from "../chrome"
import { openSession, useStore, type Open } from "../store"
import { ago, cn, folderName, Kbd, MenuItem, Popover, Tooltip, useEscape } from "../ui"
import {
  atBudget,
  BudgetEditor,
  BudgetStrip,
  closePalette,
  editBudget,
  FILES_NOTE,
  fork,
  forkTitle,
  matches,
  Meter,
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
  return <PaletteB />
}

function PaletteB() {
  const [q, setQ] = useState("")
  const panel = useRef<HTMLDivElement>(null)
  useEscape(panel, closePalette)
  const hidden = useChrome((c) => c.sidebarHidden)
  const sessions = useSessions()
  const commands = useCommands()
  const only = q.startsWith(">")
  const text = only ? q.slice(1) : q
  const rows = useMemo<PRow[]>(() => {
    const run = (f: () => void) => () => {
      closePalette()
      f()
    }
    const cmd = commands.filter((c) => matches(text, c.label)).slice(0, text || only ? 20 : 4)
    const ses = only ? [] : sessions.filter((s) => matches(text, s.title, s.directory)).slice(0, 6)
    const cRows = cmd.map((c) => ({
      key: c.id,
      run: run(c.run),
      node: (
        <>
          <span className="flex size-6 shrink-0 items-center justify-center rounded-md bg-default/70 [&_svg]:size-3.5">{c.icon}</span>
          <span className="min-w-0 flex-1 truncate">{c.label}</span>
          {c.keys && <Kbd>{c.keys}</Kbd>}
        </>
      ),
    }))
    const sRows = ses.map((s) => {
      const { base, n } = forkTitle(s.title)
      return {
        key: s.id,
        run: run(() => void openSession(s)),
        node: (
          <>
            <span className="size-6 shrink-0" />
            <span className="min-w-0 truncate">{base}</span>
            {n !== null && <span className="text-2xs shrink-0 rounded bg-default/70 px-1 text-muted">#{n}</span>}
            <span className="min-w-0 flex-1" />
            <span className="text-2xs shrink-0 text-muted tabular-nums">
              {folderName(s.directory)} · {ago(s.updated)}
            </span>
          </>
        ),
      }
    })
    // Typing a session's name puts sessions first; the empty list leads with this session's commands.
    return text ? [...sRows, ...cRows] : [...cRows, ...sRows]
  }, [text, only, sessions, commands])
  const { at, setAt, onKeyDown } = usePaletteKeys(rows)
  return (
    <div className="fixed inset-0 z-40 bg-scrim/60" onClick={closePalette}>
      <div className={cn("absolute top-11 right-2 flex justify-center px-8", hidden ? "left-2" : "left-[16.5rem]")}>
        <div
          ref={panel}
          role="dialog"
          aria-label="Go to a session or run a command"
          onClick={(e) => e.stopPropagation()}
          className="flex max-h-[min(30rem,70vh)] w-full max-w-(--conversation-width) flex-col overflow-hidden rounded-2xl bg-overlay"
        >
          <label className="flex h-11 shrink-0 items-center gap-3 px-4">
            <Search className="size-4 shrink-0 text-muted" strokeWidth={1.75} />
            <input
              autoFocus
              value={q}
              onChange={(e) => setQ(e.target.value)}
              onKeyDown={onKeyDown}
              placeholder="Go to a session or run a command"
              className="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted"
            />
          </label>
          <div className="min-h-0 overflow-y-auto px-1.5 pb-1.5">
            {rows.length === 0 ? (
              <p className="text-dense px-2.5 py-3 text-muted">Nothing matches “{text.trim()}”.</p>
            ) : (
              <PaletteRows rows={rows} at={at} setAt={setAt} />
            )}
          </div>
          <div className="text-2xs flex h-8 shrink-0 items-center gap-4 bg-surface/50 px-4 text-muted">
            <span>
              <Kbd>↑</Kbd> <Kbd>↓</Kbd> move
            </span>
            <span>
              <Kbd>Enter</Kbd> open
            </span>
            <span>
              <Kbd>&gt;</Kbd> commands only
            </span>
          </div>
        </div>
      </div>
    </div>
  )
}

/** Next to Context in the composer's footer; nothing without a budget. */
export function FooterBudget({ open }: { open: Open }) {
  const u = useUsage(open)
  const editing = useLocal((l) => l.editing)
  if (!u || (u.budget === null && !editing)) return null
  const at = atBudget(u)
  return (
    <span className="relative flex shrink-0 items-center">
      <span className="text-2xs mx-2 text-muted/60">·</span>
      <Tooltip label="Session budget, share of the 5-hour window" side="top">
        <button
          onClick={() => editBudget(!editing)}
          className={cn(
            "text-2xs flex items-center gap-1.5 rounded-md px-1 py-0.5 text-muted tabular-nums hover:text-foreground focus-visible:status-focused-field",
            at && "text-warning"
          )}
        >
          {u.budget !== null ? (
            <>
              Budget
              <Meter u={u} className="w-10" />
              {pct(u.total)} / {u.budget}%
            </>
          ) : (
            "Budget"
          )}
        </button>
      </Tooltip>
      <Popover open={editing} onClose={() => editBudget(false)} side="top" align="end" label="Session budget">
        <BudgetEditor open={open} onDone={() => editBudget(false)} />
      </Popover>
    </span>
  )
}

/** A menu after Copy, Edit and Rewind: where Branch lives, with what it does. */
export function UserAction({ id }: { id: string }) {
  const forkable = useForkable(id)
  const open = useStore((s) => s.open)
  const [menu, setMenu] = useState(false)
  if (!open || !forkable) return null
  return (
    <span className="relative">
      <Tooltip label="More" side="top">
        <button
          onClick={() => setMenu(!menu)}
          aria-label="More for this message"
          aria-haspopup="menu"
          aria-expanded={menu}
          className="flex size-7 items-center justify-center rounded-md text-muted hover:bg-default/60 hover:text-foreground focus-visible:status-focused-field"
        >
          <Ellipsis className="size-3.5" />
        </button>
      </Tooltip>
      <Popover open={menu} onClose={() => setMenu(false)} side="bottom" align="end" menu label="Message actions" className="w-80">
        <MenuItem
          disabled={!forkable}
          onSelect={() => {
            setMenu(false)
            void fork(open, id)
          }}
        >
          <GitBranch className="size-3.5 text-muted" /> Branch from here
        </MenuItem>
        <p className="text-2xs px-2.5 pt-0.5 pb-1.5 pl-8 text-muted">
          A new session with the conversation before this message. {FILES_NOTE}
        </p>
      </Popover>
    </span>
  )
}

export function Banner({ open }: { open: Open }) {
  return <BudgetStrip open={open} />
}

/** At the top of a fork's transcript. */
export function TranscriptTop({ open }: { open: Open }) {
  const from = useLocal((l) => (open.sessionId ? l.forks[open.sessionId] : undefined))
  const source = useStore((s) => s.sessions.find((r) => r.id === from?.sourceId) ?? null)
  if (!from) return null
  return (
    <div className="text-dense mb-6 flex items-center gap-3 text-muted">
      <span className="h-px flex-1 bg-separator" />
      <GitBranch className="size-3.5" />
      <span>
        Branched from{" "}
        {source ? (
          <button onClick={() => void openSession(source)} className="text-foreground/85 underline-offset-2 hover:underline">
            {from.sourceTitle}
          </button>
        ) : (
          from.sourceTitle
        )}
      </span>
      <span className="h-px flex-1 bg-separator" />
    </div>
  )
}

/** The core's ` (fork #n)` as a small tag. */
export function RowTitle({ title, active }: { title: string; active: boolean }) {
  const { base, n } = forkTitle(title)
  return (
    <span className={cn("flex min-w-0 items-center gap-1.5 text-sm", !active && "text-foreground/85")}>
      <span className="truncate">{base}</span>
      {n !== null && <span className="text-2xs shrink-0 rounded bg-default/70 px-1 text-muted tabular-nums">#{n}</span>}
    </span>
  )
}
