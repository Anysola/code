/**
 * Hooks: commands the user sets to run at fixed moments of the agent's work. Theirs can be
 * added, changed and removed here; a project's run only once its file is allowed; the
 * organization's are shown as they are.
 */
import { ChevronRight, Plus } from "lucide-react"
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react"

import type { Hook, HookDraft, HookEvent, HookFile, Hooks } from "../bridge"
import { currentBridge, setHooks, trustAnswered, useStore } from "../store"
import { btn, cn, folderKey, Spinner } from "../ui"
import { FolderSection, OpenPath } from "./agent"
import { Choice, Confirm, Group, Loaded, message, Row, Segmented, tilde, Toggle, type Load } from "./controls"
import { Field, input } from "./mcp"

const CONFIG_FILE = "~/.anysola/config.json"

// ---------------------------------------------------------------------------
// Words, shared with the conversation's card
// ---------------------------------------------------------------------------

/** The core's moments (`hooks::EVENTS`), in its order, with what a hook there can do. */
const EVENTS: { id: HookEvent; label: string; note: string }[] = [
  { id: "PreToolUse", label: "Before a tool runs", note: "Can stop it" },
  { id: "PostToolUse", label: "After a tool runs", note: "Can tell the agent what it found" },
  { id: "UserPromptSubmit", label: "When you send a message", note: "Can stop it, or add context" },
  { id: "Stop", label: "When the agent finishes", note: "Can keep it working" },
  { id: "SubagentStop", label: "When a subagent finishes", note: "Can keep it working" },
  { id: "PreCompact", label: "Before compacting", note: "Can stop a /compact you run" },
  { id: "Notification", label: "When the agent needs you", note: "Can’t stop anything" },
  { id: "SessionStart", label: "When a session starts", note: "Can add context" },
  { id: "SessionEnd", label: "When a session ends", note: "Can’t stop anything" },
]

/** Why a file allowed before 1.6.0 is asked about again. */
export const REASKED = "You allowed these before. This version also runs the ones marked new, so none of them run until you allow them again."

/** The moments 1.6.0 added (core `Event::added_in_1_6`). */
export const NEW_EVENTS = new Set<string>(["SubagentStop", "PreCompact", "Notification", "SessionEnd"])

/** Every moment a hook can run at, for labels written as the core writes them. */
export const HOOK_EVENTS = new Set<string>(EVENTS.map((e) => e.id))

/** What exit code 2 does at each moment that can stop or steer something. */
const BLOCKS: Record<Exclude<HookEvent, "Notification" | "SessionEnd">, string> = {
  PreToolUse: "stops the tool",
  PostToolUse: "tells the agent",
  UserPromptSubmit: "stops the message",
  Stop: "keeps the agent working",
  SubagentStop: "keeps the subagent working",
  PreCompact: "stops a /compact you run (an automatic one goes on)",
  SessionStart: "shows you a message",
}

/** The form's line on what a hook gets and what exit code 2 does. */
function exitLine(event: HookEvent) {
  const gets = "It gets the event as JSON on stdin."
  const notice = "Exit code 2 shows what it prints on stderr as a notice."
  if (event === "Notification") return `${gets} It can’t answer or stop anything. ${notice}`
  if (event === "SessionEnd") return `It gets the event as JSON on stdin and has 1.5 seconds, shared by every SessionEnd hook. ${notice}`
  return `${gets} Exit code 2 ${BLOCKS[event]}, and what it prints on stderr is the reason.`
}

export function defaultTimeout(event: HookEvent) {
  return event === "UserPromptSubmit" ? 30 : 600
}

/** SessionEnd's time is one budget for all of them, whatever a hook's own limit says. */
const ownTime = (e: HookEvent) => e !== "SessionEnd"

/** What a moment's matcher names: tools, how a session started, a subagent's type, what started a compaction, what the agent asks. */
type Family = "tools" | "start" | "agent" | "compact" | "ask"

function family(e: HookEvent): Family | null {
  switch (e) {
    case "PreToolUse":
    case "PostToolUse":
      return "tools"
    case "SessionStart":
      return "start"
    case "SubagentStop":
      return "agent"
    case "PreCompact":
      return "compact"
    case "Notification":
      return "ask"
    default:
      return null
  }
}

type Preset = "all" | "shell" | "files" | "startup" | "resume" | "manual" | "auto" | "permission" | "question" | "custom"

