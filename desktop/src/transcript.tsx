import {
  ChevronDown,
  ChevronRight,
  CircleAlert,
  FilePlus2,
  FileText,
  Globe,
  ListTodo,
  MessageCircleQuestion,
  Pencil,
  Search,
  SquareTerminal,
  Wrench,
  Bot,
  Check,
  Copy,
  Hammer,
  Redo2,
  RotateCcw,
  SquareSlash,
  Undo2,
} from "lucide-react"
import { useRef, useState, type ReactNode } from "react"
import Markdown from "react-markdown"
import remarkGfm from "remark-gfm"

import type { FileDiff, Item } from "./bridge"
import {
  answer,
  buildPlan,
  openExternal,
  peekAt,
  redo,
  reply,
  resend,
  retry,
  rewind,
  useStore,
  type Asked,
  type Open,
  type Permission,
  type Status,
} from "./store"
import { btn, cn, Counts, DiffView, Kbd, Spinner } from "./ui"

type Tool = Extract<Item, { kind: "tool" }>
type UserItem = Extract<Item, { kind: "user" }>

/** Lookups that only read: shown together as one line that opens. */
const EXPLORE = new Set(["read", "grep", "glob", "list", "webfetch", "websearch"])

type Block =
  | { kind: "item"; item: Item }
  | { kind: "explore"; id: string; tools: Tool[] }

function blocks(items: Item[]): Block[] {
  const out: Block[] = []
  for (const item of items) {
    const last = out[out.length - 1]
    if (item.kind === "tool" && EXPLORE.has(item.name)) {
      if (last?.kind === "explore") last.tools.push(item)
      else out.push({ kind: "explore", id: item.id, tools: [item] })
    } else out.push({ kind: "item", item })
  }
  return out
}

export function Transcript({
  open,
  permissions,
  questions,
  readOnly = false,
}: {
  open: Open
  permissions: Permission[]
  questions: Asked[]
  /** A subagent's transcript: no rewinding or editing. */
  readOnly?: boolean
}) {
  const busy = useStore((s) => (open.sessionId ? s.busy.has(open.sessionId) : false))
  const last = open.items[open.items.length - 1]
  // Plan wrote the last reply and the session is still in Plan.
  const planned =
    !readOnly &&
    !busy &&
    open.info?.agent === "plan" &&
    open.lastAgent === "plan" &&
    open.status.kind === "idle" &&
    !open.revert &&
    last?.kind === "text" &&
    open.info.agents.some((a) => a.id === "build")
  // The latest turn failed: as it happened, or as the session was left.
  const retryable =
    !readOnly &&
    !busy &&
    ((open.status.kind === "failed" && open.status.retry === true) ||
      (open.status.kind === "idle" && last?.kind === "error"))
  return (
    <div className="flex flex-col gap-5">
      {blocks(open.items).map((b) =>
        b.kind === "explore" ? (
          <Explored key={b.id} tools={b.tools} />
        ) : b.item.kind === "user" ? (
          <UserMessage key={b.item.id} item={b.item} actions={!readOnly} />
        ) : (
          <Entry key={b.item.id} item={b.item} directory={open.directory} />
        )
      )}
      {open.revert && !readOnly && <Rewound revert={open.revert} />}
      {/* Reasoning isn't shown: the gateway sends it summarized or empty, so a "Thought" row
          opened onto nothing. The status line says the model is at work meanwhile. */}
      {open.live.map((l) => l.kind === "text" && <Prose key={l.key} text={l.text} />)}
      {permissions.map((p, i) => (
        // Enter and Escape answer the first ask; the keys show on that one only.
        <PermissionCard key={p.requestId} permission={p} keys={i === 0} />
      ))}
      {questions.map((q) => (
        <QuestionCard key={q.requestId} asked={q} />
      ))}
      <StatusLine
        status={open.status}
        waiting={permissions.length + questions.length > 0}
        streaming={open.live.some((l) => l.kind === "text")}
      />
      {retryable && open.items.some((i) => i.kind === "user") && (
        <button onClick={() => void retry()} className={btn("secondary", "sm", "-mt-2 gap-1.5 self-start")}>
          <RotateCcw className="size-3.5" />
          Try again
        </button>
      )}
      {planned && (
        <div className="flex items-center gap-3">
          <button onClick={() => void buildPlan()} className={btn("primary", "sm", "gap-2")}>
            <Hammer className="size-3.5" />
            Build this plan
          </button>
          <span className="text-dense text-muted">Switches to Build and carries it out</span>
        </div>
      )}
    </div>
  )
}

