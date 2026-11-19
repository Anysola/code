import {
  ChevronDown,
  ChevronRight,
  CircleAlert,
  Ellipsis,
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
  Webhook,
} from "lucide-react"
import { createContext, memo, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react"
import Markdown, { type Components, type ExtraProps } from "react-markdown"
import remarkGfm from "remark-gfm"

import type { FileDiff, FormRequest, HookRun, Item, ToolInput } from "./bridge"
import { duration, Elapsed } from "./conversation"
import { FormCard } from "./form-card"
import { TrustCards } from "./hooks-card"
import { copyPath } from "./header"
import { getPrefs, isSendKey, sendKeyLabel, usePrefs } from "./prefs"
import {
  answer,
  buildPlan,
  isRunning,
  messageImages,
  notify,
  openExternal,
  openInEditor,
  peekAt,
  redo,
  reply,
  resend,
  retry,
  revealFile,
  rewind,
  useStore,
  type Asked,
  type Open,
  type Permission,
  type Status,
} from "./store"
import { btn, cn, Counts, DiffView, Kbd, MenuItem, Popover, Spinner, useEscape } from "./ui"

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
    // A lookup a hook spoke about stands alone: folded into the group, its line would hide.
    if (item.kind === "tool" && EXPLORE.has(item.name) && !item.hooks?.length) {
      if (last?.kind === "explore") last.tools.push(item)
      else out.push({ kind: "explore", id: item.id, tools: [item] })
    } else out.push({ kind: "item", item })
  }
  return out
}

/** The open session's folder and whether a turn runs in it: what rows deep in the list need. */
type Where = { directory: string; sessionId: string | null; live: boolean }
const WhereContext = createContext<Where>({ directory: "", sessionId: null, live: false })

/** Kinds of failure that sending again won't fix. */
const FINAL = new Set(["limit", "auth", "model", "elsewhere"])

