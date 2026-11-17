/**
 * The sidebar: new session, search, pinned sessions, sessions by project (each project folds
 * shut and has its own menu), selecting several to delete at once, updates and the account.
 */
import {
  ArrowDownToLine,
  Check,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  Ellipsis,
  FolderClosed,
  FolderOpen,
  ListChecks,
  LogOut,
  MessageSquare,
  Pencil,
  Pin,
  PinOff,
  Plus,
  Search,
  Trash2,
  X,
} from "lucide-react"
import { useCallback, useEffect, useMemo, useRef, useState } from "react"

import type { SessionRow, Window as AccountWindow } from "./bridge"
import { capitalize } from "./composer"
import {
  deleteSessions,
  installUpdate,
  newScratchSession,
  newSession,
  openSession,
  pickFolder,
  renameSession,
  reveal,
  setDefaultFolder,
  signOut,
  toggleCollapsed,
  togglePin,
  useStore,
} from "./store"
import { ago, btn, cn, folderName, Kbd, Mark, MenuItem, Popover, resetsIn, Spinner, TitleBar } from "./ui"

/** A session's folder as the window names it: the app's own folder is "No project". */
function usePlace(directory: string) {
  const scratch = useStore((s) => s.scratch)
  return directory === scratch ? "No project" : folderName(directory)
}

export function PlaceLabel({ directory }: { directory: string }) {
  return <>{usePlace(directory)}</>
}

/** New session: the default folder, a recent one, one to pick, or none. */
function NewSessionButton() {
  const sessions = useStore((s) => s.sessions)
  const scratch = useStore((s) => s.scratch)
  const preferred = useStore((s) => s.defaultFolder)
  const [show, setShow] = useState(false)
  const close = useCallback(() => setShow(false), [])
  const recent = useMemo(() => {
    const seen: string[] = []
    for (const s of sessions) {
      if (s.directory === scratch || s.directory === preferred || seen.includes(s.directory)) continue
      seen.push(s.directory)
      if (seen.length === 4) break
    }
    return seen
  }, [sessions, scratch, preferred])
  const go = (f: () => Promise<void>) => {
    close()
    void f()
  }
  return (
    <div className="relative">
      <button
        onClick={() => setShow(!show)}
        aria-expanded={show}
        className={btn("secondary", "md", "w-full justify-start gap-2")}
      >
        <Plus className="size-4" />
        <span className="flex-1 text-left">New session</span>
        <Kbd>Ctrl N</Kbd>
      </button>
      <Popover open={show} onClose={close} side="bottom" align="start" className="w-full min-w-60">
        {preferred && (
          <MenuItem onSelect={() => go(() => newSession(preferred))}>
            <FolderClosed className="size-3.5 text-muted" />
            <span className="min-w-0 flex-1 truncate" title={preferred}>
              {folderName(preferred)}
            </span>
            <span className="text-2xs text-muted">Default</span>
          </MenuItem>
        )}
        {recent.map((d) => (
          <MenuItem key={d} onSelect={() => go(() => newSession(d))}>
            <FolderClosed className="size-3.5 text-muted" />
            <span className="min-w-0 flex-1 truncate" title={d}>
              {folderName(d)}
            </span>
          </MenuItem>
        ))}
        {(preferred || recent.length > 0) && <div className="my-1 border-t border-separator" />}
        <MenuItem
          onSelect={() =>
            go(async () => {
              const d = await pickFolder()
              if (d) await newSession(d)
            })
          }
        >
          <FolderOpen className="size-3.5 text-muted" /> Choose a folder…
        </MenuItem>
        <MenuItem onSelect={() => go(newScratchSession)}>
          <MessageSquare className="size-3.5 text-muted" /> No project
        </MenuItem>
      </Popover>
    </div>
  )
}

type Selecting = {
  selected: Set<string>
  /** Plain click opens; Ctrl (⌘) click adds or removes one; Shift click takes the range. */
  pick: (row: SessionRow, e: { ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }) => void
  select: (ids: string[]) => void
}

