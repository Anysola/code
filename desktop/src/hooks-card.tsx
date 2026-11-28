/**
 * What a project's files want to run (hooks, MCP servers, formatters) and wait for the user's
 * answer on, in the conversation of its folder: nothing of it runs until allowed. One card per
 * file; it asks without taking focus or keys from the message box.
 */
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react"

import type { HookEvent, TrustEntry, TrustItem, TrustKind } from "./bridge"
import { message, type Load } from "./settings/controls"
import { HOOK_EVENTS, NEW_EVENTS, REASKED, shortPath, when } from "./settings/hooks"
import { currentBridge, setTrust, trustAnswered, useStore } from "./store"
import { btn, cn, folderKey, Spinner } from "./ui"

// ---------------------------------------------------------------------------
// The folder's answers, read once and shared
// ---------------------------------------------------------------------------

/** What the folder's project files want to run, read again when that or their hooks change. */
export function useTrust(directory: string): Load<TrustItem[]> {
  const data = useStore((s) => s.trust[folderKey(directory)])
  const changed = useStore((s) => s.trustChanged + s.hooksChanged)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const n = useRef(0)
  const reload = useCallback(() => {
    const at = ++n.current
    setLoading(true)
    setError(null)
    currentBridge()
      .trust(directory)
      .then(
        (items) => {
          if (at !== n.current) return
          setTrust(directory, items)
          setLoading(false)
        },
        (e) => {
          if (at !== n.current) return
          setError(message(e))
          setLoading(false)
        }
      )
  }, [directory])
  useEffect(reload, [reload, changed])
  const set = useCallback((items: TrustItem[]) => setTrust(directory, items), [directory])
  return { data, error, loading, reload, set }
}

/** Records one answer for `items` as they are now; every view showing them reads again. */
export async function answerTrust(directory: string, items: TrustItem[], allow: boolean) {
  const next = await currentBridge().trustAnswer(
    directory,
    items.map(({ path, kind, digest }) => ({ path, kind, digest })),
    allow
  )
  setTrust(directory, next)
  trustAnswered(items.map((i) => i.kind))
  return next
}

/** The waiting items of each project file, files in the order the core lists them. */
export function waitingFiles(items: TrustItem[] | undefined): [string, TrustItem[]][] {
  const files = new Map<string, TrustItem[]>()
  for (const i of items ?? []) if (i.decision === null) files.set(i.path, [...(files.get(i.path) ?? []), i])
  return [...files.entries()]
}

// ---------------------------------------------------------------------------
// The card
// ---------------------------------------------------------------------------

/** One card per project file with something waiting for an answer; nothing while nothing does. */
export function TrustCards({ directory, className }: { directory: string; className?: string }) {
  const load = useTrust(directory)
  const files = waitingFiles(load.data)
  if (files.length === 0) return null
  return (
    <div className={className}>
      {files.map(([path, items]) => (
        <TrustCard key={`${path}:${items.map((i) => i.digest).join(",")}`} path={path} items={items} directory={directory} />
      ))}
    </div>
  )
}

const KIND_ORDER: TrustKind[] = ["hooks", "mcp", "formatter", "permissions", "remote", "lsp"]
const HEADING: Record<TrustKind, string> = {
  hooks: "Hooks",
  mcp: "MCP servers",
  formatter: "Formatters",
  permissions: "Changes to your rules",
  remote: "What it downloads",
  lsp: "Language servers",
}

/**
 * When a project's downloads happen, as the core does them: instruction URLs when the folder
 * opens and when its list changes, skill indexes at every config load, repositories when the
 * folder opens and hourly, updated at most daily. Never per message.
 */
export const WHEN_DOWNLOADED =
  "The app downloads these when you open this project and again when its config changes. Repositories are updated at most once a day."

/** What allowing does, by what it is about: the card's question names each that waits. */
const DOES = {
  run: "run commands",
  rules: "change your rules",
  fetch: "download files",
  lsp: "run language servers",
} as const
const doesOf = (kind: TrustKind): keyof typeof DOES =>
  kind === "permissions" ? "rules" : kind === "remote" ? "fetch" : kind === "lsp" ? "lsp" : "run"

/** "Let this project run commands, change your rules and download files?" for what waits. */
function question(items: TrustItem[]) {
  const does = (["run", "rules", "fetch", "lsp"] as const)
    .filter((d) => items.some((i) => doesOf(i.kind) === d))
    .map((d) => DOES[d])
  const list = does.length > 1 ? `${does.slice(0, -1).join(", ")} and ${does[does.length - 1]}` : does[0]
  return `Let this project ${list}?`
}