/** A message the user sent: copy it, edit and send it again, or rewind to before it. */
function UserMessage({ item, actions }: { item: UserItem; actions: boolean }) {
  const [editing, setEditing] = useState(false)
  const [copied, setCopied] = useState(false)
  const [draft, setDraft] = useState(item.text)
  const area = useRef<HTMLTextAreaElement>(null)
  const [full, setFull] = useState(false)
  const long = item.text.length > 700 || item.text.split("\n").length > 10
  const submit = () => {
    if (!draft.trim()) return
    setEditing(false)
    void resend(item.id, draft)
    // Enter answers the turn's asks from the composer.
    document.querySelector<HTMLTextAreaElement>('textarea[aria-label="Message"]')?.focus()
  }
  if (editing)
    return (
      <div className="rounded-2xl bg-surface-secondary/60 p-2">
        <textarea
          ref={area}
          autoFocus
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onFocus={(e) => {
            const el = e.currentTarget
            el.setSelectionRange(el.value.length, el.value.length)
            el.style.height = `${Math.min(el.scrollHeight, 320)}px`
          }}
          onInput={(e) => {
            const el = e.currentTarget
            el.style.height = "auto"
            el.style.height = `${Math.min(el.scrollHeight, 320)}px`
          }}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault()
              submit()
            } else if (e.key === "Escape") {
              e.stopPropagation()
              setEditing(false)
              setDraft(item.text)
            }
          }}
          aria-label="Edit message"
          className="text-md block w-full resize-none bg-transparent px-2 pt-1 leading-relaxed outline-none"
        />
        <div className="flex items-center justify-end gap-2 pt-1">
          <span className="text-2xs mr-auto px-2 text-muted">
            Sends again from here, in place of this message and the turns after it
          </span>
          <button
            onClick={() => {
              setEditing(false)
              setDraft(item.text)
            }}
            className={btn("ghost", "sm")}
          >
            Cancel
          </button>
          <button onClick={submit} disabled={!draft.trim()} className={btn("primary", "sm")}>
            Send
          </button>
        </div>
      </div>
    )
  return (
    <div className="group flex flex-col items-stretch">
      <div className="rounded-2xl bg-surface-secondary/60 px-4 py-3 text-md leading-relaxed whitespace-pre-wrap">
        {item.command ? (
          <div className="flex items-center gap-2">
            <SquareSlash className="size-3.5 shrink-0 text-muted" />
            <span className="font-mono text-sm">{item.command}</span>
            <button
              onClick={() => setFull(!full)}
              className="text-2xs ml-auto text-muted hover:text-foreground focus-visible:status-focused-field"
            >
              {full ? "Hide prompt" : "Show prompt"}
            </button>
          </div>
        ) : (
          <div className={cn(long && !full && "line-clamp-6")}>{item.text}</div>
        )}
        {item.command && full && (
          <div className="text-dense mt-2 max-h-80 overflow-y-auto border-t border-separator pt-2 text-muted">{item.text}</div>
        )}
        {long && !item.command && (
          <button
            onClick={() => setFull(!full)}
            className="text-2xs mt-1.5 text-muted hover:text-foreground focus-visible:status-focused-field"
          >
            {full ? "Show less" : "Show all"}
          </button>
        )}
        {item.images > 0 && (
          <div className="text-2xs mt-1.5 text-muted">
            {item.images} image{item.images > 1 ? "s" : ""}
          </div>
        )}
      </div>
      {actions && (
        <div className="-mb-3 flex h-8 items-center justify-end gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
          <IconAction
            label={copied ? "Copied" : "Copy"}
            onClick={() =>
              void navigator.clipboard.writeText(item.text).then(() => {
                setCopied(true)
                setTimeout(() => setCopied(false), 1500)
              })
            }
          >
            {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
          </IconAction>
          {!item.command && <IconAction
            label="Edit and send again"
            onClick={() => {
              setDraft(item.text)
              setEditing(true)
            }}
          >
            <Pencil className="size-3.5" />
          </IconAction>}
          <IconAction label="Rewind to before this message" onClick={() => void rewind(item.id)}>
            <Undo2 className="size-3.5" />
          </IconAction>
        </div>
      )}
    </div>
  )
}