export function Sidebar() {
  const sessions = useStore((s) => s.sessions)
  const pinned = useStore((s) => s.pinned)
  const collapsed = useStore((s) => s.collapsed)
  const [query, setQuery] = useState("")
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const anchor = useRef<string | null>(null)
  const search = useRef<HTMLInputElement>(null)
  const q = query.trim().toLowerCase()
  const matches = useCallback(
    (s: SessionRow) => !q || s.title.toLowerCase().includes(q) || s.directory.toLowerCase().includes(q),
    [q]
  )
  const pins = useMemo(() => sessions.filter((s) => pinned.has(s.id) && matches(s)), [sessions, pinned, matches])
  const groups = useMemo(() => {
    const by = new Map<string, SessionRow[]>()
    for (const s of sessions) {
      if (pinned.has(s.id) || !matches(s)) continue
      by.set(s.directory, [...(by.get(s.directory) ?? []), s])
    }
    return [...by.entries()]
  }, [sessions, pinned, matches])
  // The rows in the order they show, for Shift ranges (folded projects show none).
  const order = useMemo(
    () => [...pins, ...groups.flatMap(([d, rows]) => (collapsed.has(d) && !q ? [] : rows))],
    [pins, groups, collapsed, q]
  )
  // A deleted or vanished session leaves the selection.
  useEffect(() => {
    setSelected((s) => {
      const next = new Set([...s].filter((id) => sessions.some((r) => r.id === id)))
      return next.size === s.size ? s : next
    })
  }, [sessions])
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault()
        search.current?.focus()
      }
      if (e.key === "Escape" && selected.size > 0) setSelected(new Set())
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [selected.size])

  const selecting: Selecting = {
    selected,
    pick(row, e) {
      if (e.shiftKey && anchor.current) {
        const from = order.findIndex((r) => r.id === anchor.current)
        const to = order.findIndex((r) => r.id === row.id)
        if (from >= 0 && to >= 0) {
          const [lo, hi] = from < to ? [from, to] : [to, from]
          setSelected(new Set([...selected, ...order.slice(lo, hi + 1).map((r) => r.id)]))
          return
        }
      }
      if (e.ctrlKey || e.metaKey || selected.size > 0) {
        const next = new Set(selected)
        if (!next.delete(row.id)) next.add(row.id)
        anchor.current = row.id
        setSelected(next)
        return
      }
      anchor.current = row.id
      void openSession(row)
    },
    select(ids) {
      anchor.current = ids[ids.length - 1] ?? null
      setSelected(new Set([...selected, ...ids]))
    },
  }

  return (
    <aside className="flex w-64 shrink-0 flex-col">
      <TitleBar>
        <div data-tauri-drag-region className="flex items-center gap-2 px-4 text-sm font-medium whitespace-nowrap">
          <Mark />
          Anysola Code
        </div>
      </TitleBar>
      <div className="px-2 pt-2 pb-4">
        <NewSessionButton />
        {sessions.length > 0 && (
          <label className="mt-1 flex h-9 items-center gap-2 rounded-lg px-3 text-muted focus-within:bg-default/60 hover:bg-default/60">
            <Search className="size-4 shrink-0" strokeWidth={1.75} />
            <input
              ref={search}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Escape") {
                  setQuery("")
                  e.currentTarget.blur()
                }
              }}
              placeholder="Search"
              aria-label="Search sessions"
              className="min-w-0 flex-1 bg-transparent text-sm text-foreground outline-none placeholder:text-muted"
            />
            {!query && <Kbd>Ctrl K</Kbd>}
          </label>
        )}
      </div>
      <nav className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-2 pb-3" aria-label="Sessions">
        {sessions.length === 0 && <p className="text-dense px-2 text-muted">No sessions yet.</p>}
        {sessions.length > 0 && pins.length === 0 && groups.length === 0 && (
          <p className="text-dense px-2 text-muted">No session matches “{query.trim()}”.</p>
        )}
        {pins.length > 0 && (
          <div>
            <div className="text-2xs flex h-7 items-center gap-1.5 px-2 font-medium tracking-wide text-muted">
              <Pin className="size-3" strokeWidth={1.75} />
              Pinned
            </div>
            <ul className="flex flex-col">
              {pins.map((r) => (
                <SessionLink key={r.id} row={r} selecting={selecting} showPlace />
              ))}
            </ul>
          </div>
        )}
        {groups.map(([directory, rows]) => (
          <Project
            key={directory}
            directory={directory}
            rows={rows}
            // A search shows every match, folded or not.
            folded={collapsed.has(directory) && !q}
            selecting={selecting}
          />
        ))}
      </nav>
      {selected.size > 0 ? (
        <SelectionBar
          rows={sessions.filter((s) => selected.has(s.id))}
          onDone={() => setSelected(new Set())}
        />
      ) : (
        <>
          <UpdateRow />
          <AccountRow />
        </>
      )}
    </aside>
  )
}