/** The line under the question: what allowing changes, when it is one thing; else that nothing applies yet. */
function consequence(items: TrustItem[]) {
  const does = new Set(items.map((i) => doesOf(i.kind)))
  if (does.size > 1) return "None of this applies until you allow it. Allow only if you trust this project."
  if (does.has("rules")) return "These change what the agent may do without asking you. Allow only if you trust this project."
  if (does.has("fetch")) return `${WHEN_DOWNLOADED} Allow only if you trust this project.`
  if (does.has("lsp"))
    return "Language servers run this project’s own config, plugins, and build tooling on edited files. Allow only if you trust this project."
  return "Its config starts these on this computer once you allow them. Allow only if you trust this project."
}

function TrustCard({ path, items, directory }: { path: string; items: TrustItem[]; directory: string }) {
  const [busy, setBusy] = useState<boolean | null>(null)
  const [error, setError] = useState<string | null>(null)
  const answer = (allow: boolean) => {
    setBusy(allow)
    setError(null)
    answerTrust(directory, items, allow)
      .catch((e) => setError(message(e)))
      .finally(() => setBusy(null))
  }
  const sorted = [...items].sort((a, b) => KIND_ORDER.indexOf(a.kind) - KIND_ORDER.indexOf(b.kind))
  const hooks = items.find((i) => i.kind === "hooks")
  // The file's hooks alone wait, and the ones allowed before still run: the 1.0.9 questions.
  const onlyHooks = items.length === 1 && hooks
  const removed = !!hooks && hooks.entries.length === 0
  const changed = !!hooks?.standing && !removed
  // Allowed before 1.6.0 and asked again only for the moments it added: nothing of it runs now.
  const reasked = !!hooks?.reasked && !removed && !changed
  return (
    <section className="rounded-xl bg-surface p-4" aria-label="What this project wants to change">
      <div className="flex items-center gap-2 text-sm">
        <span className="size-1.5 rounded-full bg-warning" />
        {onlyHooks && removed
          ? "This project’s hooks were removed"
          : onlyHooks && changed
            ? "This project’s hooks changed"
            : onlyHooks && reasked
              ? "Allow this project’s hooks again?"
              : question(items)}
      </div>
      <p className="text-dense mt-1 pl-3.5 text-muted">
        {onlyHooks && removed
          ? "The file below no longer defines hooks. The ones you allowed before keep running until you stop them."
          : onlyHooks && changed
            ? `The ones you allowed before keep running until you allow ${hooks.entries.length === 1 ? "this one" : "these"}. Allow only if you trust the change.`
            : onlyHooks && reasked
              ? REASKED
              : consequence(items)}
      </p>
      {/* Not capped: what the answer covers is all on the page, never inside a box of its own that scrolls. */}
      <div className="mt-3 flex flex-col gap-3 rounded-lg bg-background/60 px-3 py-2.5">
        <div className="text-2xs shrink-0 truncate font-mono text-muted" title={path}>
          {shortPath(path, directory)}
        </div>
        {sorted.map((item) =>
          onlyHooks && removed ? null : (
            <div key={item.kind} className="flex shrink-0 flex-col gap-1.5">
              {/* Hooks alone read as hooks ("Before shell commands"); a server or a formatter
                  alone is a bare name and a command, so it keeps its heading. */}
              {(items.length > 1 || item.kind !== "hooks") && (
                <h3 className="text-2xs font-medium tracking-wide text-muted">{HEADING[item.kind]}</h3>
              )}
              {item.kind === "hooks" && !onlyHooks && (removed || changed || reasked) && (
                <p className="text-2xs text-muted">
                  {removed
                    ? "Removed from the file. The ones you allowed before keep running until you allow this."
                    : changed
                      ? "Changed. The ones you allowed before keep running until you allow these."
                      : REASKED}
                </p>
              )}
              {/* With other kinds the question above says nothing of when: said here. */}
              {item.kind === "remote" && items.length > 1 && <p className="text-2xs text-muted">{WHEN_DOWNLOADED}</p>}
              {item.entries.length > 0 && <Entries kind={item.kind} entries={item.entries} reasked={item.kind === "hooks" && reasked} />}
            </div>
          )
        )}
      </div>
      <div className="mt-4 flex flex-wrap items-center gap-2">
        <button onClick={() => answer(true)} disabled={busy !== null} className={btn("primary", "sm")}>
          {onlyHooks && removed ? "Stop running them" : "Allow"}
        </button>
        {busy !== null && <Spinner className="ml-1" />}
        <button onClick={() => answer(false)} disabled={busy !== null} className={btn("ghost", "sm", "ml-auto")}>
          {onlyHooks && removed ? "Keep them running" : "Don’t allow"}
        </button>
      </div>
      {error && <p className="text-dense mt-2 text-danger select-text">{error}</p>}
    </section>
  )
}

