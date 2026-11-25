/**
 * The palette (Ctrl K, or Search in the sidebar): recent sessions, then commands with their keys.
 * A layer like any panel: Escape closes it and is not a press toward stopping a turn.
 */
import { GitBranch, GitMerge, Globe, Keyboard, MessageSquare, Plus, RotateCcw, Search, Settings as SettingsIcon, SquarePen, Terminal as TerminalIcon, Trash2, Wallet } from "lucide-react"
import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react"

import type { SessionRow } from "./bridge"
import { askDiscard, editBudget, flash, setReviewing, showPalette, showShortcuts, togglePreview, toggleTerminal, useChrome } from "./chrome"
import { modes } from "./composer"
import { KEYS } from "./shortcuts"
import { closeSettings, forkSession, isRunning, newSession, notify, openSession, openSettings, pickMode, undoApplyWorktree, useStore, useWorktree, useWorktreeInfo } from "./store"
import { ago, cn, folderName, Kbd, sameFolder, useEscape } from "./ui"

type Row = { key: string; section: string; run: () => void; node: ReactNode }

type Command = { id: string; label: string; hint?: string; icon: ReactNode; keys?: string; run: () => void }

function useCommands(): Command[] {
  const open = useStore((s) => s.open)
  const mode = useStore((s) => s.mode)
  const busy = useStore((s) => !!s.open?.sessionId && s.busy.has(s.open.sessionId))
  const budget = useStore((s) => (s.open?.sessionId ? (s.usage[s.open.sessionId]?.budget ?? null) : null))
  const worktree = useWorktree(open?.sessionId)
  // In a git folder a fork gets its own worktree with the files as they were.
  const isolates = useWorktreeInfo(open?.directory)?.available ?? false
  const i = "size-4 shrink-0 text-muted"
  const list: Command[] = [
    {
      id: "new",
      label: "New session",
      icon: <Plus className={i} strokeWidth={1.75} />,
      keys: KEYS.newSession,
      run: () => {
        closeSettings()
        void newSession()
      },
    },
  ]
  if (open?.sessionId) {
    const { directory, sessionId } = open
    // A running turn is half written: branching waits for it to end.
    if (!busy && !isRunning(open))
      list.push({
        id: "fork",
        label: "Branch this session",
        hint: isolates ? "Whole conversation, in its own worktree" : "Whole conversation",
        icon: <GitBranch className={i} strokeWidth={1.75} />,
        run: () => void forkSession(directory, sessionId, null),
      })
    list.push(
      {
        id: "budget",
        label: budget === null ? "Set budget…" : "Change budget…",
        icon: <Wallet className={i} strokeWidth={1.75} />,
        // After the palette has gone, so the popover isn't closed by the focus going back.
        run: () => setTimeout(() => editBudget(true), 0),
      },
      { id: "review", label: "Review changes", icon: <SquarePen className={i} strokeWidth={1.75} />, keys: KEYS.review, run: () => setReviewing(true) }
    )
    if (worktree?.state === "ready")
      list.push({
        id: "apply",
        label: "Apply to main folder",
        hint: `One commit on ${worktree.baseBranch ?? "the main folder’s branch"}`,
        icon: <GitMerge className={i} strokeWidth={1.75} />,
        run: () => setReviewing(true, "apply"),
      })
    if (worktree?.state === "ready" && worktree.appliedCommit)
      list.push({
        id: "undo-apply",
        label: "Undo last apply",
        hint: worktree.appliedCommit.slice(0, 7),
        icon: <RotateCcw className={i} strokeWidth={1.75} />,
        run: () => {
          void undoApplyWorktree({ directory, sessionId }).then((r) => {
            if ("error" in r) notify(r.error)
            // Pushed already: the worktree menu asks before undoing it.
            else if ("pushed" in r) notify(`${r.pushed.join(", ")} already has this commit. Undo it from the worktree menu, which asks first.`)
            else flash("Undid the last apply")
          })
        },
      })
    if (worktree && worktree.state !== "discarded" && worktree.state !== "applying" && worktree.state !== "preparing")
      list.push({
        id: "discard",
        label: "Discard worktree",
        hint: worktree.branch,
        icon: <Trash2 className={i} strokeWidth={1.75} />,
        // After the palette has gone, so its Escape and focus don't reach the confirm.
        run: () => setTimeout(() => askDiscard(true), 0),
      })
  }
  if (open) {
    list.push(
      {
        id: "terminal",
        label: "Toggle terminal",
        icon: <TerminalIcon className={i} strokeWidth={1.75} />,
        keys: KEYS.terminal,
        run: () => toggleTerminal(),
      },
      {
        id: "preview",
        label: "Toggle preview",
        icon: <Globe className={i} strokeWidth={1.75} />,
        keys: KEYS.preview,
        run: () => togglePreview(),
      }
    )
    const shown = open.info?.agent === "plan" ? "plan" : mode
    const hasPlan = open.info?.agents.some((a) => a.id === "plan") ?? false
    for (const m of modes)
      if (m.id !== shown && (m.id !== "plan" || hasPlan))
        list.push({
          id: `mode-${m.id}`,
          label: `Switch to ${m.label}`,
          hint: m.hint,
          icon: <MessageSquare className={i} strokeWidth={1.75} />,
          run: () => pickMode(m.id),
        })
  }
  list.push(
    { id: "settings", label: "Settings", icon: <SettingsIcon className={i} strokeWidth={1.75} />, keys: KEYS.settings, run: () => openSettings() },
    { id: "shortcuts", label: "Keyboard shortcuts", icon: <Keyboard className={i} strokeWidth={1.75} />, keys: KEYS.shortcuts, run: () => showShortcuts(true) }
  )
  return list
}

