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
} from "lucide-react"
import { useRef, useState, type ReactNode } from "react"
import Markdown from "react-markdown"
import remarkGfm from "remark-gfm"

import { getBridge, type FileDiff, type Item } from "./bridge"
import { answer, peekAt, reply, type Asked, type Open, type Permission, type Status } from "./store"
import { btn, cn, Counts, DiffView, Kbd, Spinner } from "./ui"

type Tool = Extract<Item, { kind: "tool" }>

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
}: {
  open: Open
  permissions: Permission[]
  questions: Asked[]
}) {
  return (
    <div className="flex flex-col gap-5">
      {blocks(open.items).map((b) =>
        b.kind === "explore" ? (
          <Explored key={b.id} tools={b.tools} />
        ) : (
          <Entry key={b.item.id} item={b.item} directory={open.directory} />
        )
      )}
      {open.live.map((l) =>
        l.kind === "text" ? (
          <Prose key={l.key} text={l.text} />
        ) : (
          <Thinking key={l.key} text={l.text} live />
        )
      )}
      {permissions.map((p) => (
        <PermissionCard key={p.requestId} permission={p} />
      ))}
      {questions.map((q) => (
        <QuestionCard key={q.requestId} asked={q} />
      ))}
      <StatusLine
        status={open.status}
        waiting={permissions.length + questions.length > 0}
        streaming={open.live.length > 0}
      />
    </div>
  )
}

function Entry({ item, directory }: { item: Item; directory: string }) {
  switch (item.kind) {
    case "user":
      return (
        <div className="rounded-2xl bg-surface-secondary/60 px-4 py-3 text-md leading-relaxed whitespace-pre-wrap">
          {item.text}
          {item.images > 0 && (
            <div className="text-2xs mt-1.5 text-muted">
              {item.images} image{item.images > 1 ? "s" : ""}
            </div>
          )}
        </div>
      )
    case "text":
      return <Prose text={item.text} />
    case "reasoning":
      return <Thinking text={item.text} />
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
                if (href?.startsWith("http")) void getBridge().then((b) => b.openUrl(href))
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

function Thinking({ text, live = false }: { text: string; live?: boolean }) {
  const [open, setOpen] = useState(false)
  return (
    <div className="-mx-2">
      <Disclosure open={open} onToggle={() => setOpen(!open)}>
        {live ? "Thinking…" : "Thought"}
      </Disclosure>
      {open && <div className="text-dense ml-6 border-l border-separator pl-3 whitespace-pre-wrap text-muted">{text}</div>}
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

export function EditCard({ file, status, error }: { file: FileDiff; status?: Tool["status"]; error?: string | null }) {
  const [open, setOpen] = useState(true)
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

function PermissionCard({ permission: p }: { permission: Permission }) {
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
          Allow once <Kbd onPrimary>Enter</Kbd>
        </button>
        {p.canAlways && (
          <button onClick={() => void reply(p, "always")} className={btn("secondary", "sm")}>
            Always allow{shell ? " this command" : ""}
          </button>
        )}
        <button onClick={() => void reply(p, "reject")} className={btn("ghost", "sm", "ml-auto gap-2")}>
          Deny <Kbd>Esc</Kbd>
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
