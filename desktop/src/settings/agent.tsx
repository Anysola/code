/**
 * What the agent works with in a folder, and where each piece comes from: permissions, skills,
 * commands, instructions and the agent's config file. Each reads the open session's folder, or
 * the app's own folder when none is open, and says which at the top.
 */
import { FolderOpen } from "lucide-react"
import { Fragment, useEffect, useState, type ReactNode } from "react"

import type { AgentConfig, AgentConfigPatch, Origin, PermissionRule, Repository, SavedPermission, Sources, TrustItem } from "../bridge"
import { APP_COMMANDS } from "../composer"
import { setPref, usePrefs } from "../prefs"
import { ruleText, scopeLabel, useTrust } from "../hooks-card"
import { currentBridge, loadWorktreeInfo, settingsFolder, useStore, useWorktreeInfo } from "../store"
import { btn, cn, folderKey, folderName, sameFolder, Spinner } from "../ui"
import {
  bytes,
  Confirm,
  day,
  FolderLine,
  Group,
  Loaded,
  Failed,
  message,
  Row,
  Section,
  Segmented,
  tilde,
  Toggle,
  useLoad,
  Value,
} from "./controls"
import { shellName, shortPath } from "./hooks"
import { EntryRow, ProjectDownloads, TrustRow } from "./trust"

/** The folder the agent sections read, and whether it is the app's own; or why it can't be read. */
function useFolder(): { directory: string; own: boolean } | { error: string; retry: () => void } | null {
  const open = useStore((s) => s.open?.directory ?? null)
  const scratch = useStore((s) => s.scratch)
  const [directory, setDirectory] = useState<string | null>(open)
  const [error, setError] = useState<string | null>(null)
  const [tries, setTries] = useState(0)
  useEffect(() => {
    setError(null)
    settingsFolder().then(setDirectory, (e) => setError(message(e)))
  }, [open, tries])
  if (error && !directory) return { error, retry: () => setTries((n) => n + 1) }
  return directory ? { directory, own: sameFolder(directory, scratch) } : null
}

/** Shows a file or folder in the file manager; a failure says so under the button. */
export function OpenPath({ path, label = "Open file", small = false }: { path: string; label?: string; small?: boolean }) {
  const [error, setError] = useState<string | null>(null)
  return (
    <span className="flex flex-col items-end">
      <button
        onClick={() => {
          setError(null)
          currentBridge()
            .revealPath(path)
            .catch((e) => setError(message(e)))
        }}
        title={path}
        className={btn("ghost", "sm", cn("shrink-0 gap-1.5 text-muted", small && "h-6 gap-1 px-2 text-2xs"))}
      >
        <FolderOpen className={small ? "size-3" : "size-3.5"} />
        {label}
      </button>
      {error && <span className="text-2xs max-w-56 text-right text-danger">{error}</span>}
    </span>
  )
}

/** A section that reads one folder: the folder line, then its content once known. */
export function FolderSection({ title, children }: { title: string; children: (directory: string) => ReactNode }) {
  const folder = useFolder()
  return (
    <Section title={title}>
      {folder && "error" in folder && <Failed error={folder.error} retry={folder.retry} bare />}
      {folder && "directory" in folder && <FolderLine directory={folder.directory} own={folder.own} />}
      {folder && "directory" in folder && children(folder.directory)}
    </Section>
  )
}

// ---------------------------------------------------------------------------
// Permissions
// ---------------------------------------------------------------------------

const ACTIONS: Record<string, string> = {
  shell: "Run",
  edit: "Edit",
  write: "Write",
  read: "Read",
  webfetch: "Fetch",
  websearch: "Search the web",
  external_directory: "Work outside the folder",
}

const EFFECT = { allow: "Allow", ask: "Ask", deny: "Deny" } as const

const MODES: [string, string][] = [
  ["Ask", "Every edit and every command waits for your OK."],
  ["Edits", "Files change without asking; commands wait for your OK."],
  ["Auto", "Everything runs without asking, except what a rule below says to ask about or refuse."],
  ["Plan", "The agent reads and writes a plan; nothing changes until you approve it."],
]

export function PermissionsSection() {
  return (
    <FolderSection title="Permissions">
      {(directory) => <Permissions directory={directory} />}
    </FolderSection>
  )
}