function IconAction({ label, onClick, children }: { label: string; onClick: () => void; children: ReactNode }) {
  return (
    <button
      onClick={onClick}
      aria-label={label}
      title={label}
      className="flex size-7 items-center justify-center rounded-md text-muted hover:bg-default/60 hover:text-foreground focus-visible:status-focused-field"
    >
      {children}
    </button>
  )
}

const MARK = { added: "A", modified: "M", deleted: "D" } as const

/** A staged rewind: what it hid and put back, until the next message makes it final. */
function Rewound({ revert }: { revert: NonNullable<Open["revert"]> }) {
  return (
    <div className="flex flex-col gap-3 rounded-2xl bg-surface-secondary/40 px-4 py-3">
      <div className="flex items-center gap-3">
        <Undo2 className="size-3.5 shrink-0 text-muted" />
        <span className="text-dense flex-1">
          Rewound {revert.turns} turn{revert.turns === 1 ? "" : "s"}
          {revert.files.length > 0 &&
            ` · ${revert.files.length} file${revert.files.length === 1 ? "" : "s"} put back`}
        </span>
        <button onClick={() => void redo()} className={btn("secondary", "sm", "gap-1.5")}>
          <Redo2 className="size-3.5" />
          Redo
        </button>
      </div>
      {revert.files.length > 0 && (
        <ul className="flex flex-col gap-1 pl-6.5">
          {revert.files.map((f) => (
            <li key={f.file} className="text-dense flex items-center gap-2 font-mono text-muted">
              <span className="w-3 text-center">{MARK[f.status] ?? "M"}</span>
              <span className="min-w-0 flex-1 truncate text-foreground/85">{f.file}</span>
              {f.additions + f.deletions > 0 && <Counts add={f.additions} del={f.deletions} />}
            </li>
          ))}
        </ul>
      )}
      <p className="text-2xs pl-6.5 text-muted">Your next message continues from here and makes this final.</p>
    </div>
  )
}

