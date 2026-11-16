import {
  ArrowDownToLine,
  ChevronUp,
  Ellipsis,
  FolderClosed,
  FolderOpen,
  LogOut,
  MessageSquare,
  Pencil,
  Plus,
  Search,
  Trash2,
  X,
} from "lucide-react"
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"

import type { FileDiff, SessionRow, Window as AccountWindow } from "./bridge"
import { capitalize, Composer, Tasks } from "./composer"
import {
  closePeek,
  deleteSession,
  dismissNotice,
  installUpdate,
  newScratchSession,
  newSession,
  openLoginPage,
  pickFolder,
  renameSession,
  setDefaultFolder,
  signOut,
  openSession,
  signIn,
  start,
  useStore,
  type Open,
} from "./store"
import { EditCard, Transcript } from "./transcript"
import {
  ago,
  btn,
  cn,
  Counts,
  folderName,
  Kbd,
  Mark,
  MenuItem,
  Popover,
  resetsIn,
  Spinner,
  TitleBar,
  UsageMeter,
} from "./ui"

export function App() {
  const auth = useStore((s) => s.auth)
  useEffect(() => {
    void start()
  }, [])
  useShortcuts()
  if (auth === "checking") return <div className="h-full bg-background" />
  if (auth === "signed-out") return <SignIn />
  return (
    <div className="flex h-full bg-background">
      <Sidebar />
      <Main />
      <Notice />
    </div>
  )
}

function useShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "n") {
        e.preventDefault()
        void newSession()
      }
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [])
}

// ---------------------------------------------------------------------------
// Sign-in
// ---------------------------------------------------------------------------

function SignIn() {
  const login = useStore((s) => s.login)
  const error = useStore((s) => s.loginError)
  const [starting, setStarting] = useState(false)
  return (
    <div className="flex h-full flex-col bg-background">
      <TitleBar controls />
      <main className="flex flex-1 flex-col items-center justify-center gap-8 pb-16">
        <Mark className="h-7 text-foreground" />
        <h1 className="sr-only">Sign in to Anysola Code</h1>
        {login ? (
          <div className="flex flex-col items-center gap-5">
            <div className="flex flex-col items-center gap-2">
              <span className="text-2xs tracking-wide text-muted">Code</span>
              <span className="font-mono text-3xl tracking-[0.2em] select-all">{login.code}</span>
            </div>
            <div className="text-dense flex items-center gap-2 text-muted">
              <Spinner />
              Waiting for approval in your browser
            </div>
            <button onClick={openLoginPage} className={btn("ghost", "sm", "text-muted")}>
              Open the page again
            </button>
          </div>
        ) : (
          <div className="flex flex-col items-center gap-4">
            <button
              onClick={() => {
                setStarting(true)
                void signIn().finally(() => setStarting(false))
              }}
              disabled={starting}
              className={btn("primary", "md", "min-w-56")}
            >
              {starting ? "Starting…" : "Sign in with Anysola"}
            </button>
            {error && <p className="text-dense max-w-sm text-center text-danger">{error}</p>}
          </div>
        )}
      </main>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Sidebar
// ---------------------------------------------------------------------------

/** A session's folder as the window names it: the app's own folder is "No project". */
function usePlace(directory: string) {
  const scratch = useStore((s) => s.scratch)
  return directory === scratch ? "No project" : folderName(directory)
}

function PlaceLabel({ directory }: { directory: string }) {
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

function Sidebar() {
  const sessions = useStore((s) => s.sessions)
  const [query, setQuery] = useState("")
  const search = useRef<HTMLInputElement>(null)
  const groups = useMemo(() => {
    const q = query.trim().toLowerCase()
    const by = new Map<string, SessionRow[]>()
    for (const s of sessions) {
      if (q && !s.title.toLowerCase().includes(q) && !s.directory.toLowerCase().includes(q)) continue
      by.set(s.directory, [...(by.get(s.directory) ?? []), s])
    }
    return [...by.entries()]
  }, [sessions, query])
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault()
        search.current?.focus()
      }
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [])
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
        {sessions.length > 0 && groups.length === 0 && (
          <p className="text-dense px-2 text-muted">No session matches “{query.trim()}”.</p>
        )}
        {groups.map(([directory, rows]) => (
          <div key={directory}>
            <div className="text-2xs flex items-center gap-1.5 px-2 pb-1 font-medium tracking-wide text-muted" title={directory}>
              <FolderClosed className="size-3" strokeWidth={1.75} />
              <PlaceLabel directory={directory} />
            </div>
            <ul className="flex flex-col">
              {rows.map((r) => (
                <SessionLink key={r.id} row={r} />
              ))}
            </ul>
          </div>
        ))}
      </nav>
      <UpdateRow />
      <AccountRow />
    </aside>
  )
}