function Permissions({ directory }: { directory: string }) {
  // An answer on a project's rules changes what is in effect: read again.
  const changed = useStore((s) => s.trustChanged)
  const load = useLoad(() => currentBridge().permissions(directory), [directory, changed])
  const trust = useTrust(directory)
  const items = (trust.data ?? []).filter((i) => i.kind === "permissions")
  return (
    <>
      <section className="flex flex-col gap-2 px-1">
        <h2 className="text-2xs font-medium tracking-wide text-muted">Modes</h2>
        <p className="text-dense text-muted">
          You choose the mode in the message box. It decides what the agent may do without asking you first.
        </p>
        <dl className="text-dense grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5">
          {MODES.map(([name, text]) => (
            <Fragment key={name}>
              <dt className="text-foreground/85">{name}</dt>
              <dd className="text-muted">{text}</dd>
            </Fragment>
          ))}
        </dl>
        <p className="text-2xs text-muted">The rules below apply in every mode: a rule can still ask, or refuse.</p>
      </section>
      <Loaded load={load} rows={3}>
        {({ saved, rules }) => {
          const files = ruleFiles(rules, items)
          // A project file with allow rules to answer for stays at the top whatever the answer,
          // so answering never moves it away from under the pointer.
          const approvable = files.filter((f) => f.item)
          return (
            <>
              {approvable.map((f) => (
                <RuleFile key={f.source} file={f} directory={directory} />
              ))}
              <Saved saved={saved} directory={directory} onChange={(next) => load.set({ saved: next, rules })} />
              <Rules files={files.filter((f) => !f.item)} directory={directory} />
            </>
          )
        }}
      </Loaded>
    </>
  )
}

/** A file's rules, and its project's answer on them when it has allow rules to approve. */
type RuleFileData = { source: string; rules: PermissionRule[]; item?: TrustItem }

/** The rules by file, in load order; a file whose approval covers no listed rule (a policy only) after them. */
function ruleFiles(rules: PermissionRule[], items: TrustItem[]): RuleFileData[] {
  const files = new Map<string, RuleFileData>()
  for (const r of rules) {
    const key = folderKey(r.source)
    const file = files.get(key) ?? { source: r.source, rules: [] }
    file.rules.push(r)
    files.set(key, file)
  }
  for (const i of items) {
    const key = folderKey(i.path)
    files.set(key, { ...(files.get(key) ?? { source: i.path, rules: [] }), item: i })
  }
  return [...files.values()]
}

function Saved({
  saved,
  directory,
  onChange,
}: {
  saved: SavedPermission[]
  directory: string
  onChange: (next: SavedPermission[]) => void
}) {
  const [error, setError] = useState<string | null>(null)
  const projects = new Map<string, SavedPermission[]>()
  for (const p of saved) projects.set(p.projectId, [...(projects.get(p.projectId) ?? []), p])
  // The folder open now comes first.
  const order = [...projects.entries()].sort(
    ([, a], [, b]) => Number(sameFolder(b[0].folder, directory)) - Number(sameFolder(a[0].folder, directory))
  )
  if (order.length === 0)
    return (
      <Group label="Always allowed">
        <Row label="No saved answers" hint="“Always allow” on a request saves it here, for its project" />
      </Group>
    )
  return (
    <>
      {order.map(([projectId, list]) => {
        const folder = list[0].folder
        return (
          <Group
            key={projectId}
            label={
              <span title={folder ?? undefined}>
                Always allowed in {folder ? folderName(folder) : "a project whose folder is gone"}
              </span>
            }
            aside={
              <Confirm
                label="Remove all"
                question={`Remove all ${list.length}?`}
                action="Remove all"
                className="h-6 px-2 text-2xs"
                run={async () => {
                  try {
                    onChange(await currentBridge().permissionForgetAll(projectId))
                  } catch (e) {
                    return message(e)
                  }
                }}
              />
            }
          >
            {list.map((p) => (
              <Row
                key={`${p.action}:${p.resource}`}
                label={<span className="font-mono text-xs break-all">{p.resource}</span>}
                hint={ACTIONS[p.action] ?? p.action}
              >
                <button
                  onClick={() => {
                    setError(null)
                    currentBridge()
                      .permissionForget(projectId, p.action, p.resource)
                      .then(onChange, (e) => setError(message(e)))
                  }}
                  className={btn("ghost", "sm", "text-muted")}
                >
                  Remove
                </button>
              </Row>
            ))}
          </Group>
        )
      })}
      {error && <p className="text-dense -mt-6 px-1 text-danger">{error}</p>}
    </>
  )
}

function Rules({ files, directory }: { files: RuleFileData[]; directory: string }) {
  if (files.length === 0)
    return (
      <Group label="Rules in effect">
        <Row label="No rules" hint="Rules live under “permissions” in the agent’s config file" />
      </Group>
    )
  return (
    <>
      {files.map((f) => (
        <RuleFile key={f.source} file={f} directory={directory} />
      ))}
    </>
  )
}

const PERMISSIONS_HINT = {
  waiting: "Its allow rules let the agent do more without asking you. Until you allow them, your own rules decide.",
  allowed: "Its allow rules are in effect. They never undo a rule of yours that refuses.",
  denied: "Its allow rules aren’t applied: your own rules decide. Allow if you trust this project.",
}