/** A matcher as one of the choices the form offers. */
function preset(event: HookEvent, matcher: string | null): Preset {
  const m = (matcher ?? "").trim()
  if (m === "" || m === "*") return "all"
  switch (family(event)) {
    case "start":
      return m === "startup" ? "startup" : m === "resume" ? "resume" : "custom"
    case "compact":
      return m === "manual" ? "manual" : m === "auto" ? "auto" : "custom"
    case "ask":
      return m === "permission_prompt" ? "permission" : m === "question" ? "question" : "custom"
    case "tools": {
      const names = m.split(/[|,]/).map((n) => n.trim()).filter(Boolean).sort()
      if (names.length === 1 && (names[0] === "Bash" || names[0] === "shell")) return "shell"
      if (names.length === 2 && names[0] === "Edit" && names[1] === "Write") return "files"
      return "custom"
    }
    default:
      return "custom"
  }
}

const PRESET_MATCHER: Record<Exclude<Preset, "custom">, string | null> = {
  all: null,
  shell: "Bash",
  files: "Edit|Write",
  startup: "startup",
  resume: "resume",
  manual: "manual",
  auto: "auto",
  permission: "permission_prompt",
  question: "question",
}

/** "Before shell commands", "When a session is reopened": when a hook runs, in a few words. */
export function when(event: HookEvent, matcher: string | null): ReactNode {
  const p = preset(event, matcher)
  const custom = <span className="font-mono text-xs">{matcher}</span>
  switch (event) {
    case "PreToolUse":
    case "PostToolUse": {
      const at = event === "PreToolUse" ? "Before" : "After"
      if (p === "all") return `${at} every tool`
      if (p === "shell") return `${at} shell commands`
      if (p === "files") return `${at} file changes`
      return (
        <>
          {at} {custom}
        </>
      )
    }
    case "UserPromptSubmit":
      return "When you send a message"
    case "Stop":
      return "When the agent finishes"
    case "SessionStart":
      if (p === "startup") return "When a new session starts"
      if (p === "resume") return "When a session is reopened"
      if (p === "all") return "When a session starts"
      return <>When a session starts ({custom})</>
    case "SubagentStop":
      if (p === "all") return "When a subagent finishes"
      return <>When a subagent finishes ({custom})</>
    case "PreCompact":
      if (p === "manual") return "Before a /compact you run"
      if (p === "auto") return "Before an automatic compaction"
      if (p === "all") return "Before compacting"
      return <>Before compacting ({custom})</>
    case "Notification":
      if (p === "permission") return "When the agent asks for approval"
      if (p === "question") return "When the agent asks a question"
      if (p === "all") return "When the agent needs you"
      return <>When the agent needs you ({custom})</>
    case "SessionEnd":
      return "When a session ends"
  }
}

/** A file of the folder by its place in it; anything else from home. */
export function shortPath(path: string, directory: string) {
  const base = folderKey(directory)
  const p = folderKey(path)
  if (p.startsWith(base) && /^[\\/]/.test(p.slice(base.length))) return path.slice(directory.replace(/[\\/]+$/, "").length + 1)
  return tilde(path)
}

const isPath = (source: string) => /[\\/]/.test(source)

// ---------------------------------------------------------------------------
// Shells
// ---------------------------------------------------------------------------

/** A shell path as people call it: "Git Bash", "PowerShell", "cmd", "zsh". */
export function shellName(path: string): string {
  const file = (path.split(/[\\/]/).pop() ?? path).toLowerCase().replace(/\.exe$/, "")
  if (file === "bash" && /[\\/]git[\\/]/i.test(path)) return "Git Bash"
  if (file === "pwsh" || file === "powershell") return "PowerShell"
  return file
}

/** A Windows path (the core's answer for where hooks run says which system this is). */
const windowsPath = (path: string) => /^[A-Za-z]:[\\/]/.test(path)

const isPowerShell = (path: string) => /(pwsh|powershell)(\.exe)?$/i.test(path)

type ShellChoice = "default" | "powershell" | "bash" | "cmd" | "custom"

/** The `shell` each choice writes ("custom": what was typed). */
const SHELL_VALUE: Record<Exclude<ShellChoice, "default" | "custom">, string> = {
  powershell: "powershell",
  bash: "bash",
  cmd: "cmd",
}