function SessionLink({ row }: { row: SessionRow }) {
  const active = useStore((s) => s.open?.sessionId === row.id)
  const busy = useStore((s) => s.busy.has(row.id))
  const asking = useStore(
    (s) => s.permissions.some((p) => p.rootId === row.id) || s.questions.some((q) => q.rootId === row.id)
  )
  const [menu, setMenu] = useState<"closed" | "open" | "confirm">("closed")
  const [renaming, setRenaming] = useState(false)
  const close = useCallback(() => setMenu("closed"), [])

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
        onClick={() => void openSession(row)}
        aria-current={active ? "page" : undefined}
        className={cn(
          "flex h-9 w-full items-center gap-2 rounded-lg px-2 text-left transition-colors focus-visible:status-focused-field",
          active ? "bg-default" : "hover:bg-default/60"
        )}
      >
        <span className={cn("min-w-0 flex-1 truncate text-sm", !active && "text-foreground/85")}>{row.title}</span>
        {asking ? (
          <span className="size-1.5 shrink-0 rounded-full bg-warning" aria-label="Needs you" />
        ) : (
          busy && <Spinner className="size-3" />
        )}
        <span className="text-2xs w-12 shrink-0 text-right text-muted tabular-nums group-hover:invisible">
          {ago(row.updated)}
        </span>
      </button>
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
      <Popover open={menu !== "closed"} onClose={close} side="bottom" align="end" className="w-56">
        {menu === "open" ? (
          <>
            <MenuItem
              onSelect={() => {
                close()
                setRenaming(true)
              }}
            >
              <Pencil className="size-3.5 text-muted" /> Rename
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
                  void deleteSession(row)
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

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

function Main() {
  const open = useStore((s) => s.open)
  const [reviewing, setReviewing] = useState(false)
  useEffect(() => setReviewing(false), [open?.sessionId])
  return (
    <main className="relative m-2 ml-0 flex min-w-0 flex-1 flex-col overflow-hidden rounded-2xl bg-surface/40">
      {open ? (
        <>
          <SessionHeader open={open} onReview={() => setReviewing(true)} />
          <Conversation open={open} />
          <div className="mx-auto w-full max-w-[50rem] px-8 pb-5">
            <Tasks todos={open.todos} />
            <Composer open={open} />
            <div className="mt-2 flex items-center gap-3 px-2">
              <span className="text-2xs flex min-w-0 items-center gap-1 text-muted" title={open.directory}>
                <FolderClosed className="size-3 shrink-0" strokeWidth={1.75} />
                <span className="truncate">
                  <FooterPlace directory={open.directory} />
                </span>
              </span>
              <UsageMeter percent={open.info?.fiveHour ?? null} className="ml-auto w-44 shrink-0" />
            </div>
          </div>
          {reviewing && <Review open={open} onClose={() => setReviewing(false)} />}
          <PeekSheet />
        </>
      ) : (
        <>
          <TitleBar controls />
          <div className="flex flex-1 flex-col items-center justify-center gap-2 pb-16">
            <button onClick={() => void newSession()} className={btn("secondary", "md", "min-w-60 gap-2")}>
              <FolderOpen className="size-4" />
              <span className="flex-1 text-left">Open a folder</span>
              <Kbd>Ctrl N</Kbd>
            </button>
            <button onClick={() => void newScratchSession()} className={btn("ghost", "md", "min-w-60 gap-2 text-muted")}>
              <MessageSquare className="size-4" />
              <span className="flex-1 text-left">Start without a project</span>
            </button>
          </div>
        </>
      )}
    </main>
  )
}

/** Every file the session changed, with the edits' line counts summed per file. */
function changedFiles(open: Open): { file: string; diffs: FileDiff[]; add: number; del: number }[] {
  const by = new Map<string, FileDiff[]>()
  for (const i of open.items)
    if (i.kind === "tool" && i.status === "completed")
      for (const f of i.files ?? []) by.set(f.file, [...(by.get(f.file) ?? []), f])
  return [...by.entries()].map(([file, diffs]) => ({
    file,
    diffs,
    add: diffs.reduce((n, d) => n + d.additions, 0),
    del: diffs.reduce((n, d) => n + d.deletions, 0),
  }))
}

function FooterPlace({ directory }: { directory: string }) {
  const scratch = useStore((s) => s.scratch)
  return <>{directory === scratch ? "No project · runs in the app’s own folder" : directory}</>
}

function SessionHeader({ open, onReview }: { open: Open; onReview: () => void }) {
  const files = changedFiles(open)
  const add = files.reduce((n, f) => n + f.add, 0)
  const del = files.reduce((n, f) => n + f.del, 0)
  return (
    <TitleBar className="pl-6" controls>
      <div data-tauri-drag-region className="flex min-w-0 items-center gap-3">
        <h1 className="min-w-0 truncate text-sm font-medium">{open.title}</h1>
        <span className="text-2xs flex shrink-0 items-center gap-1 text-muted">
          <FolderClosed className="size-3" strokeWidth={1.75} />
          <PlaceLabel directory={open.directory} />
        </span>
      </div>
      {files.length > 0 && (
        <button onClick={onReview} className={btn("ghost", "sm", "mr-2 ml-auto shrink-0 gap-2 text-muted")}>
          <Counts add={add} del={del} />
          Review changes
        </button>
      )}
    </TitleBar>
  )
}

function Conversation({ open }: { open: Open }) {
  const all = useStore((s) => s.permissions)
  const permissions = useMemo(() => all.filter((p) => p.rootId === open.sessionId), [all, open.sessionId])
  const asked = useStore((s) => s.questions)
  const questions = useMemo(() => asked.filter((q) => q.rootId === open.sessionId), [asked, open.sessionId])
  const ref = useRef<HTMLDivElement>(null)
  const stick = useRef(true)
  // A chat opens at its latest turn and follows new output while the reader is at the bottom.
  useLayoutEffect(() => {
    const el = ref.current
    if (el && stick.current) el.scrollTop = el.scrollHeight
  })
  useEffect(() => {
    stick.current = true
  }, [open.sessionId])
  // The composer grows (lines, images) and the area shrinks: stay at the bottom if there.
  useEffect(() => {
    const el = ref.current
    if (!el) return
    const watch = new ResizeObserver(() => {
      if (stick.current) el.scrollTop = el.scrollHeight
    })
    watch.observe(el)
    return () => watch.disconnect()
  }, [])
  return (
    <div
      ref={ref}
      onScroll={(e) => {
        const el = e.currentTarget
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48
      }}
      className="min-h-0 flex-1 overflow-y-auto"
    >
      <div className="mx-auto max-w-[50rem] px-8 pt-6 pb-8">
        {open.loading ? (
          <div className="flex justify-center pt-16">
            <Spinner />
          </div>
        ) : (
          <Transcript open={open} permissions={permissions} questions={questions} />
        )}
      </div>
    </div>
  )
}

function Review({ open, onClose }: { open: Open; onClose: () => void }) {
  const files = changedFiles(open)
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose()
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [onClose])
  return (
    <div className="absolute inset-0 z-10 flex justify-end bg-black/40" onClick={onClose}>
      <aside
        className="flex h-full w-[min(42rem,92%)] flex-col bg-overlay"
        onClick={(e) => e.stopPropagation()}
        aria-label="Changes"
      >
        <div className="flex h-12 shrink-0 items-center gap-3 px-5">
          <span className="text-sm font-medium">Changes</span>
          <span className="text-2xs text-muted">
            {files.length} file{files.length === 1 ? "" : "s"}
          </span>
          <button onClick={onClose} className={btn("ghost", "sm", "ml-auto text-muted", true)} aria-label="Close">
            <X className="size-4" />
          </button>
        </div>
        <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-4 pb-6">
          {files.map((f) => (
            <div key={f.file} className="flex flex-col gap-2">
              {f.diffs.map((d, i) => (
                <EditCard key={i} file={d} />
              ))}
            </div>
          ))}
        </div>
      </aside>
    </div>
  )
}

/** A subagent's transcript, read-only, over the session that ran it. */
function PeekSheet() {
  const peek = useStore((s) => s.peek)
  useEffect(() => {
    if (!peek) return
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && closePeek()
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [peek])
  if (!peek) return null
  const view: Open = {
    directory: peek.directory,
    sessionId: peek.sessionId,
    title: peek.title,
    items: peek.items,
    live: [],
    info: null,
    status: { kind: "idle" },
    loading: peek.loading,
    revert: null,
    todos: [],
    lastAgent: null,
    fill: null,
  }
  return (
    <div className="absolute inset-0 z-10 flex justify-end bg-black/40" onClick={closePeek}>
      <aside
        className="flex h-full w-[min(44rem,92%)] flex-col bg-overlay"
        onClick={(e) => e.stopPropagation()}
        aria-label="Subagent"
      >
        <div className="flex h-12 shrink-0 items-center gap-3 px-5">
          <span className="text-2xs text-muted">Subagent</span>
          <span className="min-w-0 truncate text-sm font-medium">{peek.title}</span>
          <button onClick={closePeek} className={btn("ghost", "sm", "ml-auto text-muted", true)} aria-label="Close">
            <X className="size-4" />
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-6 pb-8">
          {peek.loading ? (
            <div className="flex justify-center pt-16">
              <Spinner />
            </div>
          ) : (
            <Transcript open={view} permissions={[]} questions={[]} readOnly />
          )}
        </div>
      </aside>
    </div>
  )
}

function Notice() {
  const notice = useStore((s) => s.notice)
  if (!notice) return null
  return (
    <div className="text-dense fixed right-6 bottom-6 z-20 flex max-w-sm items-start gap-3 rounded-xl bg-popover px-4 py-3">
      <span className="flex-1">{notice}</span>
      <button onClick={dismissNotice} className="text-muted hover:text-foreground" aria-label="Dismiss">
        <X className="size-4" />
      </button>
    </div>
  )
}