/** "The Build agent" for `build`. */
const agentName = (id: string) => `the ${id.charAt(0).toUpperCase()}${id.slice(1)} agent`

function RuleFile({ file: f, directory }: { file: RuleFileData; directory: string }) {
  const project = !!f.item || f.rules.some((r) => r.project)
  const isFile = /[\\/]/.test(f.source)
  // What the approval covers that is no permission rule: policy statements, the agent new sessions start with.
  const extra = (f.item?.entries ?? []).filter((e) => e.label === "policy" || e.label === "sessions")
  return (
    <Group
      label={
        project ? (
          <>
            This project <span className="ml-2 font-mono font-normal">{shortPath(f.source, directory)}</span>
          </>
        ) : (
          <>
            Rules in <span className="font-mono">{tilde(f.source)}</span>
          </>
        )
      }
      aside={isFile ? <OpenPath path={f.source} small /> : undefined}
    >
      {f.item && <TrustRow item={f.item} directory={directory} hint={PERMISSIONS_HINT[f.item.decision ?? "waiting"]} />}
      {f.rules.map((r, i) => (
        <Row
          key={i}
          label={<span className="font-mono text-xs break-all">{r.resource}</span>}
          hint={
            <>
              {ACTIONS[r.action] ?? r.action}
              {r.agent && <> · Only {agentName(r.agent)}</>}
            </>
          }
          className="min-h-11 py-2"
        >
          {r.applied ? (
            <span
              className={cn(
                "text-dense",
                r.effect === "deny" ? "text-danger" : r.effect === "ask" ? "text-foreground" : "text-muted"
              )}
            >
              {EFFECT[r.effect]}
            </span>
          ) : (
            <span className="text-dense text-muted/70" title="Not in effect until you allow this file’s rules">
              {EFFECT[r.effect]} · {f.item?.decision === "denied" ? "not applied" : "waiting"}
            </span>
          )}
        </Row>
      ))}
      {extra.map((e, i) => (
        <EntryRow key={`x${i}`} scope={scopeLabel(e.label)} text={ruleText(e.command, e.label)} />
      ))}
      {f.rules.length === 0 && extra.length === 0 && <Row label="No rules" />}
    </Group>
  )
}

// ---------------------------------------------------------------------------
// Skills
// ---------------------------------------------------------------------------

const ORIGIN: Partial<Record<Origin, string>> = {
  personal: "Personal",
  project: "This project",
  claude: "Claude Code",
  agents: ".agents",
  legacy: "Older config",
  url: "From a URL",
  config: "Config file",
}

const SKILL_ORDER: Origin[] = ["personal", "project", "claude", "agents", "legacy", "url", "config"]

function parent(path: string, up = 1) {
  let p = path
  for (let i = 0; i < up; i++) p = p.replace(/[\\/][^\\/]*$/, "")
  return p
}

export function SkillsSection() {
  return <FolderSection title="Skills">{(directory) => <Skills directory={directory} />}</FolderSection>
}

/** What the folder reads, read again when an answer on a project file changes it. */
function useSources(directory: string) {
  const changed = useStore((s) => s.trustChanged)
  return useLoad(() => currentBridge().sources(directory), [directory, changed])
}

/** Tries the failed downloads again; the list it answers with replaces the one shown. */
function useRetry(directory: string, set: (s: Sources) => void) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const retry = () => {
    setBusy(true)
    setError(null)
    currentBridge()
      .sourcesRetry(directory)
      .then(set, (e) => setError(message(e)))
      .finally(() => setBusy(false))
  }
  return { busy, error, retry }
}

function RetryButton({ busy, onClick }: { busy: boolean; onClick: () => void }) {
  return (
    <button disabled={busy} onClick={onClick} className={btn("secondary", "sm", "gap-1.5 shrink-0")}>
      {busy && <Spinner className="size-3.5" />}
      Try again
    </button>
  )
}

function Skills({ directory }: { directory: string }) {
  const claude = usePrefs((p) => p.sources.claudeSkills)
  const load = useSources(directory)
  return (
    <>
      <Group>
        <Row label="Read Claude Code skills" hint="From ~/.claude/skills and a project’s .claude/skills" id="claude-skills">
          <Toggle
            labelledBy="claude-skills"
            checked={claude}
            // Read again once the file holds the switch, so the list is what the agent now reads.
            onChange={(claudeSkills) => void setPref("sources", { claudeSkills }).then((ok) => ok && load.reload())}
          />
        </Row>
      </Group>
      <Loaded load={load} rows={4}>
        {(s) => <SkillList sources={s} directory={directory} onList={load.set} />}
      </Loaded>
    </>
  )
}

