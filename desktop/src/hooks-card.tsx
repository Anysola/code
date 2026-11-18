/**
 * What a project's files want to run (hooks, MCP servers, formatters) and wait for the user's
 * answer on, in the conversation of its folder: nothing of it runs until allowed. One card per
 * file; it asks without taking focus or keys from the message box.
 */
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react"

import type { HookEvent, TrustEntry, TrustItem, TrustKind } from "./bridge"
import { message, type Load } from "./settings/controls"
import { shortPath, when } from "./settings/hooks"
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

const KIND_ORDER: TrustKind[] = ["hooks", "mcp", "formatter"]
const HEADING: Record<TrustKind, string> = { hooks: "Hooks", mcp: "MCP servers", formatter: "Formatters" }

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
  return (
    <section className="rounded-xl bg-surface p-4" aria-label="What this project wants to run">
      <div className="flex items-center gap-2 text-sm">
        <span className="size-1.5 rounded-full bg-warning" />
        {onlyHooks && removed
          ? "This project’s hooks were removed"
          : onlyHooks && changed
            ? "This project’s hooks changed"
            : "Let this project run commands?"}
      </div>
      <p className="text-dense mt-1 pl-3.5 text-muted">
        {onlyHooks && removed
          ? "The file below no longer defines hooks. The ones you allowed before keep running until you accept that."
          : onlyHooks && changed
            ? `The ones you allowed before keep running until you allow ${hooks.entries.length === 1 ? "this one" : "these"}. Allow only if you trust the change.`
            : "Its config starts these on this computer once you allow them. Allow only if you trust this project."}
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
              {item.kind === "hooks" && !onlyHooks && (removed || changed) && (
                <p className="text-2xs text-muted">
                  {removed
                    ? "Removed from the file. The ones you allowed before keep running until you allow this."
                    : "Changed. The ones you allowed before keep running until you allow these."}
                </p>
              )}
              {item.entries.length > 0 && <Entries kind={item.kind} entries={item.entries} />}
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

const EVENTS = new Set<string>(["PreToolUse", "PostToolUse", "UserPromptSubmit", "Stop", "SessionStart"])

/** A hook's label as the core writes it, `PreToolUse (Bash)`, in the app's words. */
function hookLabel(label: string): ReactNode {
  const m = /^(\w+)(?: \((.*)\))?$/.exec(label)
  if (!m || !EVENTS.has(m[1])) return label
  return when(m[1] as HookEvent, m[2] ?? null)
}

/** The core's details, in the app's words where it has them: `timeout 300 s` is a time limit. */
function detailText(detail: string) {
  return detail
    .split("; ")
    .map((d) => d.replace(/^timeout (\d+) s$/, "Time limit $1 s").replace(/^timeout /, "Time limit "))
    .map((d) => d.charAt(0).toUpperCase() + d.slice(1))
    .join(" · ")
}

function Entries({ kind, entries }: { kind: TrustKind; entries: TrustEntry[] }) {
  return (
    <ul className="grid grid-cols-[auto_auto_minmax(0,1fr)] items-baseline gap-x-2.5 gap-y-1.5">
      {entries.map((e, i) => {
        // "the built-in formatters": words, not a command line.
        const words = kind === "formatter" && /^the built-in /.test(e.command)
        return (
          <li key={i} className="contents">
            <span className="text-dense flex max-w-56 min-w-0 flex-col text-foreground/85">
              <span className="break-words">
                {kind === "hooks" ? hookLabel(e.label) : kind === "formatter" && e.label === "formatter" ? "Formatters" : e.label}
              </span>
              {/* A shorter limit can weaken a guard, a header can carry a key: said where it is decided. */}
              {e.detail && <span className="text-2xs text-muted">{detailText(e.detail)}</span>}
            </span>
            <span className="text-2xs text-muted" aria-hidden>
              →
            </span>
            <span className={cn("break-all whitespace-pre-wrap", words ? "text-dense text-muted" : "font-mono text-xs")}>
              {words ? e.command.charAt(0).toUpperCase() + e.command.slice(1) : e.command}
            </span>
          </li>
        )
      })}
    </ul>
  )
}
