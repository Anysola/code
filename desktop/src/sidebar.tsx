/**
 * The sidebar: new session, Search (the palette), pinned sessions, sessions by project (each project folds
 * shut and has its own menu), selecting several to delete at once, updates and the account.
 */
import {
  ArrowDownToLine,
  ArrowUpRight,
  Check,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  CircleHelp,
  Ellipsis,
  FileDown,
  FileInput,
  FolderClosed,
  FolderOpen,
  GitBranch,
  Gauge,
  Keyboard,
  ListChecks,
  LogOut,
  MessageSquare,
  MessageSquareText,
  Pencil,
  Pin,
  PinOff,
  Plus,
  Settings as SettingsIcon,
  Trash2,
  UserRound,
} from "lucide-react"
import { useCallback, useEffect, useMemo, useRef, useState } from "react"

import type { SessionRow, Window as AccountWindow } from "./bridge"
import { askDelete, FEEDBACK, flushDeletes, HELP, RELEASES, showShortcuts, useChrome } from "./chrome"
import { capitalize } from "./composer"
import { exportSession, SidebarToggle } from "./header"
import { SearchButton } from "./palette"
import { KEYS } from "./shortcuts"
import { usePrefs } from "./prefs"
import {
  importSession,
  installUpdate,
  openExternal,
  newScratchSession,
  newSession,
  openSession,
  openSettings,
  pickFolder,
  renameSession,
  reveal,
  signOut,
  toggleCollapsed,
  togglePin,
  useStore,
} from "./store"
import {
  ago,
  anyLayer,
  btn,
  cn,
  folderKey,
  folderName,
  Kbd,
  Mark,
  MenuItem,
  MenuSeparator,
  Popover,
  resetsIn,
  sameFolder,
  Spinner,
  TitleBar,
  Tooltip,
} from "./ui"
import { BranchName, worktreeTrouble } from "./worktree"

/** A session's folder as the window names it: the app's own folder is "No project". */
function usePlace(directory: string) {
  const scratch = useStore((s) => s.scratch)
  return sameFolder(directory, scratch) ? "No project" : folderName(directory)
}

export function PlaceLabel({ directory }: { directory: string }) {
  return <>{usePlace(directory)}</>
}

/** The folders of the latest sessions, newest first: not the app's own nor the default folder. */
export function useRecentFolders(limit = 4) {
  const sessions = useStore((s) => s.sessions)
  const scratch = useStore((s) => s.scratch)
  const preferred = usePrefs((p) => p.general.defaultFolder)
  return useMemo(() => {
    const seen = new Map<string, string>()
    for (const s of sessions) {
      const key = folderKey(s.directory)
      if (sameFolder(s.directory, scratch) || sameFolder(s.directory, preferred) || seen.has(key)) continue
      seen.set(key, s.directory)
      if (seen.size === limit) break
    }
    return [...seen.values()]
  }, [sessions, scratch, preferred, limit])
}