/** A project's sessions under its folder's name, which folds them shut. */
function Project({
  directory,
  rows,
  folded,
  selecting,
}: {
  directory: string
  rows: SessionRow[]
  folded: boolean
  selecting: Selecting
}) {
  const scratch = useStore((s) => s.scratch)
  const [menu, setMenu] = useState(false)
  const close = useCallback(() => setMenu(false), [])
  const busy = useStore((s) => rows.some((r) => s.busy.has(r.id)))
  return (
    <div>
      <div className="group/project relative">
        <button
          onClick={() => toggleCollapsed(directory)}
          aria-expanded={!folded}
          title={directory}
          className="text-2xs flex h-7 w-full items-center gap-1.5 rounded-md px-2 text-left font-medium tracking-wide text-muted hover:text-foreground focus-visible:status-focused-field"
        >
          {folded ? (
            <ChevronRight className="size-3 shrink-0" strokeWidth={2} />
          ) : (
            <ChevronDown className="size-3 shrink-0" strokeWidth={2} />
          )}
          <span className="min-w-0 truncate">
            <PlaceLabel directory={directory} />
          </span>
          {folded && (
            <span className="ml-auto flex items-center gap-1.5 pr-7 font-normal tabular-nums">
              {busy && <Spinner className="size-3" />}
              {rows.length}
            </span>
          )}
        </button>
        <button
          onClick={() => setMenu(!menu)}
          aria-label={`Actions for ${folderName(directory)}`}
          className={cn(
            "absolute top-0.5 right-1.5 flex size-6 items-center justify-center rounded-md text-muted hover:bg-default-hover hover:text-foreground focus-visible:status-focused-field",
            !menu && "invisible group-hover/project:visible focus-visible:visible"
          )}
        >
          <Ellipsis className="size-4" />
        </button>
        <Popover open={menu} onClose={close} side="bottom" align="end" className="w-60">
          <MenuItem
            onSelect={() => {
              close()
              void newSession(directory)
            }}
          >
            <Plus className="size-3.5 text-muted" /> New session here
          </MenuItem>
          {directory !== scratch && (
            <MenuItem
              onSelect={() => {
                close()
                void reveal(directory)
              }}
            >
              <FolderOpen className="size-3.5 text-muted" /> Open folder
            </MenuItem>
          )}
          <MenuItem
            onSelect={() => {
              close()
              selecting.select(rows.map((r) => r.id))
            }}
          >
            <ListChecks className="size-3.5 text-muted" /> Select all {rows.length}
          </MenuItem>
        </Popover>
      </div>
      {!folded && (
        <ul className="flex flex-col">
          {rows.map((r) => (
            <SessionLink key={r.id} row={r} selecting={selecting} />
          ))}
        </ul>
      )}
    </div>
  )
}

/** Shown while sessions are selected, in place of the account: delete them, or stop selecting. */
function SelectionBar({ rows, onDone }: { rows: SessionRow[]; onDone: () => void }) {
  const [confirm, setConfirm] = useState(false)
  const [working, setWorking] = useState(false)
  useEffect(() => setConfirm(false), [rows.length])
  const n = rows.length
  return (
    <div className="mx-2 mt-2 mb-3 flex flex-col gap-2.5 rounded-xl bg-surface px-3 py-3">
      {confirm ? (
        <p className="text-dense">
          Delete {n} session{n === 1 ? "" : "s"}? Their history is removed from this computer.
        </p>
      ) : (
        <div className="text-dense flex items-center justify-between">
          <span>
            {n} selected
          </span>
          <span className="text-2xs text-muted">Shift click for a range</span>
        </div>
      )}
      <div className="flex gap-2">
        {confirm ? (
          <button
            disabled={working}
            onClick={() => {
              setWorking(true)
              void deleteSessions(rows).finally(() => {
                setWorking(false)
                onDone()
              })
            }}
            className={btn("danger-soft", "sm", "gap-1.5")}
          >
            {working ? <Spinner className="size-3.5" /> : <Trash2 className="size-3.5" />}
            Delete {n}
          </button>
        ) : (
          <button onClick={() => setConfirm(true)} className={btn("danger-soft", "sm", "gap-1.5")}>
            <Trash2 className="size-3.5" />
            Delete
          </button>
        )}
        <button onClick={confirm ? () => setConfirm(false) : onDone} className={btn("ghost", "sm", "gap-1.5")}>
          {confirm ? "Cancel" : "Done"}
          {!confirm && <Kbd>Esc</Kbd>}
        </button>
      </div>
    </div>
  )
}