function SkillList({ sources, directory, onList }: { sources: Sources; directory: string; onList: (s: Sources) => void }) {
  const groups = SKILL_ORDER.map((o) => [o, sources.skills.filter((s) => s.origin === o)] as const).filter(
    ([, list]) => list.length > 0
  )
  const places = sources.places.filter((p) => p.holds === "skills")
  return (
    <>
      {groups.length === 0 && (
        <Group label="Skills">
          <Row label="No skills found" hint="A skill is a folder with a SKILL.md, in one of the places below" />
        </Group>
      )}
      {groups.map(([origin, list]) => {
        // A skill is <folder>/<name>/SKILL.md: the group's folder is two up.
        const folder = origin === "url" ? null : parent(list[0].path, 2)
        return (
          <Group
            key={origin}
            label={
              <>
                {ORIGIN[origin] ?? origin}
                {folder && <span className="ml-2 font-mono font-normal">{tilde(folder)}</span>}
              </>
            }
            aside={folder ? <OpenPath path={folder} label="Open folder" small /> : undefined}
          >
            {list.map((s) => (
              <Row key={s.path} label={s.name} hint={s.description ?? "No description"} className="min-h-11 py-2" />
            ))}
          </Group>
        )
      })}
      <SkillUrls sources={sources} directory={directory} onList={onList} />
      <Places label="Where skills are looked for" places={places} />
    </>
  )
}

/** Each `skills` URL and whether its list of skills could be downloaded. */
function SkillUrls({ sources, directory, onList }: { sources: Sources; directory: string; onList: (s: Sources) => void }) {
  const { busy, error, retry } = useRetry(directory, onList)
  if (sources.skillUrls.length === 0) return null
  return (
    <Group
      label="Downloaded from"
      note={error ? <span className="text-danger">{error}</span> : "Checked when you open this folder and again when its config changes."}
    >
      {sources.skillUrls.map((u) => (
        <Row
          key={u.url}
          label={<span className="font-mono text-xs break-all">{u.url}</span>}
          hint={u.error ? <span className="text-danger">Couldn’t download: {u.error}. Its skills aren’t offered.</span> : undefined}
          className="min-h-11 py-2"
        >
          {u.error ? <RetryButton busy={busy} onClick={retry} /> : <Value>Downloaded</Value>}
        </Row>
      ))}
    </Group>
  )
}