/** A hook's label as the core writes it, `PreToolUse (Bash)`, in the app's words. */
function hookLabel(label: string): ReactNode {
  const m = /^(\w+)(?: \((.*)\))?$/.exec(label)
  if (!m || !HOOK_EVENTS.has(m[1])) return label
  return when(m[1] as HookEvent, m[2] ?? null)
}

/** Where a project's rule applies, as the core labels it: `all agents`, `agent build`, `policy`, `sessions`. */
export function scopeLabel(label: string) {
  if (label === "all agents") return "Every agent"
  if (label === "policy") return "Project policy"
  if (label === "sessions") return "New sessions"
  const agent = /^agent (.+)$/.exec(label)
  return agent ? `The ${agent[1].charAt(0).toUpperCase()}${agent[1].slice(1)} agent` : label
}

/** A download's label: `instructions` and `skills` are the app's words, anything else a repository's name. */
export function remoteLabel(label: string) {
  return label === "instructions" ? "Instructions" : label === "skills" ? "Skills" : label
}

/**
 * The project's own text inside a sentence: on a tint of its own, so nothing in it (a line
 * break, the words "without asking") can pass as part of the sentence around it. A long one wraps.
 */
const Code = ({ children }: { children: ReactNode }) => (
  <span className="rounded-sm bg-default px-1 py-px font-mono text-xs break-all whitespace-pre-wrap [box-decoration-break:clone]">
    {children}
  </span>
)

/** The widest a project can ask for: said in the warning tone, like a server it would replace. */
const Wide = ({ children }: { children: ReactNode }) => <span className="text-warning">{children}</span>

