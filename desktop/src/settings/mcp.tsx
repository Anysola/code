/**
 * MCP servers for the folder: each with its state and the actions that state allows, where it
 * is defined, and a form to add one to the agent's config file.
 */
import { Plus, X } from "lucide-react"
import { useEffect, useRef, useState } from "react"

import type { McpDraft, McpServer, TrustItem } from "../bridge"
import { answerTrust, useTrust } from "../hooks-card"
import { currentBridge, openExternal, settingsFolder, useStore } from "../store"
import { btn, cn, folderKey, sameFolder, Spinner, Tooltip } from "../ui"
import { OpenPath } from "./agent"
import { shortPath } from "./hooks"
import { Confirm, Failed, FolderLine, Group, Loaded, message, Row, Section, Segmented, tilde, Toggle, useLoad } from "./controls"

const STATUS: Record<McpServer["status"], { word: string; mark: string }> = {
  connected: { word: "Connected", mark: "bg-success" },
  pending: { word: "Connecting", mark: "bg-muted" },
  disabled: { word: "Off", mark: "bg-muted/40" },
  failed: { word: "Failed", mark: "bg-danger" },
  needs_auth: { word: "Sign in needed", mark: "bg-warning" },
  // Under its file's question, which says why.
  waiting: { word: "Not started", mark: "bg-muted/40" },
}

const CONFIG_FILE = "~/.anysola/config.json"

export function McpSection() {
  const open = useStore((s) => s.open?.directory ?? null)
  const scratch = useStore((s) => s.scratch)
  const [directory, setDirectory] = useState<string | null>(open)
  const [error, setError] = useState<string | null>(null)
  const [tries, setTries] = useState(0)
  useEffect(() => {
    setError(null)
    settingsFolder().then(setDirectory, (e) => setError(message(e)))
  }, [open, tries])
  return (
    <Section title="MCP servers">
      {error && !directory && <Failed error={error} retry={() => setTries((n) => n + 1)} bare />}
      {directory && <FolderLine directory={directory} own={sameFolder(directory, scratch)} />}
      {directory && <Servers directory={directory} />}
    </Section>
  )
}

