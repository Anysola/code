/**
 * The composer: the message box with its `/` and `@` menus, images, the permission mode (and
 * Plan), the model, and the session's task list above it.
 */
import {
  ArrowUp,
  Check,
  ChevronDown,
  ChevronUp,
  Circle,
  CircleCheck,
  FileText,
  Folder,
  LoaderCircle,
  MessageSquareText,
  Paperclip,
  Sparkles,
  SquareSlash,
  X,
} from "lucide-react"
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"

import type { Attachment, Palette, Todo } from "./bridge"
import {
  chooseAgent,
  chooseModel,
  compact,
  newSession,
  palette as readPalette,
  projectFiles,
  redo,
  removeComment,
  reply,
  retry,
  runCommand,
  send,
  setMode,
  stop,
  undo,
  useStore,
  type Mode,
  type Open,
} from "./store"
import { btn, cn, Popover } from "./ui"

const modes: { id: Mode | "plan"; label: string; hint: string }[] = [
  { id: "ask", label: "Ask", hint: "Ask before every edit and command" },
  { id: "edits", label: "Edits", hint: "Edit files without asking; ask before commands" },
  { id: "auto", label: "Auto", hint: "Ask only where your rules say to" },
  { id: "plan", label: "Plan", hint: "Read and plan; change nothing until you approve" },
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

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------

type Row = {
  key: string
  label: string
  detail: string | null
  kind: "action" | "command" | "skill" | "file" | "folder"
  /** Text that replaces the typed `/…` or `@…`. */
  insert: string
  /** Runs at once instead of inserting (the window's own commands). */
  run?: () => void
}

/** The window's own commands; the folder's commands (`/init`, `/review`, its own) follow. */
function actions(open: Open): Row[] {
  const rows: Row[] = [
    { name: "undo", detail: "Rewind the last turn and put its message back here", run: () => void undo() },
    { name: "retry", detail: "Send the last message again", run: () => void retry() },
    { name: "compact", detail: "Summarize the session so far to free up context", run: () => void compact() },
    { name: "new", detail: "Start a new session in this folder", run: () => void newSession(open.directory) },
  ].map((a) => ({ key: `a:${a.name}`, label: `/${a.name}`, detail: a.detail, kind: "action", insert: "", run: a.run }))
  if (open.revert)
    rows.splice(1, 0, {
      key: "a:redo",
      label: "/redo",
      detail: "Put back what the rewind undid",
      kind: "action",
      insert: "",
      run: () => void redo(),
    })
  for (const agent of open.info?.agents ?? [])
    if (agent.id === "plan" || agent.id === "build")
      rows.push({
        key: `a:${agent.id}`,
        label: `/${agent.id}`,
        detail: agent.id === "plan" ? "Plan before changing anything" : "Make changes (the default agent)",
        kind: "action",
        insert: "",
        run: () => void chooseAgent(agent.id),
      })
  return rows
}

const ACTION_NAMES = new Set(["undo", "redo", "retry", "compact", "new", "plan", "build"])

/** Prefix matches first, then matches inside, then letters in order (fuzzy). */
function rank(text: string, query: string): number {
  if (!query) return 1
  const t = text.toLowerCase()
  const q = query.toLowerCase()
  if (t.startsWith(q)) return 4
  const base = t.slice(t.lastIndexOf("/", t.length - 2) + 1)
  if (base.startsWith(q)) return 3
  if (t.includes(q)) return 2
  let at = 0
  for (const c of q) {
    at = t.indexOf(c, at)
    if (at < 0) return 0
    at++
  }
  return 1
}

function filter(rows: Row[], query: string, by: (r: Row) => string, limit: number) {
  return rows
    .map((r, i) => ({ r, i, score: rank(by(r), query) }))
    .filter((x) => x.score > 0)
    // Without a query the list keeps its order: the window's commands, then the folder's.
    .sort((a, b) => (query ? b.score - a.score || by(a.r).length - by(b.r).length : 0) || a.i - b.i)
    .slice(0, limit)
    .map((x) => x.r)
}

/** What the caret is typing: a command at the start of the message, or an `@` mention. */
function trigger(text: string, caret: number): { kind: "/" | "@"; query: string; start: number } | null {
  const before = text.slice(0, caret)
  const slash = /^\/(\S*)$/.exec(before)
  if (slash) return { kind: "/", query: slash[1], start: 0 }
  const at = /(^|\s)@(\S*)$/.exec(before)
  if (at) return { kind: "@", query: at[2], start: before.length - at[2].length - 1 }
  return null
}

const ICON = { action: SquareSlash, command: SquareSlash, skill: Sparkles, file: FileText, folder: Folder }

function Menu({
  rows,
  active,
  onPick,
  onHover,
  empty,
}: {
  rows: Row[]
  active: number
  onPick: (r: Row) => void
  onHover: (i: number) => void
  empty: string | null
}) {
  const list = useRef<HTMLDivElement>(null)
  useEffect(() => {
    list.current?.querySelector(`[data-i="${active}"]`)?.scrollIntoView({ block: "nearest" })
  }, [active])
  return (
    <div
      ref={list}
      role="listbox"
      aria-label="Suggestions"
      className="absolute right-0 bottom-full left-0 z-30 mb-2 max-h-72 overflow-y-auto rounded-xl bg-popover p-1.5"
    >
      {rows.length === 0 && empty && <p className="text-dense px-2.5 py-2 text-muted">{empty}</p>}
      {rows.map((r, i) => {
        const Icon = ICON[r.kind]
        return (
          <button
            key={r.key}
            data-i={i}
            role="option"
            aria-selected={i === active}
            // Keeps the caret in the message box.
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => onPick(r)}
            onMouseMove={() => onHover(i)}
            className={cn(
              "flex h-9 w-full items-center gap-2.5 rounded-lg px-2.5 text-left",
              i === active && "bg-default-hover"
            )}
          >
            <Icon className="size-3.5 shrink-0 text-muted" strokeWidth={1.75} />
            <span className={cn("shrink-0 text-sm", (r.kind === "file" || r.kind === "folder") && "font-mono text-dense")}>
              {r.label}
            </span>
            {r.detail && <span className="text-2xs min-w-0 flex-1 truncate text-muted">{r.detail}</span>}
            {r.kind === "skill" && <span className="text-2xs ml-auto shrink-0 text-muted">Skill</span>}
          </button>
        )
      })}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Composer
// ---------------------------------------------------------------------------

export function Composer({ open }: { open: Open }) {
  const mode = useStore((s) => s.mode)
  const asking = useStore((s) => s.permissions.find((p) => p.rootId === open.sessionId) ?? null)
  const [text, setText] = useState("")
  const [caret, setCaret] = useState(0)
  const [files, setFiles] = useState<Attachment[]>([])
  const [problem, setProblem] = useState<string | null>(null)
  const [dragging, setDragging] = useState(false)
  const [pal, setPal] = useState<Palette | null>(null)
  const [paths, setPaths] = useState<string[] | null>(null)
  const [active, setActive] = useState(0)
  /** The trigger the user closed with Escape, until they type elsewhere. */
  const [dismissed, setDismissed] = useState<number | null>(null)
  const area = useRef<HTMLTextAreaElement>(null)
  const picker = useRef<HTMLInputElement>(null)
  const running = open.status.kind === "working" || open.status.kind === "retrying"
  const model = open.info?.models.find((m) => m.id === open.info?.modelId)
  const comments = useStore((s) => s.comments)
  const empty = !text.trim() && files.length === 0 && comments.length === 0
  const planning = open.info?.agent === "plan"
  const hasPlan = open.info?.agents.some((a) => a.id === "plan") ?? false

  useEffect(() => area.current?.focus(), [open.sessionId])
  // A rewind puts its message back here.
  const fill = open.fill
  const current = useRef(text)
  current.current = text
  useEffect(() => {
    if (!fill) return
    if (fill.replacing !== undefined && current.current !== fill.replacing) return
    setText(fill.text)
    setCaret(fill.text.length)
    const back = fill.files
    if (back?.length) setFiles((all) => (all.length ? all : back))
    requestAnimationFrame(() => {
      const el = area.current
      if (!el) return
      el.focus()
      el.setSelectionRange(fill.text.length, fill.text.length)
    })
  }, [fill])
  useLayoutEffect(() => {
    const el = area.current
    if (!el) return
    el.style.height = "auto"
    el.style.height = `${Math.min(el.scrollHeight, 240)}px`
  }, [text])
  // An answer for the folder open before a switch doesn't land in this one's menus.
  const folder = useRef(open.directory)
  folder.current = open.directory
  // Commands and skills of this folder; read again when the menu opens (MCP prompts come late).
  const loadPalette = useCallback(() => {
    const directory = open.directory
    void readPalette(directory)
      .then((p) => folder.current === directory && setPal(p))
      .catch(() => {})
  }, [open.directory])
  useEffect(() => {
    setPal(null)
    setPaths(null)
    loadPalette()
  }, [loadPalette])

  const typed = trigger(text, caret)
  const menuOpen = typed !== null && dismissed !== typed.start
  useEffect(() => {
    if (!typed) return
    if (typed.kind === "/" ) loadPalette()
    // Read on every open: the agent may have added files (the store keeps them 30 s).
    if (typed.kind === "@") {
      const directory = open.directory
      void projectFiles(directory)
        .then((list) => folder.current === directory && setPaths(list))
        .catch(() => folder.current === directory && setPaths([]))
    }
    // Only when a menu opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [typed?.kind, typed?.start])

  const rows = useMemo<Row[]>(() => {
    if (!typed || !menuOpen) return []
    if (typed.kind === "/") {
      const server: Row[] = (pal?.commands ?? [])
        .filter((c) => !ACTION_NAMES.has(c.name))
        .map((c) => ({
          key: `c:${c.name}`,
          label: `/${c.name}`,
          detail: c.description,
          kind: "command",
          insert: `/${c.name} `,
        }))
      const skills: Row[] = (pal?.skills ?? []).map((s) => ({
        key: `s:${s.name}`,
        label: `/${s.name}`,
        detail: s.description,
        kind: "skill",
        insert: `@${s.name} `,
      }))
      return filter([...actions(open), ...server, ...skills], typed.query, (r) => r.label.slice(1), 50)
    }
    const skills: Row[] = (pal?.skills ?? []).map((s) => ({
      key: `s:${s.name}`,
      label: `@${s.name}`,
      detail: s.description,
      kind: "skill",
      insert: `@${s.name} `,
    }))
    const found: Row[] = (paths ?? []).map((p) => ({
      key: `f:${p}`,
      label: p,
      detail: null,
      kind: p.endsWith("/") ? "folder" : "file",
      insert: p.endsWith("/") ? `@${p}` : `@${p} `,
    }))
    if (!typed.query) return [...skills, ...found.filter((f) => !f.label.slice(0, -1).includes("/"))].slice(0, 50)
    return [
      ...filter(skills, typed.query, (r) => r.label.slice(1), 6),
      ...filter(found, typed.query, (r) => r.label, 40),
    ]
  }, [typed?.kind, typed?.query, menuOpen, pal, paths, open])
  useEffect(() => setActive(0), [typed?.kind, typed?.query])

  const pick = (r: Row) => {
    if (!typed) return
    if (r.run) {
      setText("")
      setCaret(0)
      r.run()
      return
    }
    const next = text.slice(0, typed.start) + r.insert + text.slice(caret)
    const at = typed.start + r.insert.length
    setText(next)
    setCaret(at)
    requestAnimationFrame(() => area.current?.setSelectionRange(at, at))
  }

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

  const clear = () => {
    setText("")
    setCaret(0)
    setFiles([])
    setProblem(null)
  }

  const submit = () => {
    if (empty) return
    const command = /^\/(\S+)(?:\s+([\s\S]*))?$/.exec(text.trim())
    if (command && files.length === 0) {
      const [, name, args = ""] = command
      const own = actions(open).find((a) => a.label === `/${name}`)
      if (own?.run) {
        clear()
        own.run()
        return
      }
      if (pal?.commands.some((c) => c.name === name)) {
        if (running) {
          setProblem(`/${name} can run once this turn ends, or after you stop it.`)
          return
        }
        clear()
        void runCommand(name, args.trim())
        return
      }
    }
    const skills = (pal?.skills ?? [])
      .map((s) => s.name)
      .filter((name) => new RegExp(`(^|\\s)@${name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}(?=\\s|$)`).test(text))
    void send(text, files, skills)
    clear()
  }

  const shown = open.info?.agent === "plan" ? "plan" : mode

  return (
    <div className="relative">
      {menuOpen && (rows.length > 0 || typed?.kind === "@") && (
        <Menu
          rows={rows}
          active={Math.min(active, Math.max(rows.length - 1, 0))}
          onPick={pick}
          onHover={setActive}
          empty={paths === null ? "Reading files…" : `Nothing matches “${typed?.query}”`}
        />
      )}
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
        {comments.length > 0 && (
          <div className="flex flex-col gap-1 px-1.5 pt-1 pb-2">
            {comments.map((c) => (
              <div key={c.id} className="text-dense flex items-center gap-2 rounded-lg bg-default/50 py-1 pr-1 pl-2.5">
                <MessageSquareText className="size-3.5 shrink-0 text-muted" />
                <span className="shrink-0 font-mono text-xs text-muted">
                  {c.file.split("/").pop()}:{c.line}
                </span>
                <span className="min-w-0 flex-1 truncate">{c.text}</span>
                <button
                  onClick={() => removeComment(c.id)}
                  aria-label="Remove comment"
                  className="flex size-6 items-center justify-center rounded-md text-muted hover:text-foreground"
                >
                  <X className="size-3" />
                </button>
              </div>
            ))}
          </div>
        )}
        {files.length > 0 && (
          <div className="flex flex-wrap gap-2 px-1.5 pt-1 pb-2">
            {files.map((f, i) => (
              <div key={i} className="group relative">
                <img src={`data:${f.mime};base64,${f.data}`} alt={f.name} className="size-14 rounded-lg object-cover" />
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
          onChange={(e) => {
            setText(e.target.value)
            setCaret(e.target.selectionStart)
            setDismissed(null)
          }}
          onSelect={(e) => setCaret(e.currentTarget.selectionStart)}
          onPaste={(e) => {
            const images = [...e.clipboardData.files].filter((f) => f.type.startsWith("image/"))
            if (images.length) {
              e.preventDefault()
              void add(images)
            }
          }}
          onKeyDown={(e) => {
            if (menuOpen && rows.length > 0) {
              if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                e.preventDefault()
                const step = e.key === "ArrowDown" ? 1 : -1
                setActive((a) => (a + step + rows.length) % rows.length)
                return
              }
              if ((e.key === "Enter" && !e.shiftKey) || e.key === "Tab") {
                e.preventDefault()
                pick(rows[Math.min(active, rows.length - 1)])
                return
              }
            }
            if (menuOpen && e.key === "Escape" && typed) {
              e.preventDefault()
              setDismissed(typed.start)
              return
            }
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault()
              if (empty && asking) void reply(asking, "once")
              else submit()
            } else if (e.key === "Escape") {
              if (asking) void reply(asking, "reject")
              else if (running) stop()
            }
          }}
          placeholder={
            asking
              ? "Reply, or press Enter to allow"
              : planning
                ? "Describe what to plan"
                : "Ask for a change, / for commands, @ for files"
          }
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
          <div className="flex h-8 items-center rounded-full bg-default/60 p-0.5" role="radiogroup" aria-label="Mode">
            {modes
              .filter((m) => m.id !== "plan" || hasPlan)
              .map((m) => (
                <button
                  key={m.id}
                  role="radio"
                  aria-checked={shown === m.id}
                  title={m.hint}
                  onClick={() => {
                    if (m.id === "plan") return void chooseAgent("plan")
                    setMode(m.id)
                    if (planning) void chooseAgent("build")
                  }}
                  className={cn(
                    "text-dense h-7 rounded-full px-3 transition-colors focus-visible:status-focused-field",
                    shown === m.id ? "bg-default-hover text-foreground" : "text-muted hover:text-foreground"
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
    </div>
  )
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

const efforts: Record<string, string> = { xhigh: "Extra high" }

export function capitalize(s: string) {
  return s.charAt(0).toUpperCase() + s.slice(1)
}

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

// ---------------------------------------------------------------------------
// Tasks
// ---------------------------------------------------------------------------

/** The session's todo list, folded to the task under way; gone once every task is done. */
export function Tasks({ todos }: { todos: Todo[] }) {
  const [open, setOpen] = useState(false)
  const done = todos.filter((t) => t.status === "completed").length
  const left = todos.filter((t) => t.status !== "completed" && t.status !== "cancelled")
  if (left.length === 0) return null
  const current = todos.find((t) => t.status === "in_progress") ?? left[0]
  return (
    <div className="mb-2 rounded-xl bg-surface/70 px-1.5 py-1">
      <button
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className="text-dense flex h-8 w-full items-center gap-2.5 rounded-lg px-2 text-left hover:bg-default/40 focus-visible:status-focused-field"
      >
        {open ? (
          <span className="min-w-0 flex-1 text-muted">Tasks</span>
        ) : (
          <>
            <TaskIcon status={current.status} />
            <span className="min-w-0 flex-1 truncate">{current.content}</span>
          </>
        )}
        <span className="text-2xs shrink-0 text-muted tabular-nums">
          {done} of {todos.length}
        </span>
        {open ? <ChevronDown className="size-3.5 text-muted" /> : <ChevronUp className="size-3.5 text-muted" />}
      </button>
      {open && (
        <ul className="flex flex-col pb-1">
          {todos.map((t, i) => (
            <li key={i} className="text-dense flex items-start gap-2.5 px-2 py-1">
              <TaskIcon status={t.status} />
              <span
                className={cn(
                  "min-w-0 flex-1",
                  t.status === "completed" && "text-muted line-through decoration-muted/60",
                  t.status === "cancelled" && "text-muted"
                )}
              >
                {t.content}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

function TaskIcon({ status }: { status: string }) {
  if (status === "completed") return <CircleCheck className="mt-px size-3.5 shrink-0 text-muted" />
  if (status === "in_progress") return <LoaderCircle className="mt-px size-3.5 shrink-0 animate-spin text-foreground/80 [animation-duration:2s]" />
  return <Circle className="mt-px size-3.5 shrink-0 text-muted/70" />
}
