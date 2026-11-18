/**
 * What the agent works with in a folder, and where each piece comes from: permissions, skills,
 * commands, instructions and the agent's config file. Each reads the open session's folder, or
 * the app's own folder when none is open, and says which at the top.
 */
import { FolderOpen } from "lucide-react"
import { Fragment, useEffect, useState, type ReactNode } from "react"

import type { AgentConfig, AgentConfigPatch, Origin, PermissionRule, SavedPermission, Sources } from "../bridge"
import { APP_COMMANDS } from "../composer"
import { setPref, usePrefs } from "../prefs"
import { answerTrust, useTrust } from "../hooks-card"
import { currentBridge, settingsFolder, useStore } from "../store"
import { btn, cn, folderName, sameFolder, Spinner } from "../ui"
import {
  bytes,
  Confirm,
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
import { shortPath } from "./hooks"

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
  const load = useLoad(() => currentBridge().permissions(directory), [directory])
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
        {({ saved, rules }) => (
          <>
            <Saved
              saved={saved}
              directory={directory}
              onChange={(next) => load.set({ saved: next, rules })}
            />
            <Rules rules={rules} />
          </>
        )}
      </Loaded>
    </>
  )
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

function Rules({ rules }: { rules: PermissionRule[] }) {
  const files = new Map<string, PermissionRule[]>()
  for (const r of rules) files.set(r.source, [...(files.get(r.source) ?? []), r])
  if (rules.length === 0)
    return (
      <Group label="Rules in effect">
        <Row label="No rules" hint="Rules live under “permissions” in the agent’s config file" />
      </Group>
    )
  return (
    <>
      {[...files.entries()].map(([file, list]) => (
        <Group
          key={file}
          label={
            <>
              Rules in <span className="font-mono">{tilde(file)}</span>
            </>
          }
          aside={<OpenPath path={file} small />}
        >
          {list.map((r, i) => (
            <Row
              key={i}
              label={<span className="font-mono text-xs break-all">{r.resource}</span>}
              hint={ACTIONS[r.action] ?? r.action}
              className="min-h-11 py-2"
            >
              <span
                className={cn(
                  "text-dense",
                  r.effect === "deny" ? "text-danger" : r.effect === "ask" ? "text-foreground" : "text-muted"
                )}
              >
                {EFFECT[r.effect]}
              </span>
            </Row>
          ))}
        </Group>
      ))}
    </>
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

function useSources(directory: string) {
  return useLoad(() => currentBridge().sources(directory), [directory])
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
        {(s) => <SkillList sources={s} />}
      </Loaded>
    </>
  )
}

function SkillList({ sources }: { sources: Sources }) {
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
      <Places label="Where skills are looked for" places={places} />
    </>
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
              <span className="font-mono">{tilde(p.path)}</span>. Applies to every folder, in the terminal app too.
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

function InEffect({ directory }: { directory: string }) {
  const load = useSources(directory)
  return (
    <Loaded load={load} rows={3}>
      {(s) => (
        <>
          <Group label="Read for this folder, in this order">
            {s.instructions.length === 0 && <Row label="No instruction files" hint="An AGENTS.md in the folder is read first" />}
            {s.instructions.map((f, i) => (
              <Row
                key={f.path}
                label={
                  <span className="flex min-w-0 items-baseline gap-2.5">
                    <span className="text-2xs w-3 shrink-0 text-muted tabular-nums">{i + 1}</span>
                    <span className="font-mono text-xs break-all">{tilde(f.path)}</span>
                  </span>
                }
                className="min-h-11 py-2"
              >
                <Value>{bytes(f.bytes)}</Value>
                <OpenPath path={f.path} />
              </Row>
            ))}
          </Group>
          <Places label="Looked in" places={s.places.filter((p) => p.holds === "instructions")} />
        </>
      )}
    </Loaded>
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
              note={error ? <span className="text-danger">{error}</span> : "The terminal app reads the same file."}
            >
              <ConfigToggle id="websearch" label="Web search" hint={over("websearch")} checked={c.websearch} onChange={(websearch) => change({ websearch })} />
              <ConfigToggle id="todo" label="Task list" hint={over("todo") ?? "The agent keeps a list of steps for longer work"} checked={c.todo} onChange={(todo) => change({ todo })} />
              <Row
                label="Helper agent levels"
                hint={
                  over("subagentDepth") ??
                  "The agent can hand parts of a task to helper agents. More levels let helpers start their own."
                }
              >
                <Segmented
                  label="Helper agent levels"
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
                hint={over("autoCompact") ?? "Off, a full session stops and asks you to /compact"}
                checked={c.autoCompact}
                onChange={(autoCompact) => change({ autoCompact })}
              />
              <ConfigToggle
                id="snapshots"
                label="Keep snapshots for rewinding"
                hint={over("snapshots") ?? "Off, a rewind can’t put files back"}
                checked={c.snapshots}
                onChange={(snapshots) => change({ snapshots })}
              />
              <Shell config={c} hint={over("shell")} onChange={(shell) => change({ shell })} />
            </Group>
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

/** A project's formatters that aren't allowed: they run on edits only once they are. */
function ProjectFormatters({ directory }: { directory: string }) {
  const trust = useTrust(directory)
  const items = (trust.data ?? []).filter((i) => i.kind === "formatter" && i.decision !== "allowed")
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  if (items.length === 0) return null
  return (
    <Group label="This project’s formatters" note={error ? <span className="text-danger">{error}</span> : undefined}>
      {items.map((i) => (
        <Row
          key={i.path}
          label={<span className="font-mono text-xs break-all">{shortPath(i.path, directory)}</span>}
          hint={
            i.decision === null
              ? "Waiting for your answer. Until then, edits aren’t formatted the way this file says."
              : "Not allowed. Edits aren’t formatted the way this file says."
          }
        >
          {busy === i.path && <Spinner />}
          <button
            disabled={busy !== null}
            onClick={() => {
              setBusy(i.path)
              setError(null)
              answerTrust(directory, [i], true)
                .catch((e) => setError(message(e)))
                .finally(() => setBusy(null))
            }}
            className={btn("secondary", "sm")}
          >
            Allow
          </button>
        </Row>
      ))}
    </Group>
  )
}

function ConfigToggle({
  id,
  label,
  hint,
  checked,
  onChange,
}: {
  id: string
  label: string
  hint?: ReactNode
  checked: boolean
  onChange: (on: boolean) => void
}) {
  return (
    <Row label={label} hint={hint} id={`cfg-${id}`}>
      <Toggle labelledBy={`cfg-${id}`} checked={checked} onChange={onChange} />
    </Row>
  )
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
        hint={hint ?? (custom ? undefined : config.shellDetected ? `Uses ${config.shellDetected}` : "Picks one when a command runs")}
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