/** New session: the default folder, a recent one, one to pick, or none. */
function NewSessionButton() {
  const preferred = usePrefs((p) => p.general.defaultFolder)
  const [show, setShow] = useState(false)
  const close = useCallback(() => setShow(false), [])
  const recent = useRecentFolders()
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
      <Popover open={show} onClose={close} side="bottom" align="start" menu label="New session" matchWidth className="min-w-60">
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
        {(preferred || recent.length > 0) && <MenuSeparator />}
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
        <MenuSeparator />
        <MenuItem onSelect={() => go(() => importSession())}>
          <FileInput className="size-3.5 text-muted" /> Import session…
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
  const all = useStore((s) => s.sessions)
  const deleting = useChrome((c) => c.deleting)
  // Sessions waiting behind Undo are gone from the list already.
  const sessions = useMemo(() => (deleting ? all.filter((s) => !deleting.ids.has(s.id)) : all), [all, deleting])
  const pinned = useStore((s) => s.pinned)
  const collapsed = useStore((s) => s.collapsed)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const anchor = useRef<string | null>(null)
  const pins = useMemo(() => sessions.filter((s) => pinned.has(s.id)), [sessions, pinned])
  const groups = useMemo(() => {
    // By folder however spelled; the group shows the first spelling.
    const by = new Map<string, [string, SessionRow[]]>()
    for (const s of sessions) {
      if (pinned.has(s.id)) continue
      const key = folderKey(s.directory)
      const group = by.get(key)
      if (group) group[1].push(s)
      else by.set(key, [s.directory, [s]])
    }
    return [...by.values()]
  }, [sessions, pinned])
  // The rows in the order they show, for Shift ranges (folded projects show none).
  const order = useMemo(
    () => [...pins, ...groups.flatMap(([d, rows]) => (collapsed.has(folderKey(d)) ? [] : rows))],
    [pins, groups, collapsed]
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
      // Under Settings, a panel or a menu, Escape is theirs. Taken (marked) before the window
      // hears it, where two presses stop a running turn: this one only clears the selection.
      if (e.key === "Escape" && !e.defaultPrevented && selected.size > 0 && !anyLayer()) {
        e.preventDefault()
        setSelected(new Set())
      }
    }
    document.addEventListener("keydown", onKey)
    return () => document.removeEventListener("keydown", onKey)
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
        <div className="flex items-center gap-2 px-4 text-sm font-medium whitespace-nowrap">
          <Mark />
          Anysola Code
        </div>
        <div className="mr-1.5 ml-auto">
          <SidebarToggle hidden={false} />
        </div>
      </TitleBar>
      <div className="px-2 pt-2 pb-4">
        <NewSessionButton />
        {sessions.length > 0 && <SearchButton />}
      </div>
      <nav className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-2 pb-3" aria-label="Sessions">
        {sessions.length === 0 && <p className="text-dense px-2 text-muted">No sessions yet.</p>}
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
            key={folderKey(directory)}
            directory={directory}
            rows={rows}
            folded={collapsed.has(folderKey(directory))}
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
          <SettingsRow />
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
        <Tooltip label="Project actions">
          <button
            onClick={() => setMenu(!menu)}
            aria-label={`Actions for ${folderName(directory)}`}
            aria-haspopup="menu"
            aria-expanded={menu}
            className={cn(
              "absolute top-0.5 right-1.5 flex size-6 items-center justify-center rounded-md text-muted hover:bg-default-hover hover:text-foreground focus-visible:status-focused-field",
              !menu && "invisible group-hover/project:visible focus-visible:visible"
            )}
          >
            <Ellipsis className="size-4" />
          </button>
        </Tooltip>
        <Popover open={menu} onClose={close} side="bottom" align="end" menu label={`Actions for ${folderName(directory)}`} className="w-60">
          <MenuItem
            onSelect={() => {
              close()
              void newSession(directory)
            }}
          >
            <Plus className="size-3.5 text-muted" /> New session here
          </MenuItem>
          <MenuItem
            onSelect={() => {
              close()
              void importSession(directory)
            }}
          >
            <FileInput className="size-3.5 text-muted" /> Import session here…
          </MenuItem>
          {!sameFolder(directory, scratch) && (
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
  const openId = useStore((s) => s.open?.sessionId ?? null)
  useEffect(() => setConfirm(false), [rows.length])
  const n = rows.length
  return (
    <div className="mx-2 mt-2 mb-3 flex flex-col gap-2.5 rounded-xl bg-surface px-3 py-3">
      {confirm ? (
        <p className="text-dense">
          Delete {n} session{n === 1 ? "" : "s"}? Their history is removed from this computer; you can undo for a few seconds.
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
            onClick={() => {
              void askDelete(rows, openId)
              onDone()
            }}
            className={btn("danger-soft", "sm", "gap-1.5")}
          >
            <Trash2 className="size-3.5" />
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
  // A form belongs to a folder, not a session: the folder's running sessions may be the ones waiting.
  const asking = useStore(
    (s) =>
      s.permissions.some((p) => p.rootId === row.id) ||
      s.questions.some((q) => q.rootId === row.id) ||
      (s.busy.has(row.id) && s.forms.some((f) => sameFolder(f.directory, row.directory)))
  )
  const [menu, setMenu] = useState(false)
  const openId = useStore((s) => s.open?.sessionId ?? null)
  const scratch = useStore((s) => s.scratch)
  const [renaming, setRenaming] = useState(false)
  const close = useCallback(() => setMenu(false), [])
  const go = (f: () => void) => () => {
    close()
    f()
  }
  const choosing = selecting.selected.size > 0
  const checked = selecting.selected.has(row.id)
  const trouble = worktreeTrouble(row.worktree)

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
              // Leaves the field only: no press toward stopping a turn.
              e.preventDefault()
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
          showPlace || row.worktree ? "min-h-11 py-1.5" : "h-9",
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
          <span className={cn("flex min-w-0 items-center gap-1.5 text-sm", !active && "text-foreground/85")}>
            {row.forkOf && (
              <GitBranch className="size-3.5 shrink-0 text-muted" strokeWidth={1.75} aria-label="Branched from another session" />
            )}
            <span className="min-w-0 truncate">{row.title}</span>
          </span>
          {(showPlace || row.worktree) && (
            <span className="text-2xs flex min-w-0 items-center gap-1.5 text-muted">
              {/* Both give way on a narrow row: the folder up to half, the branch takes the rest. */}
              {showPlace && (
                <span className={cn("min-w-0 truncate", row.worktree ? "max-w-[45%] shrink-0" : "shrink")}>
                  <PlaceLabel directory={row.directory} />
                </span>
              )}
              {row.worktree && <BranchName worktree={row.worktree} className="flex-1" />}
            </span>
          )}
        </span>
        {asking ? (
          <span className="size-1.5 shrink-0 rounded-full bg-warning" aria-label="Needs you" />
        ) : busy ? (
          <Spinner className="size-3" />
        ) : (
          trouble && <span className="size-1.5 shrink-0 rounded-full bg-warning/70" aria-label={trouble} title={trouble} />
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
        <Tooltip label="Session actions">
          <button
            onClick={() => setMenu(!menu)}
            aria-label={`Actions for ${row.title}`}
            aria-haspopup="menu"
            aria-expanded={menu}
            className={cn(
              "absolute top-1.5 right-1.5 flex size-6 items-center justify-center rounded-md text-muted hover:bg-default-hover hover:text-foreground focus-visible:status-focused-field",
              !menu && "invisible group-hover:visible focus-visible:visible"
            )}
          >
            <Ellipsis className="size-4" />
          </button>
        </Tooltip>
      )}
      <Popover open={menu} onClose={close} side="bottom" align="end" menu label={`Actions for ${row.title}`} className="w-56">
        <MenuItem onSelect={go(() => togglePin(row.id))}>
          {pinned ? <PinOff className="size-3.5 text-muted" /> : <Pin className="size-3.5 text-muted" />}
          {pinned ? "Unpin" : "Pin"}
        </MenuItem>
        <MenuItem onSelect={go(() => setRenaming(true))}>
          <Pencil className="size-3.5 text-muted" /> Rename
        </MenuItem>
        {!sameFolder(row.directory, scratch) && (
          <MenuItem onSelect={go(() => void reveal(row.worktree?.path ?? row.directory))}>
            <FolderOpen className="size-3.5 text-muted" /> Open folder
          </MenuItem>
        )}
        <MenuItem onSelect={go(() => void exportSession(row.directory, row.id))}>
          <FileDown className="size-3.5 text-muted" /> Export…
        </MenuItem>
        <MenuItem onSelect={go(() => selecting.select([row.id]))}>
          <ListChecks className="size-3.5 text-muted" /> Select
        </MenuItem>
        <MenuSeparator />
        <MenuItem danger onSelect={go(() => void askDelete([row], openId))}>
          <Trash2 className="size-3.5" /> Delete
        </MenuItem>
      </Popover>
    </li>
  )
}

function UpdateRow() {
  const update = useStore((s) => s.update)
  const [notes, setNotes] = useState(false)
  const close = useCallback(() => setNotes(false), [])
  if (!update?.available) return null
  return (
    <div className="px-2 pt-2">
      <div className="relative flex items-center gap-1">
        <button
          onClick={() => void installUpdate()}
          disabled={update.installing || update.waiting}
          className={btn("secondary", "sm", "min-w-0 flex-1 justify-start gap-2")}
        >
          {update.installing || update.waiting ? <Spinner className="size-3.5" /> : <ArrowDownToLine className="size-3.5" />}
          <span className="truncate">
            {update.installing
              ? update.progress?.total
                ? `Downloading ${update.version} · ${Math.min(100, Math.round((update.progress.downloaded / update.progress.total) * 100))}%`
                : `Installing ${update.version}…`
              : update.waiting
                ? "Finishes after the running turn"
                : `Update to ${update.version}`}
          </span>
        </button>
        <button
          onClick={() => (update.notes ? setNotes(!notes) : openExternal(RELEASES))}
          aria-expanded={update.notes ? notes : undefined}
          className={btn("ghost", "sm", "shrink-0 text-muted")}
        >
          What’s new
        </button>
        {update.notes && (
          <Popover open={notes} onClose={close} side="top" align="start" label={`What’s new in ${update.version}`} matchWidth className="w-72 p-3">
            <div className="flex flex-col gap-2">
              <span className="text-sm font-medium">What’s new in {update.version}</span>
              <p className="text-dense whitespace-pre-wrap text-muted select-text">{update.notes.trim()}</p>
              <button
                onClick={() => {
                  close()
                  openExternal(RELEASES)
                }}
                className={btn("ghost", "sm", "-ml-2 gap-1 self-start text-muted")}
              >
                All releases
                <ArrowUpRight className="size-3.5" />
              </button>
            </div>
          </Popover>
        )}
      </div>
      {update.waiting && !update.installing && (
        <p className="text-2xs flex items-center gap-1 px-2 pt-1.5 text-muted">
          <button onClick={() => void installUpdate(true)} className="text-foreground/80 underline-offset-2 hover:underline">
            Restart now
          </button>
        </p>
      )}
      {update.error && <p className="text-2xs px-2 pt-1.5 text-danger">{update.error}</p>}
    </div>
  )
}

function SettingsRow() {
  return (
    <div className="px-2 pt-2">
      <button
        onClick={() => openSettings("general")}
        className="text-dense flex h-8 w-full items-center gap-2 rounded-lg px-2 text-left text-muted hover:bg-default/60 hover:text-foreground focus-visible:status-focused-field"
      >
        <SettingsIcon className="size-3.5 shrink-0" strokeWidth={1.75} />
        <span className="flex-1">Settings</span>
        <Kbd>Ctrl ,</Kbd>
      </button>
    </div>
  )
}

/** The plan and its windows: the name opens the account menu, the meters open Usage. */
function AccountRow() {
  const account = useStore((s) => s.account)
  const [open, setOpen] = useState(false)
  const close = useCallback(() => setOpen(false), [])
  if (!account || account.expired) return null
  const go = (section: "account" | "usage" | "general") => {
    close()
    openSettings(section)
  }
  const link = (url: string) => () => {
    close()
    openExternal(url)
  }
  const meters = account.fiveHour || account.weekly
  return (
    <div className="px-2 pt-1 pb-3">
      <div className="relative">
        <button
          onClick={() => setOpen(!open)}
          aria-expanded={open}
          aria-haspopup="menu"
          className="flex h-8 w-full items-center gap-2 rounded-lg px-2 text-left text-sm hover:bg-default/60 focus-visible:status-focused-field"
        >
          <span className="min-w-0 flex-1 truncate">{account.plan ? `${capitalize(account.plan)} plan` : "Anysola"}</span>
          {meters && (
            <span
              // A short window: the meters fold into this row so the session list keeps its room.
              className="text-2xs hidden shrink-0 text-muted tabular-nums [@media(max-height:720px)]:inline"
              title={[
                account.fiveHour && `5-hour ${account.fiveHour.percent}%`,
                account.weekly && `weekly ${account.weekly.percent}%`,
              ]
                .filter(Boolean)
                .join(", ")}
            >
              {[account.fiveHour && `5h ${account.fiveHour.percent}%`, account.weekly && `wk ${account.weekly.percent}%`]
                .filter(Boolean)
                .join(" · ")}
            </span>
          )}
          <ChevronUp className="size-3.5 shrink-0 text-muted" />
        </button>
        <Popover open={open} onClose={close} side="top" menu label="Account" className="w-60">
          <MenuItem onSelect={() => go("account")}>
            <UserRound className="size-3.5 text-muted" /> Account and devices
          </MenuItem>
          <MenuItem onSelect={() => go("usage")}>
            <Gauge className="size-3.5 text-muted" /> Usage and limits
          </MenuItem>
          <MenuItem onSelect={() => go("general")}>
            <SettingsIcon className="size-3.5 text-muted" />
            <span className="flex-1">Settings</span>
            <Kbd>{KEYS.settings}</Kbd>
          </MenuItem>
          <MenuSeparator />
          <MenuItem onSelect={link(HELP)}>
            <CircleHelp className="size-3.5 text-muted" />
            <span className="flex-1">Help and docs</span>
            <ArrowUpRight className="size-3.5 text-muted" />
          </MenuItem>
          <MenuItem onSelect={link(FEEDBACK)}>
            <MessageSquareText className="size-3.5 text-muted" />
            <span className="flex-1">Send feedback</span>
            <ArrowUpRight className="size-3.5 text-muted" />
          </MenuItem>
          <MenuItem
            onSelect={() => {
              close()
              showShortcuts(true)
            }}
          >
            <Keyboard className="size-3.5 text-muted" />
            <span className="flex-1">Keyboard shortcuts</span>
            <Kbd>{KEYS.shortcuts}</Kbd>
          </MenuItem>
          <MenuSeparator />
          <MenuItem
            onSelect={() => {
              close()
              // A delete waiting behind Undo goes through before the sessions are put away.
              void flushDeletes().then(signOut)
            }}
          >
            <LogOut className="size-3.5 text-muted" /> Sign out
          </MenuItem>
        </Popover>
      </div>
      {!account.included && <p className="text-dense px-2 pb-1 text-warning">This plan doesn’t include Anysola Code.</p>}
      {meters && (
        <button
          onClick={() => go("usage")}
          aria-label="Usage and limits"
          className="flex w-full flex-col gap-2 rounded-lg px-2 pt-1 pb-2 text-left hover:bg-default/60 focus-visible:status-focused-field [@media(max-height:720px)]:hidden"
        >
          <WindowMeter label="5-hour" window={account.fiveHour} />
          <WindowMeter label="Weekly" window={account.weekly} />
        </button>
      )}
    </div>
  )
}

function WindowMeter({ label, window: w }: { label: string; window: AccountWindow | null }) {
  if (!w) return null
  const resets = resetsIn(w.resetsAt)
  return (
    <span className="flex flex-col gap-1" title={resets ? `Resets ${resets}` : undefined}>
      <span className="text-2xs flex justify-between text-muted">
        <span>{label}</span>
        <span className="tabular-nums">
          {w.percent}%{resets && <span> · {resets}</span>}
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

