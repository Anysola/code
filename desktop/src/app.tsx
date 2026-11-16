import {
  ArrowDownToLine,
  ArrowUp,
  Check,
  ChevronDown,
  ChevronUp,
  Ellipsis,
  FolderClosed,
  FolderOpen,
  LogOut,
  Paperclip,
  Pencil,
  Plus,
  Search,
  Trash2,
  X,
} from "lucide-react"
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"

import type { Attachment, FileDiff, SessionRow, Window as AccountWindow } from "./bridge"
import {
  chooseModel,
  deleteSession,
  dismissNotice,
  installUpdate,
  newSession,
  openLoginPage,
  renameSession,
  signOut,
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

function Sidebar() {
  const sessions = useStore((s) => s.sessions)
  const [query, setQuery] = useState("")
  const search = useRef<HTMLInputElement>(null)
  const groups = useMemo(() => {
    const q = query.trim().toLowerCase()
    const by = new Map<string, SessionRow[]>()
    for (const s of sessions) {
      if (q && !s.title.toLowerCase().includes(q) && !folderName(s.directory).toLowerCase().includes(q)) continue
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
        <button onClick={() => void newSession()} className={btn("secondary", "md", "w-full justify-start gap-2")}>
          <Plus className="size-4" />
          <span className="flex-1 text-left">New session</span>
          <Kbd>Ctrl N</Kbd>
        </button>
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
      <UpdateRow />
      <AccountRow />
    </aside>
  )
}

function SessionLink({ row }: { row: SessionRow }) {
  const active = useStore((s) => s.open?.sessionId === row.id)
  const busy = useStore((s) => s.busy.has(row.id))
  const asking = useStore(
    (s) => s.permissions.some((p) => p.sessionId === row.id) || s.questions.some((q) => q.sessionId === row.id)
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
      <Popover open={open} onClose={close} side="top" className="w-60">
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
  const asked = useStore((s) => s.questions)
  const questions = useMemo(() => asked.filter((q) => q.sessionId === open.sessionId), [asked, open.sessionId])
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

const modes: { id: Mode; label: string; hint: string }[] = [
  { id: "ask", label: "Ask", hint: "Ask before every edit and command" },
  { id: "edits", label: "Edits", hint: "Edit files without asking; ask before commands" },
  { id: "auto", label: "Auto", hint: "Run everything without asking" },
]

/** Images only, as the model takes them; larger ones are refused before they are read. */
const MAX_IMAGE_BYTES = 5 * 1024 * 1024

function readImage(file: File): Promise<Attachment> {
  return new Promise((resolve, reject) => {
    if (!file.type.startsWith("image/")) return reject(new Error(`${file.name} isn’t an image.`))
    if (file.size > MAX_IMAGE_BYTES) return reject(new Error(`${file.name} is over 5 MB.`))
    const reader = new FileReader()
    reader.onload = () => {
      const url = String(reader.result)
      resolve({ mime: file.type, data: url.slice(url.indexOf(",") + 1), name: file.name || "image.png" })
    }
    reader.onerror = () => reject(new Error(`Couldn’t read ${file.name}.`))
    reader.readAsDataURL(file)
  })
}

function Composer({ open }: { open: Open }) {
  const mode = useStore((s) => s.mode)
  const asking = useStore((s) => s.permissions.find((p) => p.sessionId === open.sessionId) ?? null)
  const [text, setText] = useState("")
  const [files, setFiles] = useState<Attachment[]>([])
  const [problem, setProblem] = useState<string | null>(null)
  const [dragging, setDragging] = useState(false)
  const area = useRef<HTMLTextAreaElement>(null)
  const picker = useRef<HTMLInputElement>(null)
  const running = open.status.kind === "working" || open.status.kind === "retrying"
  const model = open.info?.models.find((m) => m.id === open.info?.modelId)
  const empty = !text.trim() && files.length === 0

  useEffect(() => area.current?.focus(), [open.sessionId])
  useLayoutEffect(() => {
    const el = area.current
    if (!el) return
    el.style.height = "auto"
    el.style.height = `${Math.min(el.scrollHeight, 240)}px`
  }, [text])

  const add = async (list: Iterable<File>) => {
    setProblem(null)
    for (const file of list) {
      try {
        const image = await readImage(file)
        setFiles((all) => [...all, image])
      } catch (e) {
        setProblem(e instanceof Error ? e.message : String(e))
      }
    }
  }

  const submit = () => {
    if (empty) return
    void send(text, files)
    setText("")
    setFiles([])
    setProblem(null)
  }

  return (
    <div
      onDragOver={(e) => {
        if (e.dataTransfer.types.includes("Files")) {
          e.preventDefault()
          setDragging(true)
        }
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={(e) => {
        e.preventDefault()
        setDragging(false)
        void add(e.dataTransfer.files)
      }}
      className={cn("rounded-[1.25rem] bg-surface p-2 transition-colors", dragging && "bg-default")}
    >
      {files.length > 0 && (
        <div className="flex flex-wrap gap-2 px-1.5 pt-1 pb-2">
          {files.map((f, i) => (
            <div key={i} className="group relative">
              <img
                src={`data:${f.mime};base64,${f.data}`}
                alt={f.name}
                className="size-14 rounded-lg object-cover"
              />
              <button
                onClick={() => setFiles((all) => all.filter((_, j) => j !== i))}
                aria-label={`Remove ${f.name}`}
                className="absolute -top-1.5 -right-1.5 flex size-5 items-center justify-center rounded-full bg-popover text-muted opacity-0 group-hover:opacity-100 hover:text-foreground focus-visible:opacity-100"
              >
                <X className="size-3" />
              </button>
            </div>
          ))}
        </div>
      )}
      <textarea
        ref={area}
        value={text}
        rows={2}
        onChange={(e) => setText(e.target.value)}
        onPaste={(e) => {
          const images = [...e.clipboardData.files].filter((f) => f.type.startsWith("image/"))
          if (images.length) {
            e.preventDefault()
            void add(images)
          }
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault()
            if (empty && asking) void reply(asking, "once")
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
      {(problem || (files.length > 0 && model && !model.images)) && (
        <p className="text-dense px-2.5 pb-2 text-warning">{problem ?? `${model?.name} doesn’t read images.`}</p>
      )}
      <div className="flex items-center gap-1.5">
        <button
          onClick={() => picker.current?.click()}
          className={btn("ghost", "sm", "text-muted", true)}
          aria-label="Attach images"
          title="Attach images"
        >
          <Paperclip className="size-4" strokeWidth={1.75} />
        </button>
        <input
          ref={picker}
          type="file"
          accept="image/*"
          multiple
          hidden
          onChange={(e) => {
            if (e.target.files) void add(e.target.files)
            e.target.value = ""
          }}
        />
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
          <ModelPicker open={open} />
          {running && empty ? (
            <button onClick={stop} className={btn("secondary", "sm", "", true)} aria-label="Stop">
              <span className="size-2.5 rounded-[2px] bg-current" />
            </button>
          ) : (
            <button onClick={submit} disabled={empty} className={btn("primary", "sm", "", true)} aria-label="Send">
              <ArrowUp className="size-4" />
            </button>
          )}
        </div>
      </div>
    </div>
  )
}

const efforts: Record<string, string> = { xhigh: "Extra high" }

function ModelPicker({ open }: { open: Open }) {
  const [show, setShow] = useState(false)
  const close = useCallback(() => setShow(false), [])
  const info = open.info
  if (!info?.model) return null
  const current = info.models.find((m) => m.id === info.modelId)
  return (
    <div className="relative">
      <button
        onClick={() => setShow(!show)}
        aria-expanded={show}
        className="text-dense flex h-8 items-center gap-1.5 rounded-full px-3 text-muted hover:bg-default/60 hover:text-foreground focus-visible:status-focused-field"
      >
        {info.model}
        {info.variant && <span className="text-muted/70">· {efforts[info.variant] ?? capitalize(info.variant)}</span>}
        <ChevronDown className="size-3.5" />
      </button>
      <Popover open={show} onClose={close} side="top" align="end" className="w-72">
        <div role="listbox" aria-label="Model" className="flex flex-col">
          {info.models.map((m) => (
            <button
              key={m.id}
              role="option"
              aria-selected={m.id === info.modelId}
              onClick={() => {
                // The effort carries over when the new model has it.
                const variant = info.variant && m.efforts.includes(info.variant) ? info.variant : null
                void chooseModel(m.id, variant)
              }}
              className="flex items-start gap-2.5 rounded-lg px-2.5 py-2 text-left hover:bg-default-hover focus-visible:status-focused-field"
            >
              <span className="min-w-0 flex-1">
                <span className="block text-sm">{m.name}</span>
                {m.note && <span className="text-2xs block text-muted">{m.note}</span>}
              </span>
              {m.id === info.modelId && <Check className="mt-0.5 size-3.5 shrink-0" />}
            </button>
          ))}
        </div>
        {current && current.efforts.length > 0 && (
          <div className="mt-1 border-t border-separator px-2.5 pt-2.5 pb-1.5">
            <div className="text-2xs pb-2 text-muted">Effort</div>
            <div className="flex flex-wrap gap-1" role="radiogroup" aria-label="Effort">
              {[null, ...current.efforts].map((e) => (
                <button
                  key={e ?? "default"}
                  role="radio"
                  aria-checked={info.variant === e}
                  onClick={() => void chooseModel(current.id, e)}
                  className={cn(
                    "text-2xs h-7 rounded-full px-2.5 transition-colors focus-visible:status-focused-field",
                    info.variant === e ? "bg-default-hover text-foreground" : "text-muted hover:bg-default/60"
                  )}
                >
                  {e ? (efforts[e] ?? capitalize(e)) : "Default"}
                </button>
              ))}
            </div>
          </div>
        )}
      </Popover>
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