const PATTERN = /[*?[]/

/**
 * A project rule as a person reads it: `allow shell: npm test*` is "Run npm test* without
 * asking", a policy `deny mcp.use: github` is "Turn off your server github". `label` is where it
 * applies (`sessions`: which agent new sessions start with). Anything the app has no words for
 * is shown as the core wrote it.
 */
export function ruleText(rule: string, label?: string): ReactNode {
  if (label === "sessions") {
    const agent = /^start with the agent (.+)$/s.exec(rule)
    if (agent) return <>Start new sessions with this project’s agent <Code>{agent[1]}</Code></>
    return <Code>{rule}</Code>
  }
  // `<effect> <action>: <resource>`; the resource is the rest, whatever it holds.
  const head = /^(allow|deny|ask) /.exec(rule)
  if (!head) return <Code>{rule}</Code>
  const effect = head[1]
  const tail = rule.slice(head[0].length)
  const colon = tail.indexOf(": ")
  const action = colon < 0 ? tail.replace(/:$/, "") : tail.slice(0, colon)
  const resource = colon < 0 ? "" : tail.slice(colon + 2)
  const any = resource === "" || resource === "*"
  const what = <Code>{resource}</Code>
  if (effect === "deny") {
    if (action === "mcp.use") return <>Turn off your server {what}</>
    if (action === "provider.use") return <>Turn off your provider {what}</>
    if (action === "integration.use" || action === "integration.connect") return <>Turn off your integration {what}</>
    return <Code>{rule}</Code>
  }
  if (effect !== "allow") return <Code>{rule}</Code>
  const without = " without asking"
  // A pattern for the action covers every tool whose name it matches: no tool's words fit it.
  if (action === "*")
    return <Wide>{any ? <>Do anything{without}</> : <>Do anything on {what}{without}</>}</Wide>
  if (PATTERN.test(action)) {
    const tools = <>Use any tool matching <Code>{action}</Code></>
    return any ? <Wide>{tools} on anything{without}</Wide> : <>{tools} on {what}{without}</>
  }
  switch (action) {
    case "shell":
    case "bash":
      return any ? <Wide>Run any command{without}</Wide> : <>Run {what}{without}</>
    case "edit":
    case "write":
      return any ? <>Change any file{without}</> : <>Change {what}{without}</>
    case "read":
      return any ? <>Read any file{without}</> : <>Read {what}{without}</>
    case "webfetch":
      return any ? <>Open any web page{without}</> : <>Open {what}{without}</>
    case "websearch":
      return <>Search the web{without}</>
    case "external_directory":
      return any ? <>Work in any folder outside the project{without}</> : <>Work in {what}, outside the project,{without}</>
    case "mcp.use":
      return any ? <>Use any MCP server{without}</> : <>Use the server {what}{without}</>
    case "tool.use":
      return any ? <>Use any tool{without}</> : <>Use the tool {what}{without}</>
    default:
      return any ? <>Use <Code>{action}</Code>{without}</> : <>Use <Code>{action}</Code> on {what}{without}</>
  }
}

/** An address a project makes the app fetch, said as what happens. */
export function remoteText(e: TrustEntry): ReactNode {
  if (e.label === "instructions") return <>Reads instructions from <Code>{e.command}</Code></>
  if (e.label === "skills") return <>Downloads skills from <Code>{e.command}</Code></>
  const repo = /^(.*) \(branch (.+)\)$/s.exec(e.command)
  return repo ? (
    <>
      Downloads the repository <Code>{repo[1]}</Code>, branch <Code>{repo[2]}</Code>
    </>
  ) : (
    <>Downloads the repository <Code>{e.command}</Code></>
  )
}

/** Said first, on a line of its own: allowing takes the user's own server away. */
const REPLACES = "Replaces your server of this name"

/** The core's details, in the app's words where it has them: `timeout 300 s` is a time limit. */
function detailParts(detail: string) {
  return detail
    .split("; ")
    .map((d) => d.replace(/^timeout (\d+) s$/, "Time limit $1 s").replace(/^timeout /, "Time limit "))
    .map((d) => d.replace(/^oauth\b/i, "OAuth"))
    .map((d) => d.charAt(0).toUpperCase() + d.slice(1))
}

function Entries({ kind, entries, reasked = false }: { kind: TrustKind; entries: TrustEntry[]; reasked?: boolean }) {
  return (
    <ul className="grid grid-cols-[auto_auto_minmax(0,1fr)] items-baseline gap-x-2.5 gap-y-1.5">
      {entries.map((e, i) => {
        // "the built-in formatters": words, not a command line.
        const words = kind === "formatter" && /^the built-in /.test(e.command)
        // Rules and downloads are said as sentences, with the rule's or the address's own text in them.
        const sentence = kind === "permissions" ? ruleText(e.command, e.label) : kind === "remote" ? remoteText(e) : null
        // A rule's detail is the agent file it comes from: the card already names its file.
        // An LSP entry's detail is the resolved binary path: keep it verbatim in monospace.
        const lspPath = kind === "lsp" ? e.detail : null
        const parts = e.detail && kind !== "permissions" && kind !== "lsp" ? detailParts(e.detail) : []
        const replaces = parts[0]?.startsWith(REPLACES)
        const rest = replaces ? parts.slice(1) : parts
        return (
          <li key={i} className="contents">
            <span className="text-dense max-w-56 min-w-0 break-words text-foreground/85">
              {kind === "permissions"
                ? scopeLabel(e.label)
                : kind === "remote"
                  ? remoteLabel(e.label)
                  : kind === "hooks"
                ? (
                  <>
                    {hookLabel(e.label)}
                    {reasked && NEW_EVENTS.has(e.label.split(" ")[0]) && <span className="text-2xs ml-1.5 text-muted">New</span>}
                  </>
                )
                : kind === "formatter" && e.label === "formatter"
                  ? "Formatters"
                  : kind === "mcp" && e.label === "MCP timeouts"
                    ? "Server time limits"
                    : e.label}
            </span>
            <span className="text-2xs text-muted" aria-hidden>
              →
            </span>
            {sentence ? (
              <span className="text-dense min-w-0 break-words">{sentence}</span>
            ) : (
              <span className={cn("break-all whitespace-pre-wrap", words ? "text-dense text-muted" : "font-mono text-xs")}>
                {words ? e.command.charAt(0).toUpperCase() + e.command.slice(1) : e.command}
              </span>
            )}
            {/* Under the row, the width of the card: a long value (a header, an environment
                variable) wraps here and never squeezes the command. A shorter limit can weaken a
                guard, a header can carry a key: said where it is decided. */}
            {(replaces || rest.length > 0 || lspPath) && (
              <span className="col-span-3 -mt-1 flex min-w-0 flex-col gap-0.5 pb-0.5">
                {replaces && <span className="text-dense text-warning">Replaces your own server named {e.label}</span>}
                {lspPath && <span className="text-2xs font-mono break-all text-muted">{lspPath}</span>}
                {rest.length > 0 && (
                  <span className="text-2xs break-all whitespace-pre-wrap text-muted">{rest.join(" · ")}</span>
                )}
              </span>
            )}
          </li>
        )
      })}
    </ul>
  )
}