function SessionLink({
  row,
  selecting,
  showPlace = false,
}: {
  row: SessionRow
  selecting: Selecting
  /** Pinned rows name their project, which no group header does for them. */
  showPlace?: boolean
}) {
  const active = useStore((s) => s.open?.sessionId === row.id)
  const busy = useStore((s) => s.busy.has(row.id))
  const pinned = useStore((s) => s.pinned.has(row.id))
  const asking = useStore(
    (s) => s.permissions.some((p) => p.rootId === row.id) || s.questions.some((q) => q.rootId === row.id)
  )
  const [menu, setMenu] = useState<"closed" | "open" | "confirm">("closed")
  const [renaming, setRenaming] = useState(false)
  const close = useCallback(() => setMenu("closed"), [])
  const choosing = selecting.selected.size > 0
  const checked = selecting.selected.has(row.id)

  if (renaming)
    return (
      <li>
        <input
          autoFocus
          defaultValue={row.title}
          aria-label="Session name"
          onFocus={(e) => e.currentTarget.select()}
          onBlur={(e) => {
            setRenaming(false)
            if (e.currentTarget.value.trim() && e.currentTarget.value !== row.title)
              void renameSession(row, e.currentTarget.value)
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter") e.currentTarget.blur()
            if (e.key === "Escape") {
              e.currentTarget.value = row.title
              e.currentTarget.blur()
            }
          }}
          className="h-9 w-full rounded-lg bg-default px-2 text-sm outline-none focus-visible:status-focused-field"
        />
      </li>
    )

  return (
    <li className="group relative">
      <button
        onClick={(e) => selecting.pick(row, e)}
        aria-current={active ? "page" : undefined}
        aria-pressed={choosing ? checked : undefined}
        className={cn(
          "flex w-full items-center gap-2 rounded-lg px-2 text-left transition-colors focus-visible:status-focused-field",
          showPlace ? "min-h-11 py-1.5" : "h-9",
          checked ? "bg-default" : active && !choosing ? "bg-default" : "hover:bg-default/60"
        )}
      >
        {choosing && (
          <span
            className={cn(
              "flex size-4 shrink-0 items-center justify-center rounded border",
              checked ? "border-foreground bg-foreground text-background" : "border-muted/60"
            )}
          >
            {checked && <Check className="size-3" strokeWidth={3} />}
          </span>
        )}
        <span className="min-w-0 flex-1">
          <span className={cn("block truncate text-sm", !active && "text-foreground/85")}>{row.title}</span>
          {showPlace && (
            <span className="text-2xs block truncate text-muted">
              <PlaceLabel directory={row.directory} />
            </span>
          )}
        </span>
        {asking ? (
          <span className="size-1.5 shrink-0 rounded-full bg-warning" aria-label="Needs you" />
        ) : (
          busy && <Spinner className="size-3" />
        )}
        <span
          className={cn(
            "text-2xs w-12 shrink-0 text-right text-muted tabular-nums",
            !choosing && "group-hover:invisible"
          )}
        >
          {ago(row.updated)}
        </span>
      </button>
      {!choosing && (
        <button
          onClick={() => setMenu(menu === "closed" ? "open" : "closed")}
          aria-label={`Actions for ${row.title}`}
          className={cn(
            "absolute top-1.5 right-1.5 flex size-6 items-center justify-center rounded-md text-muted hover:bg-default-hover hover:text-foreground focus-visible:status-focused-field",
            menu === "closed" && "invisible group-hover:visible focus-visible:visible"
          )}
        >
          <Ellipsis className="size-4" />
        </button>
      )}
      <Popover open={menu !== "closed"} onClose={close} side="bottom" align="end" className="w-56">
        {menu === "open" ? (
          <>
            <MenuItem
              onSelect={() => {
                close()
                togglePin(row.id)
              }}
            >
              {pinned ? <PinOff className="size-3.5 text-muted" /> : <Pin className="size-3.5 text-muted" />}
              {pinned ? "Unpin" : "Pin"}
            </MenuItem>
            <MenuItem
              onSelect={() => {
                close()
                setRenaming(true)
              }}
            >
              <Pencil className="size-3.5 text-muted" /> Rename
            </MenuItem>
            <MenuItem
              onSelect={() => {
                close()
                selecting.select([row.id])
              }}
            >
              <ListChecks className="size-3.5 text-muted" /> Select
            </MenuItem>
            <MenuItem danger onSelect={() => setMenu("confirm")}>
              <Trash2 className="size-3.5" /> Delete
            </MenuItem>
          </>
        ) : (
          <div className="flex flex-col gap-3 p-2">
            <p className="text-dense">Delete this session? Its history is removed from this computer.</p>
            <div className="flex gap-2">
              <button
                onClick={() => {
                  close()
                  void deleteSessions([row])
                }}
                className={btn("danger-soft", "sm")}
              >
                Delete
              </button>
              <button onClick={close} className={btn("ghost", "sm")}>
                Cancel
              </button>
            </div>
          </div>
        )}
      </Popover>
    </li>
  )
}