function Servers({ directory }: { directory: string }) {
  const changed = useStore((s) => s.mcpChanged)
  const load = useLoad(() => currentBridge().mcpList(directory), [directory])
  // A project's servers wait for an answer about their file: the answers are read alongside.
  const trust = useTrust(directory)
  const [adding, setAdding] = useState(false)
  const first = useRef(true)
  // The core says a server changed (connected, failed, signed in): read the list again.
  useEffect(() => {
    if (first.current) {
      first.current = false
      return
    }
    load.reload()
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [changed])
  return (
    <Loaded load={load} rows={4}>
      {(all) => {
        // A project's servers that wait are asked about by file, above the rest: one answer each.
        const servers = all.filter((s) => s.status !== "waiting")
        const files = new Map<string, McpServer[]>()
        for (const s of all) if (s.status === "waiting") files.set(s.source, [...(files.get(s.source) ?? []), s])
        return (
        <>
          {[...files.entries()].map(([source, list]) => (
            <ProjectServers
              key={source}
              source={source}
              servers={list}
              directory={directory}
              item={trust.data?.find((i) => i.kind === "mcp" && folderKey(i.path) === folderKey(source))}
            />
          ))}
          {adding && (
            <AddForm
              directory={directory}
              taken={all.map((s) => s.name)}
              onDone={(next) => {
                setAdding(false)
                if (next) load.set(next)
              }}
            />
          )}
          <Group
            label="Servers"
            aside={
              // The empty state has its own button.
              !adding &&
              servers.length > 0 && (
                <button onClick={() => setAdding(true)} className={btn("ghost", "sm", "h-6 gap-1 px-2 text-2xs text-muted")}>
                  <Plus className="size-3" />
                  Add server
                </button>
              )
            }
            note={
              servers.length > 0 ? (
                <>
                  Yours are kept in <span className="font-mono">{CONFIG_FILE}</span>. A project’s own come from its config
                  file and are changed there.
                </>
              ) : undefined
            }
          >
            {servers.length === 0 ? (
              <div className="flex min-h-12 items-center gap-4 px-4 py-3">
                <p className="text-dense min-w-0 flex-1 text-muted">
                  A server gives the agent more tools. Yours are kept in <span className="font-mono">{CONFIG_FILE}</span>.
                </p>
                {!adding && (
                  <button onClick={() => setAdding(true)} className={btn("secondary", "sm", "shrink-0")}>
                    Add server
                  </button>
                )}
              </div>
            ) : (
              servers.map((s) => <Server key={s.name} server={s} directory={directory} onList={load.set} />)
            )}
          </Group>
        </>
        )
      }}
    </Loaded>
  )
}

function Server({
  server: s,
  directory,
  onList,
}: {
  server: McpServer
  directory: string
  /** Absent for a server that waits: nothing about it can change here until it is allowed. */
  onList?: (l: McpServer[]) => void
}) {
  const [busy, setBusy] = useState<null | "toggle" | "retry" | "signout">(null)
  const [error, setError] = useState<string | null>(null)
  // `starting`: the server's sign-in is being found (up to half a minute); then the browser's turn.
  const [signing, setSigning] = useState<{ url: string; instructions: string | null } | "starting" | null>(null)
  const waiting = useRef(0)
  const status = STATUS[s.status]
  const on = s.status !== "disabled"
  const waits = s.status === "waiting"
  const run = (what: NonNullable<typeof busy>, call: () => Promise<McpServer[]>) => {
    setBusy(what)
    setError(null)
    call()
      .then(onList!, (e) => setError(message(e)))
      .finally(() => setBusy(null))
  }
  const signIn = async () => {
    setError(null)
    const n = ++waiting.current
    setSigning("starting")
    try {
      const start = await currentBridge().mcpSignIn(directory, s.name)
      if (n !== waiting.current) return
      setSigning(start)
      openExternal(start.url)
      const list = await currentBridge().mcpSignInWait(directory, s.name)
      // Cancelled meanwhile: the window stopped waiting, so the answer goes unused.
      if (n !== waiting.current) return
      onList!(list)
    } catch (e) {
      if (n === waiting.current) setError(message(e))
    } finally {
      if (n === waiting.current) setSigning(null)
    }
  }
  const cancelSignIn = () => {
    waiting.current++
    setSigning(null)
    // The core stops waiting too, so a later sign-in starts clean.
    currentBridge()
      .mcpSignInCancel(directory, s.name)
      .catch(() => {})
  }
  const counts = [
    s.tools !== null && `${s.tools} tool${s.tools === 1 ? "" : "s"}`,
    s.prompts > 0 && `${s.prompts} prompt${s.prompts === 1 ? "" : "s"}`,
  ].filter(Boolean)
  return (
    <div className="flex flex-col gap-2.5 px-4 py-3">
      {/* With little room (or a confirm open), the actions wrap under the name. */}
      <div className="flex flex-wrap items-start gap-x-4 gap-y-2">
        <div className="flex min-w-[min(100%,14rem)] flex-1 flex-col gap-1">
          <div className="flex min-w-0 items-center gap-2">
            {s.status === "pending" ? (
              <Spinner className="size-3" />
            ) : (
              <span className={cn("size-1.5 shrink-0 rounded-full", status.mark)} aria-hidden />
            )}
            <span className="truncate text-sm">{s.name}</span>
            <span className="text-2xs shrink-0 text-muted">
              {[status.word, ...counts].join(" · ")}
            </span>
          </div>
          <span className="truncate font-mono text-xs text-muted" title={s.target}>
            {s.target}
          </span>
          {!s.editable && !waits && (
            <span className="text-2xs truncate text-muted" title={s.source}>
              Defined in <span className="font-mono">{tilde(s.source)}</span>
            </span>
          )}
        </div>
        <div className="ml-auto flex max-w-full items-center gap-1">
          {s.status === "failed" && (
            <button disabled={busy !== null} onClick={() => run("retry", () => currentBridge().mcpRetry(directory, s.name))} className={btn("secondary", "sm", "gap-1.5")}>
              {busy === "retry" && <Spinner className="size-3.5" />}
              Retry
            </button>
          )}
          {s.status === "needs_auth" && !signing && (
            <button onClick={() => void signIn()} className={btn("secondary", "sm")}>
              Sign in
            </button>
          )}
          {s.oauth && s.signedIn && s.status !== "needs_auth" && (
            <button
              disabled={busy !== null}
              onClick={() => run("signout", () => currentBridge().mcpSignOut(directory, s.name))}
              className={btn("ghost", "sm", "gap-1.5 text-muted")}
            >
              {busy === "signout" && <Spinner className="size-3.5" />}
              Sign out
            </button>
          )}
          {s.editable ? (
            <Confirm
              label="Remove"
              question={`Remove ${s.name}?`}
              action="Remove"
              run={async () => {
                try {
                  onList!(await currentBridge().mcpRemove(directory, s.name))
                } catch (e) {
                  return message(e)
                }
              }}
            />
          ) : (
            !waits && <OpenPath path={s.source} />
          )}
          {/* Off or on is the project's to say once its servers are allowed. */}
          {!waits && <span className="ml-2 flex">
            <Toggle
              label={on ? `Turn ${s.name} off` : `Turn ${s.name} on`}
              checked={on}
              disabled={busy !== null}
              onChange={(enable) => run("toggle", () => currentBridge().mcpEnable(directory, s.name, enable))}
            />
          </span>}
        </div>
      </div>
      {signing && (
        <div className="text-dense flex flex-wrap items-center gap-x-3 gap-y-1.5 rounded-lg bg-default/40 px-3 py-2">
          <Spinner />
          <span className="min-w-0 flex-1">
            {signing === "starting" ? `Finding how ${s.name} signs in…` : "Waiting for you to finish in the browser…"}
          </span>
          {signing !== "starting" && (
            <button onClick={() => openExternal(signing.url)} className={btn("ghost", "sm", "text-muted")}>
              Open the page again
            </button>
          )}
          <button onClick={cancelSignIn} className={btn("ghost", "sm")}>
            Cancel
          </button>
          {signing !== "starting" && signing.instructions && (
            <p className="text-2xs w-full text-muted select-text">{signing.instructions}</p>
          )}
        </div>
      )}
      {s.status === "failed" && s.error && (
        <p className="text-dense max-h-48 overflow-auto whitespace-pre-wrap text-danger select-text">{s.error}</p>
      )}
      {error && <p className="text-dense text-danger select-text">{error}</p>}
    </div>
  )
}

/**
 * A project file's servers that wait: they start once the user allows that file's servers, here
 * or on the conversation's card. One answer covers every server in the file.
 */
function ProjectServers({
  source,
  servers,
  item,
  directory,
}: {
  source: string
  servers: McpServer[]
  item?: TrustItem
  directory: string
}) {
  const [busy, setBusy] = useState<boolean | null>(null)
  const [error, setError] = useState<string | null>(null)
  const refused = item?.decision === "denied"
  const answer = (allow: boolean) => {
    if (!item) return
    setBusy(allow)
    setError(null)
    answerTrust(directory, [item], allow)
      .catch((e) => setError(message(e)))
      .finally(() => setBusy(null))
  }
  const n = servers.length
  return (
    <Group
      label={
        <>
          This project <span className="ml-2 font-mono font-normal">{shortPath(source, directory)}</span>
        </>
      }
      aside={<OpenPath path={source} small />}
    >
      <Row
        label={
          <span className="flex items-center gap-2">
            <span className={cn("size-1.5 shrink-0 rounded-full", refused ? "bg-muted/60" : "bg-warning")} aria-hidden />
            {refused ? "Not allowed" : "Waiting for you"}
          </span>
        }
        hint={
          refused
            ? `You didn’t allow ${n === 1 ? "its server" : "its servers"}, so ${n === 1 ? "it doesn’t" : "they don’t"} start. Allow if you trust this project.`
            : `${n === 1 ? "Its server starts" : `Its ${n} servers start`} on this computer once you allow ${n === 1 ? "it" : "them"}. Allow only if you trust this project.`
        }
      >
        {busy !== null && <Spinner />}
        {item && (
          <button disabled={busy !== null} onClick={() => answer(true)} className={btn(refused ? "secondary" : "primary", "sm")}>
            Allow
          </button>
        )}
        {item && !refused && (
          <button disabled={busy !== null} onClick={() => answer(false)} className={btn("ghost", "sm", "text-muted")}>
            Don’t allow
          </button>
        )}
      </Row>
      {servers.map((s) => (
        <Server key={s.name} server={s} directory={directory} />
      ))}
      {error && <p className="text-dense px-4 py-2.5 text-danger select-text">{error}</p>}
    </Group>
  )
}

// ---------------------------------------------------------------------------
// Adding one
// ---------------------------------------------------------------------------

type Pair = { key: string; value: string }

/** A command line as its words: spaces split, quotes keep a word together. */
function words(line: string): string[] {
  const out: string[] = []
  const re = /"((?:[^"\\]|\\.)*)"|'([^']*)'|(\S+)/g
  for (let m = re.exec(line); m; m = re.exec(line)) out.push(m[1] ?? m[2] ?? m[3])
  return out
}