export function Transcript({
  open,
  permissions,
  questions,
  forms = [],
  readOnly = false,
}: {
  open: Open
  permissions: Permission[]
  questions: Asked[]
  forms?: FormRequest[]
  /** A subagent's transcript: no rewinding or editing. */
  readOnly?: boolean
}) {
  const busy = useStore((s) => (open.sessionId ? s.busy.has(open.sessionId) : false))
  const last = open.items[open.items.length - 1]
  const live = busy || isRunning(open)
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
  // The latest turn failed for a reason sending again can fix: as it happened, or as the session was left.
  const retryable =
    !readOnly &&
    !busy &&
    ((open.status.kind === "failed" && open.status.retry === true) ||
      (open.status.kind === "idle" &&
        last?.kind === "error" &&
        last.retry !== false &&
        !(last.errorKind && FINAL.has(last.errorKind))))
  const where = useMemo(
    () => ({ directory: open.directory, sessionId: open.sessionId, live }),
    [open.directory, open.sessionId, live]
  )
  const shown = useMemo(() => turns(blocks(open.items), live), [open.items, live])
  return (
    <WhereContext.Provider value={where}>
      <div className="flex flex-col gap-5">
        {shown.map((b) =>
          b.kind === "end" ? (
            <TurnEnd key={b.id} elapsed={b.elapsed} text={b.text} />
          ) : b.kind === "explore" ? (
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
        {!readOnly && <TrustCards directory={open.directory} className="flex flex-col gap-5" />}
        {permissions.map((p, i) => (
          // Enter and Escape answer the first ask; the keys show on that one only.
          <PermissionCard key={p.requestId} permission={p} keys={i === 0} />
        ))}
        {questions.map((q) => (
          <QuestionCard key={q.requestId} asked={q} />
        ))}
        {forms.map((f) => (
          <FormCard key={f.formId} form={f} />
        ))}
        {!readOnly && (
          <StatusLine
            open={open}
            waiting={permissions.length + questions.length + forms.length > 0}
            streaming={open.live.some((l) => l.kind === "text")}
          />
        )}
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
    </WhereContext.Provider>
  )
}

type Shown = Block | { kind: "end"; id: string; elapsed: number | null; text: string | null }

/** Each finished turn gets a row under it: how long it worked, and Copy for its last reply. */
function turns(list: Block[], live: boolean): Shown[] {
  const out: Shown[] = []
  let user: UserItem | null = null
  let reply: string | null = null
  const close = (running: boolean) => {
    if (user && !running && (user.elapsed != null || reply)) out.push({ kind: "end", id: `end:${user.id}`, elapsed: user.elapsed ?? null, text: reply })
  }
  for (const b of list) {
    if (b.kind === "item" && b.item.kind === "user") {
      close(false)
      user = b.item
      reply = null
    } else if (b.kind === "item" && b.item.kind === "text") reply = b.item.text
    out.push(b)
  }
  close(live)
  return out
}

function TurnEnd({ elapsed, text }: { elapsed: number | null; text: string | null }) {
  const [copied, setCopied] = useState(false)
  return (
    <div className="group -mt-3 -mb-2 flex h-7 items-center gap-1">
      {elapsed != null && elapsed >= 1000 && (
        <span className="text-2xs pr-1 text-muted/80">Worked for {duration(elapsed)}</span>
      )}
      {text && (
        <span className={cn("opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100", (elapsed == null || elapsed < 1000) && "-ml-1.5")}>
          <IconAction
            label={copied ? "Copied" : "Copy reply"}
            onClick={() =>
              void navigator.clipboard.writeText(text).then(
                () => {
                  setCopied(true)
                  setTimeout(() => setCopied(false), 1500)
                },
                () => notify("The reply couldn’t be copied. Select its text and copy it instead.")
              )
            }
          >
            {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
          </IconAction>
        </span>
      )}
    </div>
  )
}

/** A message the user sent: copy it, edit and send it again, or rewind to before it. */
function UserMessage({ item, actions }: { item: UserItem; actions: boolean }) {
  const [editing, setEditing] = useState(false)
  const [copied, setCopied] = useState(false)
  const [draft, setDraft] = useState(item.text)
  const [full, setFull] = useState(false)
  const shorten = usePrefs((p) => p.conversation.shorten)
  const long = shorten && (item.text.length > 700 || item.text.split("\n").length > 10)
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
            if (isSendKey(e)) {
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
            {/* The core's words for a ``!`cmd` `` in the template the user declined (`command::NOT_RUN`). */}
            {item.text.includes(NOT_RUN) && (
              <span className="text-2xs text-muted">· a command in it didn’t run</span>
            )}
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
        {item.images > 0 && <Thumbnails item={item} />}
      </div>
      {actions && (
        <div className="-mb-3 flex h-8 items-center justify-end gap-0.5 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
          {item.time != null && <span className="text-2xs mr-auto pl-1 text-muted/80">{sentAt(item.time)}</span>}
          <IconAction
            label={copied ? "Copied" : "Copy"}
            onClick={() =>
              void navigator.clipboard.writeText(item.text).then(
                () => {
                  setCopied(true)
                  setTimeout(() => setCopied(false), 1500)
                },
                () => notify("The message couldn’t be copied. Select its text and copy it instead.")
              )
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

/** "14:32" today, "Mon 14:32" this week, else "3 Nov 14:32". */
function sentAt(ms: number) {
  const d = new Date(ms)
  const time = d.toLocaleTimeString("en-GB", { hour: "2-digit", minute: "2-digit" })
  const days = (Date.now() - ms) / 86_400_000
  if (new Date().toDateString() === d.toDateString()) return time
  if (days < 6) return `${d.toLocaleDateString("en-GB", { weekday: "short" })} ${time}`
  return `${d.toLocaleDateString("en-GB", { day: "numeric", month: "short" })} ${time}`
}

/** A sent message's images, read when the message shows; a click opens them large. */
function Thumbnails({ item }: { item: UserItem }) {
  const { directory, sessionId } = useContext(WhereContext)
  const [urls, setUrls] = useState<string[] | null>(null)
  const [failed, setFailed] = useState(false)
  const [big, setBig] = useState<number | null>(null)
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const el = ref.current
    if (!el || !sessionId) return
    let live = true
    // Read once it scrolls into view: a long session doesn't read every image it ever sent.
    const seen = new IntersectionObserver((entries) => {
      if (!entries.some((e) => e.isIntersecting)) return
      seen.disconnect()
      messageImages(directory, sessionId, item.id).then(
        (list) => live && setUrls(list),
        () => live && setFailed(true)
      )
    })
    seen.observe(el)
    return () => {
      live = false
      seen.disconnect()
    }
  }, [directory, sessionId, item.id])
  return (
    <div ref={ref} className="mt-2.5 flex flex-wrap gap-2">
      {urls
        ? urls.map((u, i) => (
            <button
              key={i}
              onClick={() => setBig(i)}
              aria-label={`Image ${i + 1} of ${urls.length}`}
              className="overflow-hidden rounded-lg focus-visible:status-focused-field"
            >
              <img src={u} alt="" className="size-16 object-cover" />
            </button>
          ))
        : Array.from({ length: item.images }, (_, i) => (
            <span key={i} className="flex size-16 items-center justify-center rounded-lg bg-default/60 text-muted">
              {failed ? <CircleAlert className="size-3.5" /> : null}
            </span>
          ))}
      {failed && <span className="text-2xs self-center text-muted">The images couldn’t be read.</span>}
      {urls && big !== null && <Lightbox urls={urls} at={big} onMove={setBig} onClose={() => setBig(null)} />}
    </div>
  )
}

/** An image large over the window: arrows step through the message's images, Escape closes
 * (and only closes: the composer's Escape would stop the turn). */
function Lightbox({ urls, at, onMove, onClose }: { urls: string[]; at: number; onMove: (i: number) => void; onClose: () => void }) {
  const panel = useRef<HTMLDivElement>(null)
  useEscape(panel, onClose)
  useEffect(() => {
    panel.current?.focus()
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowRight" && at < urls.length - 1) onMove(at + 1)
      if (e.key === "ArrowLeft" && at > 0) onMove(at - 1)
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [at, urls.length, onMove])
  return (
    <div
      ref={panel}
      tabIndex={-1}
      role="dialog"
      aria-label="Image"
      onClick={onClose}
      className="fixed inset-0 z-50 flex flex-col items-center justify-center gap-3 bg-scrim p-10 outline-none"
    >
      <img src={urls[at]} alt="" onClick={(e) => e.stopPropagation()} className="max-h-full max-w-full min-h-0 rounded-xl object-contain" />
      {urls.length > 1 && (
        <span className="text-2xs rounded-full bg-overlay px-2.5 py-1 text-muted tabular-nums">
          {at + 1} of {urls.length}
        </span>
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

const MARK = { added: "Added", modified: "Edited", deleted: "Deleted" } as const

/** A staged rewind: what it hid and put back, until the next message makes it final. */
function Rewound({ revert }: { revert: NonNullable<Open["revert"]> }) {
  const [all, setAll] = useState(false)
  const files = all ? revert.files : revert.files.slice(0, 5)
  const more = revert.files.length - files.length
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
          {files.map((f) => (
            <li key={f.file} className="text-dense flex items-center gap-2 font-mono text-muted">
              <span className="text-2xs w-12 shrink-0 font-sans">{MARK[f.status] ?? "Edited"}</span>
              <span className="min-w-0 flex-1 truncate text-foreground/85">{f.file}</span>
              {f.additions + f.deletions > 0 && <Counts add={f.additions} del={f.deletions} />}
            </li>
          ))}
          {more > 0 && (
            <li>
              <button
                onClick={() => setAll(true)}
                className="text-2xs text-muted hover:text-foreground focus-visible:status-focused-field"
              >
                {more} more
              </button>
            </li>
          )}
        </ul>
      )}
      <p className="text-2xs pl-6.5 text-muted">Your next message continues from here and makes this final.</p>
    </div>
  )
}

/** Rows render again only when their item changed (the store keeps unchanged items as they were). */
const Entry = memo(function Entry({ item, directory }: { item: Exclude<Item, UserItem>; directory: string }) {
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
})

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

/** A link in a reply: web and mail links go to the browser, a file of the folder to the editor. */
function ReplyLink({ href, children }: { href?: string; children?: ReactNode }) {
  const { directory } = useContext(WhereContext)
  const go = () => {
    if (!href) return
    if (/^(https?:|mailto:)/i.test(href)) return openExternal(href)
    const file = localFile(href)
    if (file && directory) void openInEditor(directory, file.path, file.line)
    else if (/^file:/i.test(href)) notify("That file is on another computer, so it isn’t opened from here.")
  }
  return (
    <a
      href={href}
      title={href}
      onClick={(e) => {
        e.preventDefault()
        go()
      }}
      // A middle click would open a WebView2 window of its own.
      onAuxClick={(e) => {
        if (e.button !== 1) return
        e.preventDefault()
        go()
      }}
      className="underline decoration-foreground/30 underline-offset-2 hover:decoration-foreground"
    >
      {children}
    </a>
  )
}

/** A link to a file: `file://` or a path, with `#L12` or `:12` naming a line. Null for anything
 * else with a scheme (`javascript:`, `ftp:`…) and for a `file://` link to another computer
 * (`file://host/share/…`): following it would reach that computer. */
export function localFile(href: string): { path: string; line: number | null } | null {
  let h = href.trim()
  if (/^file:/i.test(h)) {
    // file:////host/share keeps the host in the path, past the URL parser.
    if (/^file:[\\/]{4}/i.test(h)) return null
    try {
      const url = new URL(h)
      if (url.hostname && url.hostname.toLowerCase() !== "localhost") return null
      // The line (`#L12`) is the URL's hash.
      h = decodeURIComponent(url.pathname) + url.hash
      // file:///C:/x arrives as /C:/x.
      if (/^\/[a-z]:/i.test(h)) h = h.slice(1)
    } catch {
      return null
    }
  } else if (/^[a-z][a-z0-9+.-]+:/i.test(h) && !/^[a-z]:[\\/]/i.test(h)) return null
  let line: number | null = null
  const hash = /#L(\d+)(?:-L?\d+)?$/.exec(h)
  if (hash) {
    line = Number(hash[1])
    h = h.slice(0, hash.index)
  } else {
    const colon = /:(\d+)(?::\d+)?$/.exec(h)
    if (colon && !/^[a-z]:$/i.test(h.slice(0, colon.index))) {
      line = Number(colon[1])
      h = h.slice(0, colon.index)
    }
  }
  h = h.replace(/[?#].*$/, "")
  if (!h) return null
  try {
    h = decodeURIComponent(h)
  } catch {
    // Kept as written.
  }
  return { path: h, line }
}

const MARKDOWN: Components = {
  pre: ({ children, node }) => <CodeBlock node={node}>{children}</CodeBlock>,
  a: ({ href, children }) => <ReplyLink href={href}>{children}</ReplyLink>,
}
const PLUGINS = [remarkGfm]
/** Links keep their address as written: clicks are handled above, nothing navigates. */
const keepUrl = (url: string) => url

/** Markdown, parsed again only when its text changes (a streaming reply, not its neighbours). */
export const Prose = memo(function Prose({ text }: { text: string }) {
  return (
    <div className="prose-anysola text-md leading-relaxed text-foreground/90">
      <Markdown remarkPlugins={PLUGINS} components={MARKDOWN} urlTransform={keepUrl}>
        {text}
      </Markdown>
    </div>
  )
})

type Hast = { type: string; value?: string; tagName?: string; properties?: Record<string, unknown>; children?: Hast[] }

function textOf(node: Hast | undefined): string {
  if (!node) return ""
  if (node.type === "text") return node.value ?? ""
  return (node.children ?? []).map(textOf).join("")
}

/** Over this many lines, a block shows its first lines and "Show all". */
const CODE_LIMIT = 24
const CODE_SHOWN = 16

/**
 * A fenced block from the model, with a copy button that shows on hover. With "shorten" on, a
 * long block shows its start; copy always takes the whole block.
 */
function CodeBlock({ children, node }: { children: ReactNode; node: ExtraProps["node"] }) {
  const [copied, setCopied] = useState(false)
  const [full, setFull] = useState(false)
  const shorten = usePrefs((p) => p.conversation.shorten)
  const code = (node as Hast | undefined)?.children?.find((c) => c.tagName === "code")
  const source = textOf(code ?? (node as Hast | undefined)).replace(/\n$/, "")
  const lines = source.split("\n").length
  const cut = !!code && shorten && lines > CODE_LIMIT && !full
  const language = (code?.properties?.className as string[] | undefined)?.join(" ")
  const copy = () => {
    void navigator.clipboard.writeText(source).then(
      () => {
        setCopied(true)
        setTimeout(() => setCopied(false), 1500)
      },
      () => notify("The code couldn’t be copied. Select it and copy it instead.")
    )
  }
  return (
    <div className="group relative">
      {cut ? (
        <pre>
          <code className={language}>{source.split("\n").slice(0, CODE_SHOWN).join("\n")}</code>
        </pre>
      ) : (
        <pre>{children}</pre>
      )}
      {code && shorten && lines > CODE_LIMIT && (
        <button
          onClick={() => setFull(!full)}
          aria-expanded={full}
          className="text-2xs mt-1.5 text-muted hover:text-foreground focus-visible:status-focused-field"
        >
          {full ? "Show less" : `Show all ${lines} lines`}
        </button>
      )}
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
  // Code mode: a short script that calls tools (MCP servers' among them).
  execute: "Ran a script",
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
  const { live } = useContext(WhereContext)
  const running = live && tools.some((t) => t.status === "running" || t.status === "streaming")
  const [open, setOpen] = useState(false)
  const listed = usePrefs((p) => p.conversation.lookups === "listed")
  if (listed)
    return (
      <div className="-mx-2">
        {tools.map((t) => (
          <ToolRow key={t.id} tool={t} />
        ))}
      </div>
    )
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

const FILE_TOOLS = new Set(["read", "write", "edit", "list"])
/** Lookups whose output is worth a look (the matches, the files found). */
const OPENS = new Set(["grep", "glob", "websearch", "webfetch"])

/** A row is running only while its turn is: a stopped turn can leave one that never heard its end. */
function useToolState(tool: Tool): "running" | "waiting" | "declined" | "blocked" | "stopped" | "error" | "done" {
  const { live, sessionId } = useContext(WhereContext)
  // A call held by a permission card isn't running yet: the card under it asks.
  const held = useStore(
    (s) =>
      !!tool.target &&
      s.permissions.some(
        (p) =>
          (p.sessionId === sessionId || p.rootId === sessionId) &&
          p.resources.some((r) => !!r && (tool.target.includes(r) || r.includes(tool.target)))
      )
  )
  if (tool.status === "running" || tool.status === "streaming")
    return !live ? "stopped" : held ? "waiting" : "running"
  if (tool.status === "error" && tool.error === DECLINED) return "declined"
  if (hookBlocked(tool)) return "blocked"
  if (cutShort(tool)) return "stopped"
  return tool.status === "error" ? "error" : "done"
}

/** What the core writes on a call that a stop cut short: the user's doing, not a failure. */
const CUT_SHORT = "Tool execution interrupted"
/** And on a call the user answered Deny to. */
const DECLINED = "The user declined this tool call"

/** Ended by the user, by a stop, a Deny or their hook: the row says so itself, with no error under it. */
function cutShort(tool: Tool) {
  // A call settled after the app died reads "Tool execution interrupted: <tool>".
  return tool.status === "error" && (!!tool.error?.startsWith(CUT_SHORT) || tool.error === DECLINED || hookBlocked(tool))
}

/** A PreToolUse hook stopped the call: its line under the row gives the reason the error repeats. */
function hookBlocked(tool: Tool) {
  return tool.status === "error" && !!tool.hooks?.some((h) => h.event === "PreToolUse" && h.outcome === "blocked")
}

/** What a call would have done, for a row whose call never ran. */
const intents: Record<string, string> = {
  read: "Read",
  write: "Write",
  edit: "Edit",
  list: "List",
  grep: "Search",
  glob: "Find files",
  webfetch: "Fetch",
  websearch: "Search the web",
  skill: "Use a skill",
  execute: "Run a script",
}

/** What the user's hooks did around a call, under its row whether or not the row is open. */
function HookLines({ runs, className }: { runs?: HookRun[]; className?: string }) {
  if (!runs?.length) return null
  return (
    <div className={cn("flex flex-col", className)}>
      {runs.map((r, i) => (
        <HookLine key={i} run={r} />
      ))}
    </div>
  )
}

const HOOK_SAYS: Record<HookRun["outcome"], string> = {
  blocked: "Blocked by a hook",
  feedback: "A hook told the agent something",
  context: "A hook added context for the agent",
  error: "A hook failed; the work went on",
  notice: "A hook says",
}

function HookLine({ run }: { run: HookRun }) {
  const [open, setOpen] = useState(false)
  const icon = (
    <Webhook
      className={cn(
        "mt-[0.2rem] size-3 shrink-0",
        run.outcome === "blocked" ? "text-danger" : run.outcome === "error" ? "text-warning" : "text-muted"
      )}
      strokeWidth={1.75}
    />
  )
  const title = run.command ? `Hook: ${run.command}` : undefined
  // Said to the agent: one click away. Said to the user (a block's reason, a failure, a notice): in the line.
  if (run.outcome === "feedback" || run.outcome === "context")
    return (
      <div title={title}>
        <button
          onClick={() => setOpen(!open)}
          aria-expanded={open}
          className="text-2xs flex min-h-6 items-start gap-2 rounded-md py-1 pr-1 text-left text-muted hover:text-foreground focus-visible:status-focused-field"
        >
          {icon}
          <span>{HOOK_SAYS[run.outcome]}</span>
          {open ? <ChevronDown className="mt-px size-3 shrink-0" /> : <ChevronRight className="mt-px size-3 shrink-0" />}
        </button>
        {open && (
          <pre className="mt-1 mb-1 max-h-60 overflow-auto rounded-lg bg-background/60 px-3 py-2.5 font-mono text-xs leading-relaxed whitespace-pre-wrap text-foreground/75 select-text">
            {run.message}
          </pre>
        )}
      </div>
    )
  // A notice with no command is the app's own word about hooks, not something a hook said.
  const own = run.outcome === "notice" && !run.command
  return (
    <div title={title} className="text-2xs flex min-h-6 items-start gap-2 py-1">
      {icon}
      <span className="max-h-24 min-w-0 overflow-y-auto whitespace-pre-wrap select-text">
        {!own && (
          <span className={run.outcome === "notice" ? "text-muted" : "text-foreground/85"}>{HOOK_SAYS[run.outcome]}</span>
        )}
        {run.message && (
          <span className={run.outcome === "notice" ? "text-foreground/85" : "text-muted"}>
            {own ? run.message : ` · ${run.message}`}
          </span>
        )}
      </span>
    </div>
  )
}

/** What a write, an edit or a script says while it is being written and while it runs. */
const going: Record<string, string> = {
  write: "Writing",
  edit: "Editing",
  execute: "Running a script",
}

/** The input a `streaming` row has so far, while its turn runs; null once the call is whole. */
function useInput(tool: Tool): ToolInput | null {
  const { live } = useContext(WhereContext)
  const input = useStore((s) => (tool.status === "streaming" ? (s.inputs[tool.id] ?? null) : null))
  return live ? input : null
}

/** "12 lines" as a written text grows. */
function LineCount({ input }: { input: ToolInput | null }) {
  if (!input?.lines) return null
  return (
    <span className="tabular-nums">
      {input.lines.toLocaleString()} {input.lines === 1 ? "line" : "lines"}
    </span>
  )
}

/** The last lines of a text being written: grows to its few lines, then holds its height while
 * they move up; long lines are cut, never wrapped. */
function LiveTail({ text }: { text: string }) {
  const lines = text.split("\n")
  const most = useRef(0)
  most.current = Math.max(most.current, lines.length)
  return (
    <div
      aria-hidden
      style={{ height: `calc(${most.current} * 1.25rem + 1.25rem)` }}
      className="mx-2 mt-1 mb-1 flex flex-col justify-end overflow-hidden rounded-lg bg-background/60 px-3 py-2.5 font-mono text-xs leading-5 text-foreground/70"
    >
      {lines.map((line, i) => (
        <div key={i} className="truncate whitespace-pre">
          {line || "\u00a0"}
        </div>
      ))}
    </div>
  )
}

function ToolState({ state }: { state: ReturnType<typeof useToolState> }) {
  if (state === "running") return <Spinner className="size-3" />
  if (state === "waiting") return <span>Waiting for you</span>
  if (state === "declined") return <span>Declined</span>
  if (state === "blocked") return <span>Blocked</span>
  if (state === "stopped") return <span>Stopped</span>
  if (state === "error") return <span className="text-danger">Failed</span>
  return null
}

function ToolRow({ tool, verb, icon }: { tool: Tool; verb?: string; icon?: typeof FileText }) {
  const Icon = icon ?? icons[tool.name] ?? Wrench
  const state = useToolState(tool)
  const input = useInput(tool)
  const { directory } = useContext(WhereContext)
  const error = cutShort(tool) ? null : tool.error
  const output = OPENS.has(tool.name) ? (error ?? tool.output) : error
  const [open, setOpen] = useState(false)
  const target = input?.target || tool.target
  const file = FILE_TOOLS.has(tool.name) && target && directory && !input ? target : null
  // The model is still writing a script: it runs once written.
  const named =
    state === "running" && tool.status === "streaming" && tool.name === "execute" ? "Writing a script" : state === "running" ? going[tool.name] : undefined
  const head = (
    <>
      <Icon className="size-3.5 shrink-0 text-muted" strokeWidth={1.75} />
      <span className="shrink-0 text-foreground/80">
        {verb ?? named ?? (state === "blocked" || state === "stopped" ? (intents[tool.name] ?? verbs[tool.name]) : verbs[tool.name]) ?? tool.name}
      </span>
      <span className="min-w-0 truncate font-mono text-xs text-muted">{target}</span>
    </>
  )
  return (
    <div className="group/tool">
      <div className="text-dense flex h-8 items-center gap-2.5 rounded-lg px-2 hover:bg-default/40">
        {output ? (
          <button
            onClick={() => setOpen(!open)}
            aria-expanded={open}
            className="flex min-w-0 flex-1 items-center gap-2.5 text-left focus-visible:status-focused-field"
          >
            {head}
          </button>
        ) : file ? (
          <button
            onClick={() => void openInEditor(directory, file, null)}
            title="Open in your editor"
            className="flex min-w-0 flex-1 items-center gap-2.5 text-left focus-visible:status-focused-field [&>span:last-child]:hover:underline [&>span:last-child]:decoration-muted/50 [&>span:last-child]:underline-offset-2"
          >
            {head}
          </button>
        ) : (
          <span className="flex min-w-0 flex-1 items-center gap-2.5">{head}</span>
        )}
        <span className="ml-auto flex shrink-0 items-center gap-2 text-2xs text-muted">
          <LineCount input={input} />
          <ToolState state={state} />
          {file && <PathMenu directory={directory} path={file} />}
          {output && (open ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />)}
        </span>
      </div>
      <HookLines runs={tool.hooks} className="pl-8" />
      {input?.tail && <LiveTail text={input.tail} />}
      {open && output && (
        <pre
          className={cn(
            "mx-2 mt-1 mb-1 max-h-60 overflow-auto rounded-lg bg-background/60 px-3 py-2.5 font-mono text-xs leading-relaxed whitespace-pre-wrap",
            error ? "text-danger" : "text-foreground/75"
          )}
        >
          {output}
        </pre>
      )}
    </div>
  )
}

/** Open in the editor, show in the file manager, or copy a file's path. */
function PathMenu({ directory, path }: { directory: string; path: string }) {
  const [show, setShow] = useState(false)
  const close = () => setShow(false)
  const absolute = /^([a-z]:[\\/]|[\\/])/i.test(path) ? path : `${directory.replace(/[\\/]+$/, "")}${directory.includes("\\") ? "\\" : "/"}${path}`
  return (
    <span className="relative" onClick={(e) => e.stopPropagation()}>
      <button
        onClick={() => setShow(!show)}
        aria-label={`More for ${path}`}
        aria-expanded={show}
        className={cn(
          "flex size-6 items-center justify-center rounded-md text-muted hover:bg-default/60 hover:text-foreground focus-visible:opacity-100 focus-visible:status-focused-field",
          !show && "opacity-0 group-hover/tool:opacity-100"
        )}
      >
        <Ellipsis className="size-3.5" />
      </button>
      <Popover open={show} onClose={close} side="bottom" align="end" menu label={path} className="w-48">
        <MenuItem onSelect={() => (close(), void openInEditor(directory, path, null))}>Open in editor</MenuItem>
        <MenuItem onSelect={() => (close(), void revealFile(absolute, directory))}>Reveal in folder</MenuItem>
        <MenuItem onSelect={() => (close(), copyPath(absolute))}>Copy path</MenuItem>
      </Popover>
    </span>
  )
}

function Subagent({ tool, directory }: { tool: Tool; directory: string }) {
  const state = useToolState(tool)
  const target = useInput(tool)?.target || tool.target
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
        <span className="min-w-0 truncate text-muted">{target}</span>
        <span className="ml-auto flex shrink-0 items-center gap-2 text-2xs text-muted">
          {state === "running" ? <Spinner className="size-3" /> : <ToolState state={state} />}
          {state === "done" && <span>Done</span>}
          {tool.elapsed != null && state !== "running" && <span className="tabular-nums">{duration(tool.elapsed)}</span>}
          {child && <ChevronRight className="size-3.5" />}
        </span>
      </button>
      <HookLines runs={tool.hooks} className="pl-8" />
    </div>
  )
}

function OtherTool({ tool }: { tool: Tool }) {
  return (
    <div className="-mx-2">
      <ToolRow tool={tool} />
      {tool.error && !cutShort(tool) && (
        <div className="text-2xs ml-8 max-h-32 overflow-y-auto whitespace-pre-wrap text-danger">{tool.error}</div>
      )}
    </div>
  )
}

function Command({ tool }: { tool: Tool }) {
  const state = useToolState(tool)
  // A command shows as it is typed, in its place on the row.
  const target = useInput(tool)?.target || tool.target
  // A failed command shows why at once; the user's own click wins after that.
  const [chosen, setChosen] = useState<boolean | null>(null)
  const open = chosen ?? (state === "error" || getPrefs().conversation.commands === "open")
  const output = (cutShort(tool) ? null : tool.error) || tool.output
  return (
    <div className="-mx-2">
      <button
        onClick={() => output && setChosen(!open)}
        aria-expanded={output ? open : undefined}
        className="text-dense group flex h-8 w-full items-center gap-2.5 rounded-lg px-2 text-left hover:bg-default/60 focus-visible:status-focused-field"
      >
        <SquareTerminal className="size-3.5 shrink-0 text-muted" strokeWidth={1.75} />
        <span className="shrink-0 text-foreground/80">
          {state === "running" ? "Running" : state === "waiting" || state === "declined" || state === "blocked" ? "Run" : "Ran"}
        </span>
        <span className="min-w-0 truncate font-mono text-xs text-muted">{target}</span>
        <span className="ml-auto flex shrink-0 items-center gap-2 text-2xs text-muted">
          <ToolState state={state} />
          {output && (open ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />)}
        </span>
      </button>
      <HookLines runs={tool.hooks} className="pl-8" />
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
        <EditCard key={f.file} file={f} status={tool.status} error={cutShort(tool) ? null : tool.error} />
      ))}
      <HookLines runs={tool.hooks} className="-mt-1 pl-9" />
    </div>
  )
}

/** A file's change: closed or open in the conversation as Settings says; Review passes its own. */
export function EditCard({
  file,
  status,
  error,
  startOpen,
}: {
  file: FileDiff
  status?: Tool["status"]
  error?: string | null
  startOpen?: boolean
}) {
  const [open, setOpen] = useState(() => startOpen ?? getPrefs().conversation.changes === "open")
  const added = file.status === "added"
  const live = useContext(WhereContext).live
  const waiting = status === "running" && live
  // Review, outside the conversation, shows the open session's folder too.
  const openDirectory = useStore((s) => s.open?.directory ?? "")
  const directory = useContext(WhereContext).directory || openDirectory
  return (
    <div className="group/tool overflow-hidden rounded-xl bg-surface">
      <div className="flex h-9 items-center hover:bg-default/30">
        <button
          onClick={() => setOpen(!open)}
          aria-expanded={open}
          className="text-dense flex h-full min-w-0 flex-1 items-center gap-2.5 pl-3 text-left focus-visible:status-focused-field focus-visible:ring-inset"
        >
          {added ? (
            <FilePlus2 className="size-3.5 shrink-0 text-muted" strokeWidth={1.75} />
          ) : (
            <Pencil className="size-3.5 shrink-0 text-muted" strokeWidth={1.75} />
          )}
          <span className="shrink-0 text-foreground/80">
            {waiting ? (added ? "Add" : "Edit") : added ? "Added" : file.status === "deleted" ? "Deleted" : "Edited"}
          </span>
          <span className="min-w-0 truncate font-mono text-xs text-muted">{file.file}</span>
          <span className="ml-auto flex shrink-0 items-center gap-2 pr-1">
            {status === "error" && <span className="text-2xs text-danger">Not applied</span>}
            <Counts add={file.additions} del={file.deletions} />
            {open ? <ChevronDown className="size-3.5 text-muted" /> : <ChevronRight className="size-3.5 text-muted" />}
          </span>
        </button>
        <span className="flex shrink-0 items-center pr-2">
          {directory && file.status !== "deleted" ? <PathMenu directory={directory} path={file.file} /> : <span className="w-1" />}
        </span>
      </div>
      {/* In the conversation a long diff scrolls inside its card; Review shows it whole. */}
      {open && (
        <DiffView
          patch={file.patch}
          className={cn("pb-2", startOpen === undefined && "max-h-[min(36rem,70vh)] overflow-y-auto")}
        />
      )}
      {error && <div className="text-2xs max-h-32 overflow-y-auto px-3 pb-2.5 whitespace-pre-wrap text-danger">{error}</div>}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Asks and status
// ---------------------------------------------------------------------------

/** What the core puts in a command's prompt for a shell placeholder the user declined. */
const NOT_RUN = "(not run: the session's permissions did not allow this shell command)"

const FILE_ASKS = new Set(["read", "edit", "write"])
const ABSOLUTE = /^([\\/]|[A-Za-z]:[\\/])/

const asks: Record<string, string> = {
  shell: "Run a command?",
  read: "Read this file?",
  edit: "Edit this file?",
  write: "Write this file?",
  webfetch: "Fetch this page?",
  external_directory: "Work outside this folder?",
}

function PermissionCard({ permission: p, keys }: { permission: Permission; keys: boolean }) {
  const shell = p.action === "shell"
  const send = usePrefs((s) => s.conversation.send)
  // The answer is on its way: the card waits for it instead of going (and coming back on a failure).
  const sending = useStore((s) => s.sending.has(p.requestId))
  return (
    <div className="rounded-xl bg-surface p-4">
      <div className="flex items-center gap-2 text-sm">
        <span className="size-1.5 rounded-full bg-warning" />
        {asks[p.action] ?? `Allow ${p.action}?`}
        {/* A helper the agent started is asking: its own conversation is not the one shown. */}
        {p.sessionId !== p.rootId && <span className="text-2xs text-muted">· asked by a subagent</span>}
      </div>
      {shell ? (
        // Capped, so Allow and Deny stay on screen however long the ask is.
        <div className="text-dense mt-3 max-h-[min(22rem,40vh)] overflow-auto rounded-lg bg-background/60 px-3 py-2.5 font-mono whitespace-pre-wrap">
          {p.resources.map((r) => (
            <div key={r}>
              <span className="text-muted select-none">$ </span>
              {r}
            </div>
          ))}
        </div>
      ) : p.files?.length ? (
        <div className="mt-3 flex max-h-[min(22rem,40vh)] flex-col gap-2 overflow-y-auto rounded-lg">
          {p.files.map((f) => (
            <div key={f.file} className="shrink-0 overflow-hidden rounded-lg bg-background/60">
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
        // "*" is the rule's own wildcard (an MCP tool has no narrower target): nothing to read.
        p.resources.some((r) => r !== "*") && (
          <div className="mt-2 max-h-40 overflow-y-auto font-mono text-xs break-all text-muted">
            {FILE_ASKS.has(p.action) ? (
              // One file can come under two names (from the session's folder, then from the
              // project's root): the first, and the other said as the same file.
              <>
                {p.resources[0]}
                {/* A relative name is the project's; an absolute one is where a link leads. */}
                {p.resources.slice(1).some((r) => !ABSOLUTE.test(r)) && (
                  <span className="text-2xs block font-sans">
                    Same file as {p.resources.slice(1).filter((r) => !ABSOLUTE.test(r)).join(", ")} in the project
                  </span>
                )}
                {p.resources.slice(1).some((r) => ABSOLUTE.test(r)) && (
                  <span className="text-2xs block font-sans">
                    Leads to {p.resources.slice(1).filter((r) => ABSOLUTE.test(r)).join(", ")}
                  </span>
                )}
              </>
            ) : (
              p.resources.filter((r) => r !== "*").join(", ")
            )}
          </div>
        )
      )}
      <div className="mt-4 flex flex-wrap items-center gap-2">
        <button onClick={() => void reply(p, "once")} disabled={sending} className={btn("primary", "sm", "gap-2")}>
          Allow once {keys && <Kbd onPrimary>{sendKeyLabel(send)}</Kbd>}
        </button>
        {p.canAlways && (
          <button onClick={() => void reply(p, "always")} disabled={sending} className={btn("secondary", "sm")}>
            Always allow{shell ? " this command" : ""}
          </button>
        )}
        {sending && <Spinner className="ml-1" />}
        <button onClick={() => void reply(p, "reject")} disabled={sending} className={btn("ghost", "sm", "ml-auto gap-2")}>
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
  const sending = useStore((s) => s.sending.has(asked.requestId))
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
      {/* Capped, so Send stays on screen however many questions come. */}
      <div className="-m-1 flex max-h-[min(26rem,50vh)] flex-col gap-5 overflow-y-auto p-1">
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
                      "mt-0.5 flex size-4 shrink-0 items-center justify-center",
                      q.multiple ? "rounded" : "rounded-full",
                      on ? "bg-accent text-accent-foreground" : "border-[1.5px] border-muted/70"
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
      </div>
      <div className="flex items-center gap-2">
        <button
          disabled={!complete || sending}
          onClick={() => void answer(asked, answers)}
          className={btn("primary", "sm")}
        >
          Send answers
        </button>
        {sending && <Spinner />}
        <button onClick={() => void answer(asked, null)} disabled={sending} className={btn("ghost", "sm", "ml-auto text-muted")}>
          Skip
        </button>
      </div>
    </div>
  )
}

/** What runs now, from the newest running tool: "Running pnpm test", "Editing src/app.tsx". */
function doing(items: Item[]): string | null {
  const running: Tool[] = []
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i]
    if (it.kind === "user") break
    if (it.kind === "tool" && (it.status === "running" || it.status === "streaming")) running.push(it)
  }
  const t = running[0]
  if (!t) return null
  const reads = running.filter((r) => r.name === "read").length
  if (t.name === "read" && reads > 1) return `Reading ${reads} files`
  const target = t.target ? ` ${t.target}` : ""
  switch (t.name) {
    case "shell":
    case "bash":
      return `Running${target}`
    case "edit":
      return `Editing${target}`
    case "write":
      return `Writing${target}`
    case "read":
      return `Reading${target}`
    case "list":
      return `Listing${target}`
    case "grep":
    case "glob":
      return `Searching for${target}`
    case "webfetch":
      return `Fetching${target}`
    case "websearch":
      return `Searching the web for${target}`
    case "task":
    case "subagent":
      return t.target ? `Subagent: ${t.target}` : "Running a subagent"
    case "todowrite":
      return "Updating the plan"
    default:
      return `Using ${t.name}`
  }
}

/** Kinds the banner over the composer explains, with its own way out. */
const BANNERED = new Set(["limit", "offline", "auth", "model"])

function StatusLine({ open, waiting, streaming }: { open: Open; waiting: boolean; streaming: boolean }) {
  const status: Status = open.status
  const doingNow = useMemo(() => doing(open.items), [open.items])
  // A running row right above already names what runs; the line then only keeps the time.
  const last = open.items[open.items.length - 1]
  const named = last?.kind === "tool" && (last.status === "running" || last.status === "streaming") && !last.child
  const now = named ? null : doingNow
  if (waiting) return null
  if (open.stopping && (status.kind === "working" || status.kind === "retrying"))
    return (
      <div className="text-dense flex items-center gap-2 text-muted">
        <Spinner />
        Stopping…
      </div>
    )
  switch (status.kind) {
    case "working":
      // Streaming text already shows the model at work; what a tool does still says so.
      if (streaming && !doingNow) return null
      return (
        <div className="text-dense flex min-w-0 items-center gap-2 text-muted">
          <Spinner className="shrink-0" />
          <span className="min-w-0 truncate">{now ?? (open.live.some((l) => l.kind === "reasoning") ? "Thinking" : "Working")}</span>
          {open.started && (
            <span className="text-2xs shrink-0 text-muted/80">
              · <Elapsed since={open.started} />
            </span>
          )}
        </div>
      )
    case "retrying":
      return (
        <div className="text-dense flex items-center gap-2 text-muted" title={status.error}>
          <Spinner />
          <span>Reconnecting… (attempt {status.attempt})</span>
        </div>
      )
    case "failed":
      if (status.errorKind && BANNERED.has(status.errorKind)) return null
      // The user's own hook said no: their rule at work, not a failure. The line says the text is
      // back only when it is (a queued message the window can't tell apart isn't).
      if (status.hook)
        return (
          <div className="text-dense flex items-start gap-2">
            <Webhook className="mt-0.5 size-3.5 shrink-0 text-muted" strokeWidth={1.75} />
            <span className="flex min-w-0 flex-col gap-0.5">
              <span>
                {status.back
                  ? "A hook stopped this message. It’s back in the message box."
                  : "A hook stopped a message, so it wasn’t sent."}
              </span>
              <span className="max-h-40 overflow-y-auto whitespace-pre-wrap text-muted select-text">{status.error}</span>
            </span>
          </div>
        )
      return (
        // Running elsewhere isn't an error to fix here: said plainly, not in red.
        <div className={cn("text-dense flex items-start gap-2", status.errorKind === "elsewhere" ? "text-muted" : "text-danger")}>
          <CircleAlert className="mt-0.5 size-3.5 shrink-0" />
          <span className="max-h-40 min-w-0 overflow-y-auto whitespace-pre-wrap select-text">{status.error}</span>
        </div>
      )
    case "stopped":
      // Every stop ends with the core's own note in the transcript (stopped, denied, the budget).
      return null
    default:
      return null
  }
}