function matches(q: string, ...texts: string[]) {
  const s = q.trim().toLowerCase()
  return !s || texts.some((t) => t.toLowerCase().includes(s))
}

export function Palette() {
  const shown = useChrome((c) => c.palette)
  return shown ? <Panel /> : null
}

function Panel() {
  const [q, setQ] = useState("")
  /** The highlighted row by key: it stays on the same item when the rows change under it. */
  const [picked, setPicked] = useState<string | null>(null)
  const panel = useRef<HTMLDivElement>(null)
  const list = useRef<HTMLDivElement>(null)
  const close = () => showPalette(false)
  useEscape(panel, close)
  // Focus goes back where it was (the message box, mostly) unless a command moved it.
  useEffect(() => {
    const before = document.activeElement as HTMLElement | null
    return () => {
      if (before?.isConnected && (document.activeElement === document.body || !document.activeElement)) before.focus()
    }
  }, [])
  const all = useStore((s) => s.sessions)
  const deleting = useChrome((c) => c.deleting)
  const openId = useStore((s) => s.open?.sessionId ?? null)
  const commands = useCommands()
  const rows = useMemo<Row[]>(() => {
    const go = (f: () => void) => () => {
      close()
      f()
    }
    const sessions = all.filter((s) => !deleting?.ids.has(s.id) && matches(q, s.title, s.directory)).slice(0, q.trim() ? 8 : 5)
    return [
      ...sessions.map((s) => ({
        key: s.id,
        section: q.trim() ? "Sessions" : "Recent sessions",
        run: go(() => void openSession(s)),
        node: <SessionLine s={s} current={s.id === openId} />,
      })),
      ...commands
        .filter((c) => matches(q, c.label))
        .map((c) => ({
          key: c.id,
          section: "Commands",
          run: go(c.run),
          node: (
            <>
              {c.icon}
              <span className="min-w-0 flex-1 truncate">{c.label}</span>
              {c.id === "fork" && <span className="text-2xs shrink-0 text-muted">{c.hint}</span>}
              {c.keys && <Kbd>{c.keys}</Kbd>}
            </>
          ),
        })),
    ]
  }, [q, all, deleting, commands, openId])
  useEffect(() => setPicked(null), [q])
  const current = Math.max(0, rows.findIndex((r) => r.key === picked))
  useEffect(() => {
    list.current?.querySelector<HTMLElement>(`[data-row="${current}"]`)?.scrollIntoView({ block: "nearest" })
  }, [current])
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault()
      const step = e.key === "ArrowDown" ? 1 : -1
      if (rows.length) setPicked(rows[(current + step + rows.length) % rows.length].key)
    } else if (e.key === "Enter" && !e.nativeEvent.isComposing) {
      e.preventDefault()
      rows[current]?.run()
    }
  }
  let last: string | undefined
  return (
    <div className="fixed inset-0 z-40 flex items-start justify-center bg-scrim px-6 pt-[14vh]" onClick={close}>
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
            aria-label="Search sessions and commands"
            role="combobox"
            aria-expanded
            aria-controls="palette-list"
            className="min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted"
          />
          <Kbd>Esc</Kbd>
        </label>
        <div ref={list} id="palette-list" role="listbox" className="min-h-0 overflow-y-auto px-1.5 pb-1.5">
          {rows.length === 0 ? (
            <p className="text-dense px-2.5 py-3 text-muted">Nothing matches “{q.trim()}”.</p>
          ) : (
            rows.map((r, i) => {
              const head = r.section !== last ? r.section : null
              last = r.section
              return (
                <div key={r.key}>
                  {head && <div className={cn("text-2xs flex h-7 items-end px-2.5 pb-1 font-medium tracking-wide text-muted", i > 0 && "mt-1.5")}>{head}</div>}
                  <button
                    role="option"
                    data-row={i}
                    aria-selected={i === current}
                    tabIndex={-1}
                    onMouseMove={() => setPicked(r.key)}
                    onClick={r.run}
                    className={cn(
                      "text-dense flex h-9 w-full items-center gap-2.5 rounded-lg px-2.5 text-left outline-none",
                      i === current && "bg-default-hover"
                    )}
                  >
                    {r.node}
                  </button>
                </div>
              )
            })
          )}
        </div>
      </div>
    </div>
  )
}

function SessionLine({ s, current }: { s: SessionRow; current: boolean }) {
  // The app's own folder reads as the sidebar names it.
  const scratch = useStore((x) => x.scratch)
  return (
    <>
      {s.forkOf && <GitBranch className="size-3.5 shrink-0 text-muted" strokeWidth={1.75} aria-label="Branched" />}
      <span className="min-w-0 truncate">{s.title}</span>
      <span className="text-2xs min-w-0 flex-1 truncate text-muted">{sameFolder(s.directory, scratch) ? "No project" : folderName(s.directory)}</span>
      <span className="text-2xs shrink-0 text-muted tabular-nums">{current ? "Open" : ago(s.updated)}</span>
    </>
  )
}

/** The sidebar's Search: opens the palette. */
export function SearchButton() {
  return (
    <button
      onClick={() => showPalette(true)}
      className="mt-1 flex h-9 w-full items-center gap-2 rounded-lg px-3 text-sm text-muted hover:bg-default/60 focus-visible:status-focused-field"
    >
      <Search className="size-4 shrink-0" strokeWidth={1.75} />
      <span className="flex-1 text-left">Search</span>
      <Kbd>{KEYS.palette}</Kbd>
    </button>
  )
}