function AddForm({
  directory,
  taken,
  onDone,
}: {
  directory: string
  taken: string[]
  onDone: (list: McpServer[] | null) => void
}) {
  const [name, setName] = useState("")
  const [kind, setKind] = useState<"remote" | "local">("remote")
  const [url, setUrl] = useState("")
  const [command, setCommand] = useState("")
  const [pairs, setPairs] = useState<{ remote: Pair[]; local: Pair[] }>({ remote: [], local: [] })
  const [tried, setTried] = useState(false)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const form = useRef<HTMLFormElement>(null)
  useEffect(() => {
    form.current?.scrollIntoView({ block: "nearest" })
    form.current?.querySelector("input")?.focus()
  }, [])

  const list = pairs[kind]
  const setList = (next: Pair[]) => setPairs((p) => ({ ...p, [kind]: next }))
  const problems = {
    name: !name.trim()
      ? "Give it a name."
      : !/^[A-Za-z0-9_-]+$/.test(name.trim())
        ? "Use letters, digits, - and _ only."
        : taken.includes(name.trim())
          ? `There’s already a server named ${name.trim()}.`
          : null,
    url:
      kind !== "remote"
        ? null
        : !url.trim()
          ? "Enter the server’s address."
          : !/^https?:\/\/\S+$/.test(url.trim())
            ? "The address starts with https:// (or http:// on this computer)."
            : null,
    command: kind !== "local" ? null : words(command).length === 0 ? "Enter the command that starts the server." : null,
    pairs: list.some((p) => !p.key.trim() && p.value.trim())
      ? kind === "remote"
        ? "A header needs a name."
        : "A variable needs a name."
      : null,
  }
  const valid = !Object.values(problems).some(Boolean)
  const record = (l: Pair[]) => Object.fromEntries(l.filter((p) => p.key.trim()).map((p) => [p.key.trim(), p.value]))

  const submit = () => {
    setTried(true)
    if (!valid) return
    const draft: McpDraft =
      kind === "remote"
        ? { name: name.trim(), kind, url: url.trim(), headers: record(list) }
        : { name: name.trim(), kind, command: words(command), environment: record(list) }
    setSaving(true)
    setError(null)
    currentBridge()
      .mcpAdd(directory, draft)
      .then(onDone, (e) => setError(message(e)))
      .finally(() => setSaving(false))
  }

  return (
    <form
      ref={form}
      onSubmit={(e) => {
        e.preventDefault()
        submit()
      }}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault()
          e.stopPropagation()
          onDone(null)
        }
      }}
      aria-label="Add an MCP server"
      className="flex scroll-my-6 flex-col gap-2"
    >
      <div className="flex min-h-6 items-end px-1">
        <h2 className="text-2xs font-medium tracking-wide text-muted">Add a server</h2>
      </div>
      <div className="flex flex-col gap-4 rounded-xl bg-surface p-4">
        <Field label="Name" error={tried ? problems.name : null}>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="github"
            spellCheck={false}
            aria-invalid={tried && !!problems.name}
            className={input}
          />
        </Field>
        <div className="flex items-center gap-3">
          <span className="text-2xs text-muted">Runs</span>
          <Segmented
            label="Where it runs"
            value={kind}
            onChange={setKind}
            options={[
              { id: "remote", label: "Remote", title: "A server at an address" },
              { id: "local", label: "Local", title: "A program this computer starts" },
            ]}
          />
        </div>
        {kind === "remote" ? (
          <Field label="Address" error={tried ? problems.url : null}>
            <input
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              placeholder="https://mcp.example.com/mcp"
              spellCheck={false}
              aria-invalid={tried && !!problems.url}
              className={cn(input, "font-mono")}
            />
          </Field>
        ) : (
          <Field label="Command" error={tried ? problems.command : null}>
            <input
              value={command}
              onChange={(e) => setCommand(e.target.value)}
              placeholder="npx -y @modelcontextprotocol/server-filesystem ~/notes"
              spellCheck={false}
              aria-invalid={tried && !!problems.command}
              className={cn(input, "font-mono")}
            />
          </Field>
        )}
        <Pairs
          label={kind === "remote" ? "Headers" : "Environment"}
          add={kind === "remote" ? "Add a header" : "Add a variable"}
          keyHint={kind === "remote" ? "Authorization" : "API_KEY"}
          list={list}
          onChange={setList}
          error={tried ? problems.pairs : null}
        />
        {error && <p className="text-dense text-danger select-text">{error}</p>}
        <div className="flex items-center gap-2">
          <span className="text-2xs mr-auto text-muted">
            Saved to <span className="font-mono">{CONFIG_FILE}</span>
          </span>
          <button type="button" onClick={() => onDone(null)} className={btn("ghost", "sm")}>
            Cancel
          </button>
          <button type="submit" disabled={saving} className={btn("primary", "sm", "gap-1.5")}>
            {saving && <Spinner className="size-3.5 text-accent-foreground" />}
            Add
          </button>
        </div>
      </div>
    </form>
  )
}