function Entry({ item, directory }: { item: Exclude<Item, UserItem>; directory: string }) {
  switch (item.kind) {
    case "text":
      return <Prose text={item.text} />
    case "reasoning":
      return null
    case "note":
      return (
        <div className={cn("text-dense flex items-center gap-3", item.failed ? "text-danger" : "text-muted")}>
          <span className="h-px flex-1 bg-separator" />
          {item.text}
          <span className="h-px flex-1 bg-separator" />
        </div>
      )
    case "error":
      return (
        <div className="text-dense flex items-start gap-2 text-danger">
          <CircleAlert className="mt-0.5 size-3.5 shrink-0" />
          {item.message}
        </div>
      )
    case "tool":
      if (item.files?.length) return <Edits tool={item} />
      if (item.name === "shell" || item.name === "bash") return <Command tool={item} />
      if (item.name === "subagent" || item.name === "task") return <Subagent tool={item} directory={directory} />
      return <OtherTool tool={item} />
  }
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

export function Prose({ text }: { text: string }) {
  return (
    <div className="prose-anysola text-md leading-relaxed text-foreground/90">
      <Markdown
        remarkPlugins={[remarkGfm]}
        components={{
          pre: ({ children }) => <CodeBlock>{children}</CodeBlock>,
          a: ({ href, children }) => (
            <a
              href={href}
              onClick={(e) => {
                e.preventDefault()
                if (href?.startsWith("http")) openExternal(href)
              }}
              // A middle click would open a WebView2 window of its own.
              onAuxClick={(e) => {
                if (e.button !== 1) return
                e.preventDefault()
                if (href?.startsWith("http")) openExternal(href)
              }}
              className="underline decoration-foreground/30 underline-offset-2 hover:decoration-foreground"
            >
              {children}
            </a>
          ),
        }}
      >
        {text}
      </Markdown>
    </div>
  )
}

/** A fenced block from the model, with a copy button that shows on hover. */
function CodeBlock({ children }: { children: ReactNode }) {
  const ref = useRef<HTMLPreElement>(null)
  const [copied, setCopied] = useState(false)
  const copy = () => {
    const text = ref.current?.innerText ?? ""
    void navigator.clipboard.writeText(text).then(() => {
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    })
  }
  return (
    <div className="group relative">
      <pre ref={ref}>{children}</pre>
      <button
        onClick={copy}
        aria-label={copied ? "Copied" : "Copy"}
        className="absolute top-1.5 right-1.5 flex size-7 items-center justify-center rounded-md bg-surface text-muted opacity-0 group-hover:opacity-100 hover:text-foreground focus-visible:opacity-100 focus-visible:status-focused-field"
      >
        {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
      </button>
    </div>
  )
}

function Disclosure({ open, onToggle, children }: { open: boolean; onToggle: () => void; children: ReactNode }) {
  return (
    <button
      onClick={onToggle}
      aria-expanded={open}
      className="text-dense flex h-8 w-full items-center gap-2 rounded-lg px-2 text-left text-muted hover:bg-default/60 focus-visible:status-focused-field"
    >
      {open ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
      {children}
    </button>
  )
}

// ---------------------------------------------------------------------------
// Tools
// ---------------------------------------------------------------------------

const icons: Record<string, typeof FileText> = {
  read: FileText,
  write: FilePlus2,
  edit: Pencil,
  list: FileText,
  grep: Search,
  glob: Search,
  webfetch: Globe,
  websearch: Globe,
  todowrite: ListTodo,
  question: MessageCircleQuestion,
}

const verbs: Record<string, string> = {
  read: "Read",
  // Edits without a diff (older sessions' writes) still say what they did.
  write: "Wrote",
  edit: "Edited",
  list: "Listed",
  grep: "Searched",
  glob: "Found files",
  webfetch: "Fetched",
  websearch: "Searched the web",
  todowrite: "Updated the plan",
  task: "Ran a subagent",
  subagent: "Ran a subagent",
  question: "Asked",
  skill: "Used a skill",
}

function summary(tools: Tool[]) {
  const reads = tools.filter((t) => t.name === "read" || t.name === "list").length
  const searches = tools.length - reads
  const parts = []
  if (reads) parts.push(`Read ${reads} file${reads > 1 ? "s" : ""}`)
  if (searches) parts.push(`searched ${searches === 1 ? "once" : `${searches} times`}`)
  const text = parts.join(", ")
  return text.charAt(0).toUpperCase() + text.slice(1)
}

function Explored({ tools }: { tools: Tool[] }) {
  const running = tools.some((t) => t.status === "running" || t.status === "streaming")
  const [open, setOpen] = useState(false)
  return (
    <div className="-mx-2">
      <Disclosure open={open} onToggle={() => setOpen(!open)}>
        {summary(tools)}
        {running && <Spinner className="size-3" />}
      </Disclosure>
      {open && (
        <div className="ml-[0.9rem] border-l border-separator pl-2">
          {tools.map((t) => (
            <ToolRow key={t.id} tool={t} />
          ))}
        </div>
      )}
    </div>
  )
}

function ToolRow({ tool, verb, icon }: { tool: Tool; verb?: string; icon?: typeof FileText }) {
  const Icon = icon ?? icons[tool.name] ?? Wrench
  return (
    <div className="text-dense flex h-8 items-center gap-2.5 rounded-lg px-2">
      <Icon className="size-3.5 shrink-0 text-muted" strokeWidth={1.75} />
      <span className="shrink-0 text-foreground/80">{verb ?? verbs[tool.name] ?? tool.name}</span>
      <span className="min-w-0 truncate font-mono text-xs text-muted">{tool.target}</span>
      <span className="ml-auto flex shrink-0 items-center gap-2 text-2xs text-muted">
        {(tool.status === "running" || tool.status === "streaming") && <Spinner className="size-3" />}
        {tool.status === "error" && <span className="text-danger">Failed</span>}
      </span>
    </div>
  )
}

function Subagent({ tool, directory }: { tool: Tool; directory: string }) {
  const running = tool.status === "running" || tool.status === "streaming"
  const child = tool.child
  return (
    <div className="-mx-2">
      <button
        onClick={() => child && void peekAt(directory, child, tool.target || "Subagent")}
        disabled={!child}
        className="text-dense group flex h-8 w-full items-center gap-2.5 rounded-lg px-2 text-left enabled:hover:bg-default/60 focus-visible:status-focused-field"
      >
        <Bot className="size-3.5 shrink-0 text-muted" strokeWidth={1.75} />
        <span className="shrink-0 text-foreground/80">Subagent</span>
        <span className="min-w-0 truncate text-muted">{tool.target}</span>
        <span className="ml-auto flex shrink-0 items-center gap-2 text-2xs text-muted">
          {running && <Spinner className="size-3" />}
          {tool.status === "error" && <span className="text-danger">Failed</span>}
          {child && <ChevronRight className="size-3.5 opacity-0 group-hover:opacity-100" />}
        </span>
      </button>
    </div>
  )
}

function OtherTool({ tool }: { tool: Tool }) {
  return (
    <div className="-mx-2">
      <ToolRow tool={tool} />
      {tool.error && <div className="text-2xs ml-8 text-danger">{tool.error}</div>}
    </div>
  )
}

function Command({ tool }: { tool: Tool }) {
  const [open, setOpen] = useState(false)
  const running = tool.status === "running" || tool.status === "streaming"
  const output = tool.error ?? tool.output
  return (
    <div className="-mx-2">
      <button
        onClick={() => output && setOpen(!open)}
        aria-expanded={output ? open : undefined}
        className="text-dense group flex h-8 w-full items-center gap-2.5 rounded-lg px-2 text-left hover:bg-default/60 focus-visible:status-focused-field"
      >
        <SquareTerminal className="size-3.5 shrink-0 text-muted" strokeWidth={1.75} />
        <span className="shrink-0 text-foreground/80">{running ? "Running" : "Ran"}</span>
        <span className="min-w-0 truncate font-mono text-xs text-muted">{tool.target}</span>
        <span className="ml-auto flex shrink-0 items-center gap-2 text-2xs text-muted">
          {running && <Spinner className="size-3" />}
          {tool.status === "error" && <span className="text-danger">Failed</span>}
          {output && (open ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5 opacity-0 group-hover:opacity-100" />)}
        </span>
      </button>
      {open && output && (
        <pre className="mx-2 mt-1 max-h-80 overflow-auto rounded-lg bg-background/60 px-3 py-2.5 font-mono text-xs leading-relaxed whitespace-pre-wrap text-foreground/75">
          {output}
        </pre>
      )}
    </div>
  )
}

function Edits({ tool }: { tool: Tool }) {
  return (
    <div className="flex flex-col gap-2">
      {tool.files!.map((f) => (
        <EditCard key={f.file} file={f} status={tool.status} error={tool.error} />
      ))}
    </div>
  )
}

/** A file's change. In the conversation it starts closed, one row per file; Review opens it. */
export function EditCard({
  file,
  status,
  error,
  startOpen = false,
}: {
  file: FileDiff
  status?: Tool["status"]
  error?: string | null
  startOpen?: boolean
}) {
  const [open, setOpen] = useState(startOpen)
  const added = file.status === "added"
  const waiting = status === "running"
  return (
    <div className="overflow-hidden rounded-xl bg-surface">
      <button
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className="text-dense flex h-9 w-full items-center gap-2.5 px-3 text-left hover:bg-default/30 focus-visible:status-focused-field focus-visible:ring-inset"
      >
        {added ? (
          <FilePlus2 className="size-3.5 shrink-0 text-muted" strokeWidth={1.75} />
        ) : (
          <Pencil className="size-3.5 shrink-0 text-muted" strokeWidth={1.75} />
        )}
        <span className="shrink-0 text-foreground/80">
          {waiting ? (added ? "Create" : "Edit") : added ? "Created" : file.status === "deleted" ? "Deleted" : "Edited"}
        </span>
        <span className="min-w-0 truncate font-mono text-xs text-muted">{file.file}</span>
        <span className="ml-auto flex shrink-0 items-center gap-2">
          {status === "error" && <span className="text-2xs text-danger">Not applied</span>}
          <Counts add={file.additions} del={file.deletions} />
          {open ? (
            <ChevronDown className="size-3.5 text-muted" />
          ) : (
            <ChevronRight className="size-3.5 text-muted" />
          )}
        </span>
      </button>
      {open && <DiffView patch={file.patch} className="pb-2" />}
      {error && <div className="text-2xs px-3 pb-2.5 text-danger">{error}</div>}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Asks and status
// ---------------------------------------------------------------------------

const asks: Record<string, string> = {
  shell: "Run a command?",
  edit: "Edit this file?",
  write: "Write this file?",
  webfetch: "Fetch this page?",
  external_directory: "Work outside this folder?",
}

function PermissionCard({ permission: p, keys }: { permission: Permission; keys: boolean }) {
  const shell = p.action === "shell"
  return (
    <div className="rounded-xl bg-surface p-4">
      <div className="flex items-center gap-2 text-sm">
        <span className="size-1.5 rounded-full bg-warning" />
        {asks[p.action] ?? `Allow ${p.action}?`}
      </div>
      {shell ? (
        <div className="text-dense mt-3 overflow-x-auto rounded-lg bg-background/60 px-3 py-2.5 font-mono whitespace-pre-wrap">
          {p.resources.map((r) => (
            <div key={r}>
              <span className="text-muted select-none">$ </span>
              {r}
            </div>
          ))}
        </div>
      ) : p.files?.length ? (
        <div className="mt-3 flex flex-col gap-2">
          {p.files.map((f) => (
            <div key={f.file} className="overflow-hidden rounded-lg bg-background/60">
              <div className="flex h-9 items-center gap-2 px-3">
                <span className="min-w-0 truncate font-mono text-xs text-muted">{f.file}</span>
                <span className="ml-auto">
                  <Counts add={f.additions} del={f.deletions} />
                </span>
              </div>
              <DiffView patch={f.patch} className="pb-2" />
            </div>
          ))}
        </div>
      ) : (
        <div className="mt-2 font-mono text-xs text-muted">{p.resources.join(", ")}</div>
      )}
      <div className="mt-4 flex flex-wrap items-center gap-2">
        <button onClick={() => void reply(p, "once")} className={btn("primary", "sm", "gap-2")}>
          Allow once {keys && <Kbd onPrimary>Enter</Kbd>}
        </button>
        {p.canAlways && (
          <button onClick={() => void reply(p, "always")} className={btn("secondary", "sm")}>
            Always allow{shell ? " this command" : ""}
          </button>
        )}
        <button onClick={() => void reply(p, "reject")} className={btn("ghost", "sm", "ml-auto gap-2")}>
          Deny {keys && <Kbd>Esc</Kbd>}
        </button>
      </div>
    </div>
  )
}

function QuestionCard({ asked }: { asked: Asked }) {
  const [picked, setPicked] = useState<string[][]>(() => asked.questions.map(() => []))
  const [other, setOther] = useState<string[]>(() => asked.questions.map(() => ""))
  const answers = asked.questions.map((_, i) => [...picked[i], ...(other[i].trim() ? [other[i].trim()] : [])])
  const complete = answers.every((a) => a.length > 0)
  const toggle = (i: number, label: string, multiple: boolean) => {
    // One answer to a single choice: picking an option drops a typed one.
    if (!multiple) setOther((all) => all.map((v, j) => (j === i ? "" : v)))
    setPicked((all) =>
      all.map((list, j) =>
        j !== i ? list : multiple ? (list.includes(label) ? list.filter((l) => l !== label) : [...list, label]) : [label]
      )
    )
  }
  return (
    <div className="flex flex-col gap-5 rounded-xl bg-surface p-4">
      {asked.questions.map((q, i) => (
        <fieldset key={i} className="flex flex-col gap-2.5">
          <legend className="contents">
            <span className="text-2xs font-medium tracking-wide text-muted">{q.header}</span>
            <span className="text-md mt-1 block leading-relaxed">{q.question}</span>
          </legend>
          <div className="flex flex-col gap-1">
            {q.options.map((o) => {
              const on = picked[i].includes(o.label)
              return (
                <button
                  key={o.label}
                  role={q.multiple ? "checkbox" : "radio"}
                  aria-checked={on}
                  onClick={() => toggle(i, o.label, !!q.multiple)}
                  className={cn(
                    "flex items-start gap-3 rounded-lg px-3 py-2 text-left transition-colors focus-visible:status-focused-field",
                    on ? "bg-default" : "hover:bg-default/60"
                  )}
                >
                  <span
                    className={cn(
                      "mt-0.5 flex size-4 shrink-0 items-center justify-center bg-default-hover",
                      q.multiple ? "rounded" : "rounded-full",
                      on && "bg-accent text-accent-foreground"
                    )}
                  >
                    {on && <Check className="size-3" strokeWidth={2.5} />}
                  </span>
                  <span className="min-w-0">
                    <span className="block text-sm">{o.label}</span>
                    {o.description && <span className="text-dense block text-muted">{o.description}</span>}
                  </span>
                </button>
              )
            })}
            <input
              value={other[i]}
              onChange={(e) => setOther((all) => all.map((v, j) => (j === i ? e.target.value : v)))}
              onFocus={() => !q.multiple && setPicked((all) => all.map((l, j) => (j === i ? [] : l)))}
              placeholder="Something else"
              aria-label={`Your own answer to: ${q.question}`}
              className="h-9 rounded-lg bg-background/60 px-3 text-sm outline-none placeholder:text-field-placeholder focus-visible:status-focused-field"
            />
          </div>
        </fieldset>
      ))}
      <div className="flex items-center gap-2">
        <button
          disabled={!complete}
          onClick={() => void answer(asked, answers)}
          className={btn("primary", "sm")}
        >
          Send answers
        </button>
        <button onClick={() => void answer(asked, null)} className={btn("ghost", "sm", "ml-auto text-muted")}>
          Skip
        </button>
      </div>
    </div>
  )
}

function StatusLine({ status, waiting, streaming }: { status: Status; waiting: boolean; streaming: boolean }) {
  if (waiting) return null
  switch (status.kind) {
    case "working":
      // Streaming text already shows the model at work.
      if (streaming) return null
      return (
        <div className="text-dense flex items-center gap-2 text-muted">
          <Spinner />
          Working
        </div>
      )
    case "retrying":
      return (
        <div className="text-dense flex items-start gap-2 text-muted">
          <Spinner className="mt-0.5" />
          <span>
            Retrying, attempt {status.attempt}
            <span className="block text-2xs text-muted/80">{status.error}</span>
          </span>
        </div>
      )
    case "failed":
      return (
        <div className="text-dense flex items-start gap-2 text-danger">
          <CircleAlert className="mt-0.5 size-3.5 shrink-0" />
          {status.error}
        </div>
      )
    case "stopped":
      return (
        <div className="text-dense text-muted">
          {status.reason === "budget"
            ? "Stopped at this session’s budget."
            : status.reason === "declined"
              ? "Stopped after a denied request."
              : "Stopped."}
        </div>
      )
    default:
      return null
  }
}