function UpdateRow() {
  const update = useStore((s) => s.update)
  if (!update?.available) return null
  return (
    <div className="px-2 pt-2">
      <button
        onClick={() => void installUpdate()}
        disabled={update.installing}
        className={btn("secondary", "sm", "w-full justify-start gap-2")}
      >
        {update.installing ? <Spinner className="size-3.5" /> : <ArrowDownToLine className="size-3.5" />}
        {update.installing ? `Installing ${update.version}…` : `Update to ${update.version}`}
      </button>
      {update.error && <p className="text-2xs px-2 pt-1.5 text-danger">{update.error}</p>}
    </div>
  )
}

function AccountRow() {
  const account = useStore((s) => s.account)
  const [open, setOpen] = useState(false)
  const close = useCallback(() => setOpen(false), [])
  if (!account || account.expired) return null
  return (
    <div className="relative px-2 pt-2 pb-3">
      <button
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className="flex w-full flex-col gap-2 rounded-lg px-2 py-2 text-left hover:bg-default/60 focus-visible:status-focused-field"
      >
        <span className="flex items-center justify-between text-sm">
          {account.plan ? `${capitalize(account.plan)} plan` : "Anysola"}
          <ChevronUp className="size-3.5 text-muted" />
        </span>
        {!account.included && <span className="text-dense text-warning">This plan doesn’t include Anysola Code.</span>}
        <WindowMeter label="5-hour" window={account.fiveHour} />
        <WindowMeter label="Weekly" window={account.weekly} />
      </button>
      <Popover open={open} onClose={close} side="top" className="left-2 w-60">
        <DefaultFolder onDone={close} />
        <div className="my-1 border-t border-separator" />
        <MenuItem
          onSelect={() => {
            close()
            void signOut()
          }}
        >
          <LogOut className="size-3.5 text-muted" /> Sign out
        </MenuItem>
      </Popover>
    </div>
  )
}

/** Where new sessions start (Ctrl N); without one, each asks for a folder. */
function DefaultFolder({ onDone }: { onDone: () => void }) {
  const preferred = useStore((s) => s.defaultFolder)
  return (
    <div className="flex flex-col">
      <div className="px-2.5 pt-1.5 pb-1">
        <div className="text-2xs text-muted">New sessions start in</div>
        <div className="truncate text-sm" title={preferred ?? undefined}>
          {preferred ? folderName(preferred) : "A folder you pick each time"}
        </div>
      </div>
      <MenuItem
        onSelect={() => {
          onDone()
          void pickFolder().then((d) => d && setDefaultFolder(d))
        }}
      >
        <FolderOpen className="size-3.5 text-muted" /> {preferred ? "Change default folder…" : "Set a default folder…"}
      </MenuItem>
      {preferred && (
        <MenuItem
          onSelect={() => {
            onDone()
            setDefaultFolder(null)
          }}
        >
          <X className="size-3.5 text-muted" /> Ask each time
        </MenuItem>
      )}
    </div>
  )
}

function WindowMeter({ label, window: w }: { label: string; window: AccountWindow | null }) {
  if (!w) return null
  const resets = resetsIn(w.resetsAt)
  return (
    <span className="flex flex-col gap-1" title={resets ? `Starts over ${resets}` : undefined}>
      <span className="text-2xs flex justify-between text-muted">
        <span>{label}</span>
        <span className="tabular-nums">
          {w.percent}%{resets && <span className="text-muted/70"> · {resets}</span>}
        </span>
      </span>
      <span className="block h-1 overflow-hidden rounded-full bg-default">
        <span
          className={cn("block h-full rounded-full", w.percent >= 90 ? "bg-warning" : "bg-foreground/70")}
          style={{ width: `${Math.min(100, w.percent)}%` }}
        />
      </span>
    </span>
  )
}