export const input =
  "text-dense h-8 w-full min-w-0 rounded-lg bg-default px-2.5 outline-none placeholder:font-sans placeholder:text-field-placeholder focus-visible:status-focused-field aria-invalid:bg-danger/10"

export function Field({ label, error, children }: { label: string; error: string | null; children: React.ReactNode }) {
  return (
    <label className="flex flex-col gap-1.5">
      <span className="text-2xs text-muted">{label}</span>
      {children}
      {error && <span className="text-2xs text-danger">{error}</span>}
    </label>
  )
}

function Pairs({
  label,
  add,
  keyHint,
  list,
  onChange,
  error,
}: {
  label: string
  add: string
  keyHint: string
  list: Pair[]
  onChange: (l: Pair[]) => void
  error: string | null
}) {
  return (
    <div className="flex flex-col gap-1.5">
      <span className="text-2xs text-muted">{label}</span>
      {list.map((p, i) => (
        <div key={i} className="flex items-center gap-2">
          <input
            value={p.key}
            onChange={(e) => onChange(list.map((x, j) => (j === i ? { ...x, key: e.target.value } : x)))}
            placeholder={keyHint}
            aria-label={`${label} name`}
            spellCheck={false}
            className={cn(input, "w-2/5 font-mono")}
          />
          <input
            value={p.value}
            onChange={(e) => onChange(list.map((x, j) => (j === i ? { ...x, value: e.target.value } : x)))}
            placeholder="Value"
            aria-label={`${label} value`}
            spellCheck={false}
            className={cn(input, "flex-1 font-mono")}
          />
          <Tooltip label="Remove">
            <button
              type="button"
              onClick={() => onChange(list.filter((_, j) => j !== i))}
              aria-label="Remove"
              className={btn("ghost", "sm", "shrink-0 text-muted", true)}
            >
              <X className="size-3.5" />
            </button>
          </Tooltip>
        </div>
      ))}
      <button
        type="button"
        onClick={() => onChange([...list, { key: "", value: "" }])}
        className={btn("ghost", "sm", "gap-1 self-start px-2 text-muted")}
      >
        <Plus className="size-3.5" />
        {add}
      </button>
      {error && <span className="text-2xs text-danger">{error}</span>}
    </div>
  )
}
