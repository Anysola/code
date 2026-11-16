import { ArrowUp, FolderClosed, FolderOpen, Plus, X } from "lucide-react"
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"

import type { FileDiff, SessionRow } from "./bridge"
import {
  dismissNotice,
  newSession,
  openLoginPage,
  openSession,
  reply,
  send,
  setMode,
  signIn,
  start,
  stop,
  useStore,
  type Mode,
  type Open,
} from "./store"
import { EditCard, Transcript } from "./transcript"
import { ago, btn, cn, Counts, folderName, Kbd, Mark, Spinner, TitleBar, UsageMeter } from "./ui"

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

function Sidebar() {
  const sessions = useStore((s) => s.sessions)
  const fiveHour = useStore((s) => s.open?.info?.fiveHour ?? null)
  const groups = useMemo(() => {
    const by = new Map<string, SessionRow[]>()
    for (const s of sessions) by.set(s.directory, [...(by.get(s.directory) ?? []), s])
    return [...by.entries()]
  }, [sessions])
  return (
    <aside className="flex w-64 shrink-0 flex-col">
      <TitleBar>
        <div data-tauri-drag-region className="flex items-center gap-2 px-4 text-sm font-medium whitespace-nowrap">
          <Mark />
          Anysola Code
        </div>
      </TitleBar>
      <div className="px-2 pt-2 pb-4">
        <button onClick={() => void newSession()} className={btn("secondary", "md", "w-full justify-start gap-2")}>
          <Plus className="size-4" />
          <span className="flex-1 text-left">New session</span>
          <Kbd>Ctrl N</Kbd>
        </button>
      </div>
      <nav className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-2 pb-3" aria-label="Sessions">
        {groups.length === 0 && <p className="text-dense px-2 text-muted">No sessions yet.</p>}
        {groups.map(([directory, rows]) => (
          <div key={directory}>
            <div className="text-2xs flex items-center gap-1.5 px-2 pb-1 font-medium tracking-wide text-muted" title={directory}>
              <FolderClosed className="size-3" strokeWidth={1.75} />
              {folderName(directory)}
            </div>
            <ul className="flex flex-col">
              {rows.map((r) => (
                <SessionLink key={r.id} row={r} />
              ))}
            </ul>
          </div>
        ))}
      </nav>
      {fiveHour !== null && <UsageMeter percent={fiveHour} className="px-4 pt-2 pb-4" />}
    </aside>
  )
}

function SessionLink({ row }: { row: SessionRow }) {
  const active = useStore((s) => s.open?.sessionId === row.id)
  const busy = useStore((s) => s.busy.has(row.id))
  const asking = useStore((s) => s.permissions.some((p) => p.sessionId === row.id))
  return (
    <li>
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
          <span className="size-1.5 shrink-0 rounded-full bg-warning" aria-label="Needs approval" />
        ) : (
          busy && <Spinner className="size-3" />
        )}
        <span className="text-2xs shrink-0 text-muted tabular-nums">{ago(row.updated)}</span>
      </button>
    </li>
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
            <Composer open={open} />
            <div className="mt-2 flex items-center gap-3 px-2">
              <span className="text-2xs flex min-w-0 items-center gap-1 text-muted" title={open.directory}>
                <FolderClosed className="size-3 shrink-0" strokeWidth={1.75} />
                <span className="truncate">{open.directory}</span>
              </span>
              <UsageMeter percent={open.info?.fiveHour ?? null} className="ml-auto w-44 shrink-0" />
            </div>
          </div>
          {reviewing && <Review open={open} onClose={() => setReviewing(false)} />}
        </>
      ) : (
        <>
          <TitleBar controls />
          <div className="flex flex-1 items-center justify-center pb-16">
            <button onClick={() => void newSession()} className={btn("secondary", "md", "gap-2")}>
              <FolderOpen className="size-4" />
              Open a folder
              <Kbd>Ctrl N</Kbd>
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
          {folderName(open.directory)}
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
  const permissions = useMemo(() => all.filter((p) => p.sessionId === open.sessionId), [all, open.sessionId])
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
          <Transcript open={open} permissions={permissions} />
        )}
      </div>
    </div>
  )
}

const modes: { id: Mode; label: string; hint: string }[] = [
  { id: "ask", label: "Ask", hint: "Ask before every edit and command" },
  { id: "edits", label: "Edits", hint: "Edit files without asking; ask before commands" },
  { id: "auto", label: "Auto", hint: "Run everything without asking" },
]

function Composer({ open }: { open: Open }) {
  const mode = useStore((s) => s.mode)
  const asking = useStore((s) => s.permissions.find((p) => p.sessionId === open.sessionId) ?? null)
  const [text, setText] = useState("")
  const area = useRef<HTMLTextAreaElement>(null)
  const running = open.status.kind === "working" || open.status.kind === "retrying"

  useEffect(() => area.current?.focus(), [open.sessionId])
  useLayoutEffect(() => {
    const el = area.current
    if (!el) return
    el.style.height = "auto"
    el.style.height = `${Math.min(el.scrollHeight, 240)}px`
  }, [text])

  const submit = () => {
    if (!text.trim()) return
    void send(text)
    setText("")
  }

  return (
    <div className="rounded-[1.25rem] bg-surface p-2">
      <textarea
        ref={area}
        value={text}
        rows={2}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault()
            if (!text.trim() && asking) void reply(asking, "once")
            else submit()
          } else if (e.key === "Escape") {
            if (asking) void reply(asking, "reject")
            else if (running) stop()
          }
        }}
        placeholder={asking ? "Reply, or press Enter to allow" : "Ask for a change"}
        aria-label="Message"
        className="text-md block min-h-14 w-full resize-none bg-transparent px-2.5 pt-1.5 leading-relaxed outline-none placeholder:text-field-placeholder"
      />
      <div className="flex items-center gap-1.5">
        <div className="flex h-8 items-center rounded-full bg-default/60 p-0.5" role="radiogroup" aria-label="Permissions">
          {modes.map((m) => (
            <button
              key={m.id}
              role="radio"
              aria-checked={mode === m.id}
              title={m.hint}
              onClick={() => setMode(m.id)}
              className={cn(
                "text-dense h-7 rounded-full px-3 transition-colors focus-visible:status-focused-field",
                mode === m.id ? "bg-default-hover text-foreground" : "text-muted hover:text-foreground"
              )}
            >
              {m.label}
            </button>
          ))}
        </div>
        <div className="ml-auto flex items-center gap-2">
          {open.info?.model && (
            <span className="text-dense px-2 text-muted">
              {open.info.model}
              {open.info.variant && <span className="text-muted/70"> · {capitalize(open.info.variant)}</span>}
            </span>
          )}
          {running && !text.trim() ? (
            <button onClick={stop} className={btn("secondary", "sm", "", true)} aria-label="Stop">
              <span className="size-2.5 rounded-[2px] bg-current" />
            </button>
          ) : (
            <button onClick={submit} disabled={!text.trim()} className={btn("primary", "sm", "", true)} aria-label="Send">
              <ArrowUp className="size-4" />
            </button>
          )}
        </div>
      </div>
    </div>
  )
}

function capitalize(s: string) {
  return s.charAt(0).toUpperCase() + s.slice(1)
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