function Places({ label, places }: { label: string; places: Sources["places"] }) {
  if (places.length === 0) return null
  return (
    <Group label={label}>
      {places.map((p) => (
        <Row
          key={p.path}
          label={<span className={cn("font-mono text-xs break-all", !p.exists && "text-muted")}>{tilde(p.path)}</span>}
          className="min-h-10 py-2"
        >
          <Value>
            {ORIGIN[p.origin] ?? p.origin} · {p.exists ? "Found" : "Not there"}
          </Value>
        </Row>
      ))}
    </Group>
  )
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

export function CommandsSection() {
  return <FolderSection title="Commands">{(directory) => <Commands directory={directory} />}</FolderSection>
}

function Commands({ directory }: { directory: string }) {
  const load = useSources(directory)
  return (
    <Loaded load={load} rows={5}>
      {(s) => {
        const builtin = s.commands.filter((c) => c.origin === "builtin")
        const mcp = s.commands.filter((c) => c.origin === "mcp")
        const files = s.commands.filter((c) => c.origin !== "builtin" && c.origin !== "mcp" && c.origin !== "app")
        return (
          <>
            <Group label="The app’s own" note="These run in the window and never reach the model.">
              {APP_COMMANDS.map((c) => (
                <CommandRow key={c.name} name={c.name} description={c.detail} />
              ))}
            </Group>
            <Group label="Built in">
              {builtin.length === 0 && <Row label="None" />}
              {builtin.map((c) => (
                <CommandRow key={c.name} name={c.name} description={c.description} />
              ))}
            </Group>
            <Group
              label="From files"
              note={files.length === 0 ? "A Markdown file in ~/.anysola/commands or a project’s .anysola/commands becomes a command." : undefined}
            >
              {files.length === 0 && <Row label="None yet" />}
              {files.map((c) => (
                <CommandRow
                  key={c.name}
                  name={c.name}
                  description={c.description}
                  origin={c.source ? <span className="font-mono text-xs">{tilde(c.source)}</span> : ORIGIN[c.origin]}
                />
              ))}
            </Group>
            {mcp.length > 0 && (
              <Group label="From MCP servers">
                {mcp.map((c) => (
                  <CommandRow key={`${c.source}:${c.name}`} name={c.name} description={c.description} origin={c.source} />
                ))}
              </Group>
            )}
          </>
        )
      }}
    </Loaded>
  )
}

function CommandRow({ name, description, origin }: { name: string; description: string | null; origin?: ReactNode }) {
  return (
    <div className="flex min-h-11 items-center gap-4 px-4 py-2">
      <span className="w-36 shrink-0 truncate font-mono text-xs">/{name}</span>
      <span className="text-dense min-w-0 flex-1 text-muted">{description ?? "No description"}</span>
      {origin && <span className="text-2xs max-w-48 shrink-0 truncate text-right text-muted">{origin}</span>}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Instructions
// ---------------------------------------------------------------------------

export function InstructionsSection() {
  return (
    <FolderSection title="Instructions">
      {(directory) => (
        <>
          <Personal />
          <Downloads directory={directory} />
          <InEffect directory={directory} />
        </>
      )}
    </FolderSection>
  )
}

function Personal() {
  const load = useLoad(() => currentBridge().personalInstructions(), [])
  const [text, setText] = useState<string | null>(null)
  const [saving, setSaving] = useState(false)
  const [state, setState] = useState<"saved" | string | null>(null)
  const saved = load.data?.text ?? ""
  const value = text ?? saved
  const changed = value !== saved
  return (
    <Loaded load={load} rows={3}>
      {(p) => (
        <Group
          label="Personal instructions"
          aside={<OpenPath path={p.path} small />}
          note={
            <>
              <span className="font-mono">{tilde(p.path)}</span>. Applies to every folder.
            </>
          }
        >
          <div className="flex flex-col gap-2.5 p-3">
            <textarea
              value={value}
              onChange={(e) => {
                setText(e.target.value)
                setState(null)
              }}
              rows={8}
              spellCheck={false}
              placeholder="What the agent should know or do in every folder"
              aria-label="Personal instructions"
              className="text-dense block max-h-[min(24rem,45vh)] min-h-32 w-full resize-y rounded-lg bg-default px-3 py-2.5 font-mono leading-relaxed outline-none placeholder:font-sans placeholder:text-field-placeholder focus-visible:status-focused-field"
            />
            <div className="flex items-center gap-2">
              <span className={cn("text-2xs mr-auto", state && state !== "saved" ? "text-danger" : "text-muted")}>
                {state && state !== "saved" ? state : changed ? "Unsaved changes" : state === "saved" ? "Saved" : `${value.length} characters`}
              </span>
              <button disabled={!changed || saving} onClick={() => setText(null)} className={btn("ghost", "sm")}>
                Revert
              </button>
              <button
                disabled={!changed || saving}
                onClick={() => {
                  setSaving(true)
                  currentBridge()
                    .savePersonalInstructions(value)
                    .then(
                      () => {
                        load.set({ ...p, text: value })
                        setText(null)
                        setState("saved")
                      },
                      (e) => setState(message(e))
                    )
                    .finally(() => setSaving(false))
                }}
                className={btn("primary", "sm", "gap-1.5")}
              >
                {saving && <Spinner className="size-3.5 text-accent-foreground" />}
                Save
              </button>
            </div>
          </div>
        </Group>
      )}
    </Loaded>
  )
}

/** A project's downloads, waiting, allowed or not. */
function Downloads({ directory }: { directory: string }) {
  const trust = useTrust(directory)
  return <ProjectDownloads directory={directory} items={(trust.data ?? []).filter((i) => i.kind === "remote")} />
}

function InEffect({ directory }: { directory: string }) {
  const load = useSources(directory)
  const { busy, error, retry } = useRetry(directory, load.set)
  // A repository downloads in the background and nothing says when it is done: look again meanwhile.
  const downloading = !!load.data?.repositories.some((r) => !r.cloned && !r.error)
  const { reload } = load
  useEffect(() => {
    if (!downloading) return
    const t = setInterval(reload, 3000)
    return () => clearInterval(t)
  }, [downloading, reload])
  return (
    <Loaded load={load} rows={3}>
      {(s) => {
        let n = 0
        return (
          <>
            <Group
              label="Read for this folder, in this order"
              note={error ? <span className="text-danger">{error}</span> : undefined}
            >
              {s.instructions.length === 0 && <Row label="No instruction files" hint="An AGENTS.md in the folder is read first" />}
              {s.instructions.map((f) => (
                <Row
                  key={f.path}
                  label={
                    <span className="flex min-w-0 items-baseline gap-2.5">
                      {/* One that couldn't be downloaded isn't read: no place in the order. */}
                      <span className="text-2xs w-3 shrink-0 text-muted tabular-nums">{f.error ? "" : ++n}</span>
                      <span className={cn("font-mono text-xs break-all", f.error && "text-muted")}>{tilde(f.path)}</span>
                    </span>
                  }
                  hint={
                    f.error ? (
                      <span className="block pl-5.5 text-danger">Couldn’t download: {f.error}. It isn’t read.</span>
                    ) : undefined
                  }
                  className="min-h-11 py-2"
                >
                  {f.error ? (
                    <RetryButton busy={busy} onClick={retry} />
                  ) : (
                    <>
                      <Value>{bytes(f.bytes)}</Value>
                      {f.origin !== "url" && <OpenPath path={f.path} />}
                    </>
                  )}
                </Row>
              ))}
            </Group>
            <Repositories list={s.repositories} busy={busy} retry={retry} />
            <Places label="Looked in" places={s.places.filter((p) => p.holds === "instructions")} />
          </>
        )
      }}
    </Loaded>
  )
}

/** The git repositories the agent may read (config `references`), and how each download stands. */
function Repositories({ list, busy, retry }: { list: Repository[]; busy: boolean; retry: () => void }) {
  if (list.length === 0) return null
  return (
    <Group
      label="Repositories the agent can read"
      note="Downloaded when you open this folder; each copy is updated at most once a day."
    >
      {list.map((r) => (
        <Row
          key={r.name}
          label={<span className="font-mono text-xs break-all">@{r.name}</span>}
          hint={
            <>
              {r.repository && (
                <span className="block font-mono text-xs break-all">
                  {r.repository}
                  {r.branch && ` · ${r.branch}`}
                </span>
              )}
              {r.error && (
                <span className="block text-danger">
                  {r.cloned ? `Couldn’t update: ${r.error}. The copy from before is used.` : `Couldn’t download: ${r.error}.`}
                </span>
              )}
            </>
          }
          className="min-h-11 py-2"
        >
          {r.error && !r.cloned ? (
            <RetryButton busy={busy} onClick={retry} />
          ) : r.cloned ? (
            <Value>{r.attemptedAt ? `Updated ${day(r.attemptedAt)}` : "Downloaded"}</Value>
          ) : (
            <span className="text-dense flex items-center gap-2 text-muted">
              <Spinner />
              Downloading
            </span>
          )}
        </Row>
      ))}
    </Group>
  )
}

// ---------------------------------------------------------------------------
// Advanced
// ---------------------------------------------------------------------------

/** The config key each setting is stored under, as `overridden` may name it. */
const KEYS: Record<keyof AgentConfigPatch, string[]> = {
  model: ["model"],
  websearch: ["websearch"],
  todo: ["todo", "experimental.todo"],
  subagentDepth: ["subagentDepth", "subagent_depth", "experimental.subagent_depth"],
  autoCompact: ["autoCompact", "compaction.auto", "compaction"],
  snapshots: ["snapshots", "snapshot"],
  shell: ["shell"],
  lsp: ["lsp"],
  sandbox: ["sandbox"],
}

export function AdvancedSection() {
  return <FolderSection title="Advanced">{(directory) => <Advanced directory={directory} />}</FolderSection>
}

function Advanced({ directory }: { directory: string }) {
  const load = useLoad(() => currentBridge().configGet(directory), [directory])
  const [error, setError] = useState<string | null>(null)
  return (
    <Loaded load={load} rows={6}>
      {(c) => {
        const change = (patch: AgentConfigPatch) => {
          setError(null)
          load.set({ ...c, ...patch })
          currentBridge()
            .configSet(directory, patch)
            .then(load.set, (e) => {
              load.set(c)
              setError(message(e))
            })
        }
        const over = (key: keyof AgentConfigPatch) => {
          const o = c.overridden.find((x) => KEYS[key].includes(x.key))
          return o ? <span className="text-warning">Overridden in {tilde(o.file)} for this folder</span> : undefined
        }
        return (
          <>
            <Group
              label={
                <>
                  Agent config <span className="ml-2 font-mono font-normal">{tilde(c.file)}</span>
                </>
              }
              aside={<OpenPath path={c.file} label="Open config file" small />}
              note={error ? <span className="text-danger">{error}</span> : undefined}
            >
              <ConfigToggle id="websearch" label="Web search" hint={over("websearch")} checked={c.websearch} onChange={(websearch) => change({ websearch })} />
              <ConfigToggle id="todo" label="Task list" hint={over("todo") ?? "The agent keeps a list of steps for longer work"} checked={c.todo} onChange={(todo) => change({ todo })} />
              <Row
                label="Subagent levels"
                hint={
                  over("subagentDepth") ??
                  "The agent can hand parts of a task to subagents. More levels let subagents start their own."
                }
              >
                <Segmented
                  label="Subagent levels"
                  value={String(c.subagentDepth) as "1" | "2" | "3"}
                  onChange={(d) => change({ subagentDepth: Number(d) })}
                  options={[
                    { id: "1", label: "1" },
                    { id: "2", label: "2" },
                    { id: "3", label: "3" },
                  ]}
                />
              </Row>
              <ConfigToggle
                id="compact"
                label="Summarize when the context fills"
                hint={over("autoCompact") ?? "Off: a full session stops and asks you to /compact"}
                checked={c.autoCompact}
                onChange={(autoCompact) => change({ autoCompact })}
              />
              <ConfigToggle
                id="snapshots"
                label="Keep snapshots for rewinding"
                hint={over("snapshots") ?? "Off: a rewind can’t put files back"}
                checked={c.snapshots}
                onChange={(snapshots) => change({ snapshots })}
              />
              <Shell config={c} hint={over("shell")} onChange={(shell) => change({ shell })} />
              <ConfigToggle
                id="lsp"
                label="Language server diagnostics"
                hint={
                  over("lsp") ??
                  "On for projects you allow; turn off to disable everywhere. Language servers run a project’s own config, plugins, and build tooling on edited files."
                }
                checked={c.lsp}
                onChange={(lsp) => change({ lsp })}
              />
              <ConfigToggle
                id="sandbox"
                label="Shell sandbox"
                hint={
                  c.sandbox && c.sandboxUnavailable ? (
                    <span className="text-warning">On, but it can’t start here, so no shell command runs. {c.sandboxUnavailable}</span>
                  ) : (
                    over("sandbox") ??
                    (c.sandboxUnavailable ? (
                      `Experimental: shell commands could write only inside the session’s folder. ${c.sandboxUnavailable}`
                    ) : (
                      "Experimental, Linux only: shell commands, scheduled checks, and template commands run in a private PID/session namespace and write only inside the session’s folder (plus a session cache); .git stays read-only so Git commits don’t work inside it."
                    ))
                  )
                }
                checked={c.sandbox}
                // Off stays reachable: a sandbox that's on but can't start stops every command.
                disabled={!!c.sandboxUnavailable && !c.sandbox}
                onChange={(sandbox) => change({ sandbox })}
              />
            </Group>
            <WorktreeSetup directory={directory} />
            <ProjectLsp directory={directory} />
            <ProjectFormatters directory={directory} />
            {c.warnings.length > 0 && (
              <Group label="The config loader says">
                {c.warnings.map((w, i) => (
                  <p key={i} className="text-dense px-4 py-2.5 whitespace-pre-wrap text-warning select-text">
                    {w}
                  </p>
                ))}
              </Group>
            )}
          </>
        )
      }}
    </Loaded>
  )
}

/**
 * The folder's setup command for new worktrees (install, build): the user's own, kept in the
 * app's preferences by folder, never read from the repository. Only for git folders.
 */
function WorktreeSetup({ directory }: { directory: string }) {
  const info = useWorktreeInfo(directory)
  const key = folderKey(directory)
  const saved = usePrefs((p) => p.worktreeSetup[key] ?? "")
  const [text, setText] = useState(saved)
  useEffect(() => setText(saved), [saved])
  useEffect(() => {
    if (!info) void loadWorktreeInfo(directory)
  }, [directory, info])
  if (!info?.available) return null
  const commit = () => {
    const next = text.trim()
    // A blank one is no command (the file drops it).
    if (next !== saved) void setPref("worktreeSetup", { [key]: next })
  }
  return (
    <Group
      label="Worktrees"
      note="New sessions here work in their own worktree. Ignored .env files are copied in; anything else they need, this command makes. A failed run doesn’t stop the session."
    >
      <div className="flex flex-col gap-2 px-4 py-3">
        <div className="flex flex-col gap-0.5">
          <span id="worktree-setup" className="text-sm">
            Setup command
          </span>
          <span className="text-dense text-muted">Runs in each new worktree before the first message</span>
        </div>
        <input
          value={text}
          onChange={(e) => setText(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => e.key === "Enter" && commit()}
          placeholder="pnpm install"
          aria-labelledby="worktree-setup"
          spellCheck={false}
          className="text-dense h-8 min-w-0 rounded-lg bg-default px-2.5 font-mono outline-none placeholder:text-field-placeholder focus-visible:status-focused-field"
        />
      </div>
    </Group>
  )
}

const LSP_HINT = {
  waiting: "Waiting for your answer. Until then, no language server starts for this folder.",
  allowed: "Allowed. Installed language servers report diagnostics on edited files in this folder.",
  denied: "Not allowed. No language server starts for this folder.",
}

/** Installed language servers detected for this folder: they start only once allowed. */
function ProjectLsp({ directory }: { directory: string }) {
  const trust = useTrust(directory)
  const items = (trust.data ?? []).filter((i) => i.kind === "lsp")
  if (items.length === 0) return null
  return (
    <Group label="This project’s language servers">
      {items.map((i) => (
        <Fragment key={i.path}>
          <TrustRow
            item={i}
            directory={directory}
            label={<span className="font-mono text-xs break-all">{shortPath(i.path, directory)}</span>}
            hint={LSP_HINT[i.decision ?? "waiting"]}
          />
          {i.entries.map((e, n) => (
            <EntryRow
              key={n}
              scope={e.label}
              text={
                <span className="flex flex-col gap-0.5">
                  <span className="font-mono text-xs break-all">{e.command}</span>
                  {e.detail && <span className="text-2xs font-mono break-all text-muted">{e.detail}</span>}
                </span>
              }
            />
          ))}
        </Fragment>
      ))}
    </Group>
  )
}

const FORMATTER_HINT = {
  waiting: "Waiting for your answer. Until then, edits aren’t formatted the way this file says.",
  allowed: "Allowed. Edits are formatted the way this file says.",
  denied: "Not allowed. Edits aren’t formatted the way this file says.",
}

/** A project's formatters: they run on edits only once allowed. */
function ProjectFormatters({ directory }: { directory: string }) {
  const trust = useTrust(directory)
  const items = (trust.data ?? []).filter((i) => i.kind === "formatter")
  if (items.length === 0) return null
  return (
    <Group label="This project’s formatters">
      {items.map((i) => (
        <Fragment key={i.path}>
          <TrustRow
            item={i}
            directory={directory}
            label={<span className="font-mono text-xs break-all">{shortPath(i.path, directory)}</span>}
            hint={FORMATTER_HINT[i.decision ?? "waiting"]}
          />
          {/* What the answer covers: each formatter and what it runs. */}
          {i.entries.map((e, n) => (
            <EntryRow
              key={n}
              scope={e.label === "formatter" ? "Formatters" : e.label}
              text={
                /^the built-in /.test(e.command) ? (
                  <span className="text-muted">{e.command.charAt(0).toUpperCase() + e.command.slice(1)}</span>
                ) : (
                  <span className="font-mono text-xs break-all">{e.command}</span>
                )
              }
            />
          ))}
        </Fragment>
      ))}
    </Group>
  )
}

function ConfigToggle({
  id,
  label,
  hint,
  checked,
  disabled,
  onChange,
}: {
  id: string
  label: string
  hint?: ReactNode
  checked: boolean
  disabled?: boolean
  onChange: (on: boolean) => void
}) {
  return (
    <Row label={label} hint={hint} id={`cfg-${id}`}>
      <Toggle labelledBy={`cfg-${id}`} checked={checked} disabled={disabled} onChange={onChange} />
    </Row>
  )
}

/** What runs commands, and hooks when they run elsewhere (Windows: Git Bash when installed). */
function shellHint(config: AgentConfig, custom: boolean): string | undefined {
  const hooks = config.hooksShellDetected ?? null
  const active = custom ? config.shell : config.shellDetected
  const differ = hooks !== null && active !== null && shellName(hooks) !== shellName(active)
  if (custom) return differ ? `Hooks run in ${shellName(hooks)}` : undefined
  if (!config.shellDetected) return "Picks one when a command runs"
  return differ ? `Uses ${config.shellDetected}; hooks run in ${shellName(hooks)}` : `Uses ${config.shellDetected}`
}

function Shell({ config, hint, onChange }: { config: AgentConfig; hint?: ReactNode; onChange: (shell: string | null) => void }) {
  const [custom, setCustom] = useState(config.shell !== null)
  const [path, setPath] = useState(config.shell ?? "")
  useEffect(() => {
    setCustom(config.shell !== null)
    setPath(config.shell ?? "")
  }, [config.shell])
  const commit = () => {
    const p = path.trim()
    if (p && p !== config.shell) onChange(p)
  }
  return (
    <>
      <Row
        label="Shell"
        hint={hint ?? shellHint(config, custom)}
      >
        <Segmented
          label="Shell"
          value={custom ? "custom" : "auto"}
          onChange={(v) => {
            if (v === "auto") {
              setCustom(false)
              if (config.shell !== null) onChange(null)
            } else setCustom(true)
          }}
          options={[
            { id: "auto", label: "Automatic" },
            { id: "custom", label: "A path" },
          ]}
        />
      </Row>
      {custom && (
        <div className="flex items-center gap-2 px-4 py-2.5">
          <input
            autoFocus={config.shell === null}
            value={path}
            onChange={(e) => setPath(e.target.value)}
            onBlur={commit}
            onKeyDown={(e) => e.key === "Enter" && commit()}
            placeholder={config.shellDetected ?? "/bin/bash"}
            aria-label="Shell path"
            spellCheck={false}
            className="text-dense h-8 min-w-0 flex-1 rounded-lg bg-default px-2.5 font-mono outline-none placeholder:text-field-placeholder focus-visible:status-focused-field"
          />
        </div>
      )}
    </>
  )
}