function shellChoice(shell: string | null | undefined, windows: boolean): ShellChoice {
  const s = (shell ?? "").trim()
  if (!s) return "default"
  if (windows) {
    const lower = s.toLowerCase()
    if (lower === "powershell" || lower === "pwsh") return "powershell"
    if (lower === "bash") return "bash"
    if (lower === "cmd") return "cmd"
  }
  return "custom"
}

// ---------------------------------------------------------------------------
// The folder's hooks, read once and shared
// ---------------------------------------------------------------------------

/** The folder's hooks, read again when the core says they changed; every view shows the same. */
export function useHooks(directory: string): Load<Hooks> {
  const data = useStore((s) => s.hooks[folderKey(directory)])
  const changed = useStore((s) => s.hooksChanged)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const n = useRef(0)
  const reload = useCallback(() => {
    const at = ++n.current
    setLoading(true)
    setError(null)
    currentBridge()
      .hooks(directory)
      .then(
        (h) => {
          if (at !== n.current) return
          setHooks(directory, h)
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
  const set = useCallback((h: Hooks) => setHooks(directory, h), [directory])
  return { data, error, loading, reload, set }
}

// ---------------------------------------------------------------------------
// The section
// ---------------------------------------------------------------------------

export function HooksSection() {
  return <FolderSection title="Hooks">{(directory) => <HookList directory={directory} />}</FolderSection>
}

/** The form's starting point: a new hook (blank or a starter's), or one being changed. */
type Draft = HookDraft & { id: string | null; starter?: StarterId }

function HookList({ directory }: { directory: string }) {
  const load = useHooks(directory)
  // `n` starts the form over when another starter is picked while it is open.
  const [adding, setAdding] = useState<(Draft & { n: number }) | null>(null)
  const [editing, setEditing] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [switching, setSwitching] = useState(false)
  return (
    <Loaded load={load} rows={3}>
      {(h) => {
        // Where hooks without a shell of their own run, as the core answers it.
        const shells = { path: h.shell ?? "", windows: windowsPath(h.shell ?? "") }
        const mine = h.hooks.filter((x) => x.scope === "user")
        const managed = h.hooks.filter((x) => x.scope === "managed")
        const any = h.hooks.length > 0 || h.files.length > 0
        const add = (draft?: Omit<Draft, "id">) => {
          setEditing(null)
          setAdding((a) => ({ event: "PreToolUse", matcher: null, command: "", timeout: null, shell: null, ...draft, id: null, n: (a?.n ?? 0) + 1 }))
        }
        return (
          <>
            {(any || h.disabled) && (
              <Group note={error ? <span className="text-danger">{error}</span> : undefined}>
                <Row
                  label="Run hooks"
                  id="hooks-on"
                  hint={
                    h.disabled
                      ? managed.length > 0
                        ? "Off: yours and the project’s don’t run. Your organization’s still do."
                        : "Off: none of them run"
                      : undefined
                  }
                >
                  <Toggle
                    labelledBy="hooks-on"
                    checked={!h.disabled}
                    disabled={switching}
                    onChange={(on) => {
                      setSwitching(true)
                      setError(null)
                      currentBridge()
                        .hooksDisable(directory, !on)
                        .then((next) => {
                          // The answer is what holds: another file or a variable can keep them off.
                          load.set(next)
                          if (on && next.disabled)
                            setError("Hooks stay off: another config file or an environment variable turns them off.")
                        }, (e) => setError(message(e)))
                        .finally(() => setSwitching(false))
                    }}
                  />
                </Row>
              </Group>
            )}
            {/* A file waiting for an answer comes first: nothing of it runs until then. */}
            {h.files
              .filter((f) => f.decision === null)
              .map((f) => (
                <ProjectFile key={f.path} file={f} directory={directory} off={h.disabled} onList={load.set} defaultShell={shells.path} />
              ))}
            {adding && (
              <HookForm
                key={adding.n}
                directory={directory}
                draft={adding}
                shells={shells}
                onDone={(next) => {
                  setAdding(null)
                  if (next) load.set(next)
                }}
              />
            )}
            <Group
              label={
                <>
                  Your hooks
                  {shells.path && <span className="ml-2 font-normal">run in {shellName(shells.path)}</span>}
                </>
              }
              aside={
                // The empty state has its own button.
                !adding &&
                mine.length > 0 && (
                  <button onClick={() => add()} className={btn("ghost", "sm", "h-6 gap-1 px-2 text-2xs text-muted")}>
                    <Plus className="size-3" />
                    Add hook
                  </button>
                )
              }
              note={
                mine.length > 0 ? (
                  <>
                    Kept in <span className="font-mono">{CONFIG_FILE}</span>. A project’s own run only once you allow them.
                  </>
                ) : undefined
              }
            >
              {mine.length === 0 ? (
                <Empty adding={!!adding} shells={shells} onAdd={add} />
              ) : (
                mine.map((x, i) =>
                  x.id && editing === x.id ? (
                    <HookForm
                      key={x.id}
                      directory={directory}
                      draft={{ ...x, shell: x.shell ?? null, timeout: x.timeout === defaultTimeout(x.event) ? null : x.timeout, id: x.id }}
                      shells={shells}
                      bare
                      onDone={(next) => {
                        setEditing(null)
                        if (next) load.set(next)
                      }}
                    />
                  ) : (
                    <HookRow
                      key={x.id ?? `${x.source}:${i}`}
                      hook={x}
                      directory={directory}
                      defaultShell={shells.path}
                      onEdit={
                        x.id
                          ? () => {
                              setAdding(null)
                              setEditing(x.id)
                            }
                          : undefined
                      }
                      onList={load.set}
                    />
                  )
                )
              )}
            </Group>
            {h.files
              .filter((f) => f.decision !== null)
              .map((f) => (
              <ProjectFile key={f.path} file={f} directory={directory} off={h.disabled} onList={load.set} defaultShell={shells.path} />
            ))}
            {managed.length > 0 && (
              <Group label="From your organization" note="Set by your organization. They always run, even with hooks off.">
                {managed.map((x, i) => (
                  <HookRow key={`${x.source}:${i}`} hook={x} directory={directory} defaultShell={shells.path} />
                ))}
              </Group>
            )}
          </>
        )
      }}
    </Loaded>
  )
}

/** The custom matcher field, by what the moment's matcher names. */
const CUSTOM: Record<Family, { placeholder: string; label: string; hint: string; missing: string }> = {
  tools: { placeholder: "Bash|Edit|mcp__github__.*", label: "Tool names", hint: "Tool names separated by |, or a pattern.", missing: "Name the tools, like Bash or Edit|Write." },
  start: { placeholder: "startup", label: "Session start", hint: "startup or resume.", missing: "Name the start, like startup." },
  agent: { placeholder: "explore|general", label: "Subagent types", hint: "Subagent types separated by |, like general or explore.", missing: "Name the subagent type, like explore." },
  compact: { placeholder: "manual", label: "Compaction", hint: "manual or auto.", missing: "Enter manual or auto." },
  ask: { placeholder: "permission_prompt", label: "Request", hint: "permission_prompt or question.", missing: "Enter permission_prompt or question." },
}

type Shells = { path: string; windows: boolean }

type StarterId = "block" | "lint" | "test"

/** A starter's command in PowerShell's spelling or the one sh, bash, zsh and Git Bash share. */
function starterCommand(id: StarterId, pwsh: boolean): string {
  const failing = (run: string) =>
    pwsh
      ? `$out = ${run} 2>&1 | Out-String; if ($LASTEXITCODE) { [Console]::Error.Write($out); exit 2 }`
      : `${run} >&2 || exit 2`
  switch (id) {
    case "block":
      return pwsh
        ? `if ([Console]::In.ReadToEnd() -match 'git push.*(-f|--force)') { [Console]::Error.WriteLine('Force-pushing is off for the agent.'); exit 2 }`
        : `if grep -qE 'git push.*(-f|--force)'; then echo 'Force-pushing is off for the agent.' >&2; exit 2; fi`
    case "lint":
      return failing("npm run lint --silent")
    case "test":
      return failing("npm test --silent")
  }
}

/**
 * Whether a starter written for this choice is PowerShell's spelling; null where neither fits
 * (cmd, a shell typed in), so the command is left as it is.
 */
function starterPwsh(choice: ShellChoice, shells: Shells): boolean | null {
  if (choice === "powershell") return true
  if (choice === "bash") return false
  if (choice === "default") return isPowerShell(shells.path)
  return null
}

/**
 * Three hooks people start with. On Windows each names the shell it is written for, so it keeps
 * running there whatever the default becomes; elsewhere every default shell reads the same.
 */
function starters(shells: Shells): { label: string; hint: string; draft: Omit<Draft, "id"> }[] {
  const pwsh = isPowerShell(shells.path)
  const shell = shells.windows ? (pwsh ? "powershell" : "bash") : null
  const draft = (starter: StarterId, event: HookEvent, matcher: string | null) => ({
    starter,
    event,
    matcher,
    timeout: null,
    shell,
    command: starterCommand(starter, pwsh),
  })
  return [
    { label: "Block a command", hint: "Stop the agent from force-pushing with git", draft: draft("block", "PreToolUse", "Bash") },
    {
      label: "Check the code after every edit",
      hint: "Run the linter after each file change and tell the agent what it found",
      draft: draft("lint", "PostToolUse", "Edit|Write"),
    },
    {
      label: "Don’t finish until tests pass",
      hint: "Run the tests when the agent finishes, and keep it working while they fail",
      draft: draft("test", "Stop", null),
    },
  ]
}

function Empty({ adding, shells, onAdd }: { adding: boolean; shells: Shells; onAdd: (d?: Omit<Draft, "id">) => void }) {
  return (
    <>
      <div className="flex min-h-12 items-center gap-4 px-4 py-3">
        <p className="text-dense min-w-0 flex-1 text-muted">
          A hook runs a command of yours at a set moment, such as before every shell command, and can stop the agent or
          tell it what went wrong.
        </p>
        {!adding && (
          <button onClick={() => onAdd()} className={btn("secondary", "sm", "shrink-0")}>
            Add hook
          </button>
        )}
      </div>
      {starters(shells).map((s) => (
        <button
          type="button"
          key={s.label}
          onClick={() => onAdd(s.draft)}
          className="group flex min-h-12 w-full items-center gap-4 px-4 py-2.5 text-left transition-colors last:rounded-b-xl hover:bg-default/40 focus-visible:status-focused-field"
        >
          <span className="flex min-w-0 flex-1 flex-col gap-0.5">
            <span className="text-sm">{s.label}</span>
            <span className="text-dense text-muted">{s.hint}</span>
          </span>
          <ChevronRight className="size-3.5 shrink-0 text-muted group-hover:text-foreground" />
        </button>
      ))}
    </>
  )
}

function HookRow({
  hook: h,
  directory,
  defaultShell,
  onEdit,
  onList,
  fresh = false,
}: {
  hook: Hook
  /** At a moment this version added, in a file asked about again for it: marked new. */
  fresh?: boolean
  directory: string
  /** Where hooks without a shell of their own run (the group's heading says it). */
  defaultShell: string
  onEdit?: () => void
  onList?: (h: Hooks) => void
}) {
  // Where it runs, when that isn't where the heading says: its own shell (or the one it falls
  // back to when that one isn't on this computer).
  const runsIn = h.runsIn ? shellName(h.runsIn) : null
  const own = h.shell ? shellName(h.shell) : null
  const missing = own !== null && runsIn !== null && own.toLowerCase() !== runsIn.toLowerCase() && !(own === "bash" && runsIn === "Git Bash")
  const elsewhere = runsIn !== null && (own !== null || runsIn !== shellName(defaultShell))
  // One that doesn't run is quieter; the switch or its file's state above says why.
  const meta = [
    fresh && "New",
    elsewhere && !missing && `In ${runsIn}`,
    ownTime(h.event) && h.timeout !== defaultTimeout(h.event) && `Time limit ${h.timeout} s`,
  ].filter(Boolean)
  const editable = !!h.id && !!onList
  return (
    <div className="flex flex-wrap items-start gap-x-4 gap-y-2 px-4 py-3">
      <div className="flex min-w-[min(100%,14rem)] flex-1 flex-col gap-1">
        <div className={cn("flex min-w-0 items-baseline gap-2", !h.active && "text-muted")}>
          <span className="truncate text-sm">{when(h.event, h.matcher)}</span>
          {meta.length > 0 && <span className="text-2xs shrink-0 text-muted">{meta.join(" · ")}</span>}
        </div>
        {missing && (
          <span className="text-2xs text-warning">
            Hooks can’t run in {h.shell} here, so it runs in {runsIn}
          </span>
        )}
        <span className="truncate font-mono text-xs text-muted" title={h.command}>
          {h.command}
        </span>
        {h.scope === "user" && !h.id && (
          <span className="text-2xs truncate text-muted" title={h.source}>
            Defined in <span className="font-mono">{isPath(h.source) ? shortPath(h.source, directory) : h.source}</span>
          </span>
        )}
      </div>
      {(editable || (h.scope === "user" && isPath(h.source))) && (
        <div className="ml-auto flex max-w-full items-center gap-1">
          {editable ? (
            <>
              <button onClick={onEdit} className={btn("ghost", "sm", "text-muted")}>
                Edit
              </button>
              <Confirm
                label="Remove"
                question="Remove this hook?"
                action="Remove"
                run={async () => {
                  try {
                    onList!(await currentBridge().hookRemove(directory, h.id!))
                  } catch (e) {
                    return message(e)
                  }
                }}
              />
            </>
          ) : (
            <OpenPath path={h.source} />
          )}
        </div>
      )}
    </div>
  )
}

const DECISION = {
  waiting: { word: "Waiting for you", mark: "bg-warning" },
  allowed: { word: "Allowed", mark: "bg-success" },
  denied: { word: "Not allowed", mark: "bg-muted/60" },
}

function ProjectFile({
  file: f,
  directory,
  off,
  onList,
  defaultShell,
}: {
  file: HookFile
  directory: string
  off: boolean
  onList: (h: Hooks) => void
  defaultShell: string
}) {
  const [busy, setBusy] = useState<boolean | null>(null)
  const [error, setError] = useState<string | null>(null)
  const state = DECISION[f.decision ?? "waiting"]
  // Its hooks allowed earlier were removed: they keep running until the user accepts that.
  const emptied = f.hooks.length === 0
  const answer = (allow: boolean) => {
    setBusy(allow)
    setError(null)
    currentBridge()
      .hooksTrust(directory, f.path, f.digest, allow)
      .then((h) => {
        onList(h)
        // The conversation's card asks about the same file.
        trustAnswered(["hooks"])
      }, (e) => setError(message(e)))
      .finally(() => setBusy(null))
  }
  return (
    <Group
      label={
        <>
          This project <span className="ml-2 font-mono font-normal">{shortPath(f.path, directory)}</span>
        </>
      }
      aside={<OpenPath path={f.path} small />}
    >
      <Row
        label={
          <span className="flex items-center gap-2">
            <span className={cn("size-1.5 shrink-0 rounded-full", state.mark)} aria-hidden />
            {state.word}
          </span>
        }
        hint={
          emptied
            ? f.decision === null
              ? "This file no longer defines hooks. The ones you allowed before keep running until you stop them."
              : f.standing
                ? "This file no longer defines hooks. The ones you allowed before keep running until the app restarts."
                : "This file defines no hooks."
            : f.decision === null
              ? f.standing
                ? "These changed. The ones you allowed before keep running until you allow the change."
                : f.reasked
                  ? REASKED
                  : "These run commands on this computer. Allow them only if you trust this project."
              : f.decision === "allowed"
                ? off
                  ? "They run once hooks are back on."
                  : "They run while you work in this folder."
                : f.standing
                  ? "This change isn’t allowed. The hooks you allowed before it keep running until the app restarts."
                  : "They don’t run. Allow them if you trust this project."
        }
      >
        {busy !== null && <Spinner />}
        {f.decision !== "allowed" && !(emptied && f.decision !== null && !f.standing) && (
          <button
            disabled={busy !== null}
            onClick={() => answer(true)}
            className={btn(f.decision === null ? "primary" : "secondary", "sm")}
          >
            {emptied ? "Stop running them" : "Allow"}
          </button>
        )}
        {emptied && f.decision === null && (
          <button disabled={busy !== null} onClick={() => answer(false)} className={btn("ghost", "sm", "text-muted")}>
            Keep them running
          </button>
        )}
        {!emptied && f.decision !== "denied" && (
          <button disabled={busy !== null} onClick={() => answer(false)} className={btn("ghost", "sm", "text-muted")}>
            {f.decision === null ? "Don’t allow" : "Stop allowing"}
          </button>
        )}
      </Row>
      {f.hooks.map((x, i) => (
        <HookRow key={i} hook={x} directory={directory} defaultShell={defaultShell} fresh={f.decision === null && !!f.reasked && NEW_EVENTS.has(x.event)} />
      ))}
      {error && <p className="text-dense px-4 py-2.5 text-danger select-text">{error}</p>}
    </Group>
  )
}

// ---------------------------------------------------------------------------
// Adding or changing one
// ---------------------------------------------------------------------------

function HookForm({
  directory,
  draft,
  shells,
  bare = false,
  onDone,
}: {
  directory: string
  draft: Draft
  shells: Shells
  /** In place of its row, inside the list: no heading or surface of its own. */
  bare?: boolean
  onDone: (h: Hooks | null) => void
}) {
  const [event, setEvent] = useState<HookEvent>(draft.event)
  const [choice, setChoice] = useState<Preset>(preset(draft.event, draft.matcher))
  const [custom, setCustom] = useState(preset(draft.event, draft.matcher) === "custom" ? (draft.matcher ?? "") : "")
  const [command, setCommand] = useState(draft.command)
  const [timeout, setTimeoutText] = useState(draft.timeout === null ? "" : String(draft.timeout))
  const [shell, setShell] = useState<ShellChoice>(shellChoice(draft.shell, shells.windows))
  const [customShell, setCustomShell] = useState(shellChoice(draft.shell, shells.windows) === "custom" ? (draft.shell ?? "") : "")
  const pwsh = starterPwsh(shell, shells) ?? isPowerShell(shells.path)
  const pickShell = (next: ShellChoice) => {
    // A starter not changed yet is written again for the shell picked.
    if (draft.starter) {
      const was = starterPwsh(shell, shells)
      const now = starterPwsh(next, shells)
      if (was !== null && now !== null && command === starterCommand(draft.starter, was)) setCommand(starterCommand(draft.starter, now))
    }
    setShell(next)
  }
  const [tried, setTried] = useState(false)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const form = useRef<HTMLFormElement>(null)
  const commandBox = useRef<HTMLTextAreaElement>(null)
  useEffect(() => {
    form.current?.scrollIntoView({ block: "nearest" })
    const box = commandBox.current
    if (!box) return
    box.focus()
    box.setSelectionRange(box.value.length, box.value.length)
  }, [])

  const kind = family(event)
  const tools = kind === "tools"
  const matches = kind !== null
  const pick = (next: HookEvent) => {
    // Each moment's matcher names something else: a choice for one means nothing to another.
    if (family(next) !== kind) {
      setChoice("all")
      setCustom("")
    }
    setEvent(next)
  }
  const seconds = timeout.trim()
  const problems = {
    custom: matches && choice === "custom" && !custom.trim() ? CUSTOM[kind].missing : null,
    command: !command.trim() ? "Enter the command to run." : null,
    timeout: seconds && !/^[1-9]\d{0,5}$/.test(seconds) ? "A whole number of seconds." : null,
    shell: shell === "custom" && !customShell.trim() ? "Enter a shell’s name or path." : null,
  }
  const valid = !Object.values(problems).some(Boolean)

  const submit = () => {
    setTried(true)
    if (!valid) return
    const hook: HookDraft = {
      event,
      matcher: !matches ? null : choice === "custom" ? custom.trim() : PRESET_MATCHER[choice],
      command: command.trim(),
      timeout: seconds && ownTime(event) ? Number(seconds) : null,
      shell: shell === "default" ? null : shell === "custom" ? customShell.trim() : SHELL_VALUE[shell],
    }
    setSaving(true)
    setError(null)
    const b = currentBridge()
    ;(draft.id ? b.hookUpdate(directory, draft.id, hook) : b.hookAdd(directory, hook))
      .then(onDone, (e) => setError(message(e)))
      .finally(() => setSaving(false))
  }

  const options: { id: Preset; label: string }[] =
    kind === "tools"
      ? [
          { id: "all", label: "All tools" },
          { id: "shell", label: "Shell commands" },
          { id: "files", label: "File changes" },
          { id: "custom", label: "Custom" },
        ]
      : kind === "agent"
        ? [
            { id: "all", label: "Any" },
            { id: "custom", label: "By type" },
          ]
        : kind === "compact"
          ? [
              { id: "all", label: "Any" },
              { id: "manual", label: "/compact" },
              { id: "auto", label: "Automatic" },
            ]
          : kind === "ask"
            ? [
                { id: "all", label: "Any" },
                { id: "permission", label: "Approvals" },
                { id: "question", label: "Questions" },
              ]
            : [
                { id: "all", label: "Any" },
                { id: "startup", label: "New" },
                { id: "resume", label: "Reopened" },
              ]
  // A matcher the form has no word for stays as typed.
  if (choice === "custom" && !options.some((o) => o.id === "custom")) options.push({ id: "custom", label: "Custom" })

  // Windows' shells by name; elsewhere the default (every one reads POSIX) or one typed in.
  const shellOptions: { id: ShellChoice; label: string; title?: string }[] = [
    { id: "default", label: "Default", title: shells.path ? `Where hooks run: ${shellName(shells.path)}` : undefined },
    ...(shells.windows
      ? ([
          { id: "powershell", label: "PowerShell" },
          { id: "bash", label: "Git Bash" },
          { id: "cmd", label: "cmd" },
        ] as const)
      : []),
    { id: "custom", label: "Other" },
  ]

  const body = (
    <div className={cn("flex flex-col gap-4", bare ? "px-4 py-4" : "rounded-xl bg-surface p-4")}>
      <div className="grid grid-cols-[auto_minmax(0,1fr)] items-center gap-x-4 gap-y-3">
        <span className="text-2xs text-muted">When</span>
        <div className="flex">
          <Choice label="When it runs" value={event} onChange={pick} options={EVENTS} align="start" />
        </div>
        {matches && (
          <>
            <span className="text-2xs text-muted">Applies to</span>
            <div className="flex min-w-0 flex-wrap items-center gap-2">
              <Segmented label="Applies to" value={choice} onChange={setChoice} options={options} />
            </div>
          </>
        )}
        {matches && choice === "custom" && (
          <>
            <span />
            <span className="flex flex-col gap-1.5">
              <input
                value={custom}
                onChange={(e) => setCustom(e.target.value)}
                placeholder={CUSTOM[kind!].placeholder}
                aria-label={CUSTOM[kind!].label}
                spellCheck={false}
                aria-invalid={tried && !!problems.custom}
                className={cn(input, "font-mono")}
              />
              <span className={cn("text-2xs", tried && problems.custom ? "text-danger" : "text-muted")}>
                {tried && problems.custom ? problems.custom : CUSTOM[kind!].hint}
              </span>
            </span>
          </>
        )}
        <span className="text-2xs text-muted">Runs in</span>
        <div className="flex min-w-0 flex-wrap items-center gap-2">
          <Segmented label="Runs in" value={shell} onChange={pickShell} options={shellOptions} />
          {shell === "default" && shells.path && <span className="text-2xs text-muted">{shellName(shells.path)}</span>}
        </div>
        {shell === "custom" && (
          <>
            <span />
            <span className="flex flex-col gap-1.5">
              <input
                value={customShell}
                onChange={(e) => setCustomShell(e.target.value)}
                placeholder={shells.windows ? "C:\\Program Files\\PowerShell\\7\\pwsh.exe" : "/bin/zsh"}
                aria-label="Shell"
                spellCheck={false}
                aria-invalid={tried && !!problems.shell}
                className={cn(input, "font-mono")}
              />
              {tried && problems.shell && <span className="text-2xs text-danger">{problems.shell}</span>}
            </span>
          </>
        )}
      </div>
      <Field label="Command" error={tried ? problems.command : null}>
        <textarea
          ref={commandBox}
          value={command}
          onChange={(e) => setCommand(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault()
              submit()
            }
          }}
          rows={2}
          placeholder={pwsh ? "npm run lint" : "./scripts/check.sh"}
          spellCheck={false}
          aria-invalid={tried && !!problems.command}
          className={cn(input, "field-sizing-content h-auto max-h-40 min-h-16 resize-none py-2 font-mono leading-relaxed")}
        />
      </Field>
      <p className="text-2xs -mt-2.5 leading-relaxed text-muted">{exitLine(event)}</p>
      {ownTime(event) && (
      <label className="flex items-center gap-3">
        <span className="text-2xs text-muted">Time limit</span>
        <input
          value={timeout}
          onChange={(e) => setTimeoutText(e.target.value)}
          inputMode="numeric"
          placeholder={String(defaultTimeout(event))}
          aria-invalid={tried && !!problems.timeout}
          className={cn(input, "w-20 tabular-nums")}
        />
        <span className={cn("text-2xs", tried && problems.timeout ? "text-danger" : "text-muted")}>
          {tried && problems.timeout ? problems.timeout : "seconds"}
        </span>
      </label>
      )}
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
          {draft.id ? "Save" : "Add"}
        </button>
      </div>
    </div>
  )

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
      aria-label={draft.id ? "Change a hook" : "Add a hook"}
      className="flex scroll-my-6 flex-col gap-2"
    >
      {!bare && (
        <div className="flex min-h-6 items-end px-1">
          <h2 className="text-2xs font-medium tracking-wide text-muted">Add a hook</h2>
        </div>
      )}
      {body}
    </form>
  )
}
