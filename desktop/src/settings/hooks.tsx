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

const EVENTS: { id: HookEvent; label: string; note: string }[] = [
  { id: "PreToolUse", label: "Before a tool runs", note: "Can stop it" },
  { id: "PostToolUse", label: "After a tool runs", note: "Can tell the agent what it found" },
  { id: "UserPromptSubmit", label: "When you send a message", note: "Can stop it, or add context" },
  { id: "Stop", label: "When the agent finishes", note: "Can keep it working" },
  { id: "SessionStart", label: "When a session starts", note: "Can add context" },
]

/** What exit code 2 does at each moment. */
const BLOCKS: Record<HookEvent, string> = {
  PreToolUse: "stops the tool",
  PostToolUse: "tells the agent",
  UserPromptSubmit: "stops the message",
  Stop: "keeps the agent working",
  SessionStart: "shows you a message",
}

export function defaultTimeout(event: HookEvent) {
  return event === "UserPromptSubmit" ? 30 : 600
}

const toolEvent = (e: HookEvent) => e === "PreToolUse" || e === "PostToolUse"

type Preset = "all" | "shell" | "files" | "startup" | "resume" | "custom"

/** A matcher as one of the choices the form offers. */
function preset(event: HookEvent, matcher: string | null): Preset {
  const m = (matcher ?? "").trim()
  if (m === "" || m === "*") return "all"
  if (event === "SessionStart") return m === "startup" ? "startup" : m === "resume" ? "resume" : "custom"
  const names = m.split(/[|,]/).map((n) => n.trim()).filter(Boolean).sort()
  if (names.length === 1 && (names[0] === "Bash" || names[0] === "shell")) return "shell"
  if (names.length === 2 && names[0] === "Edit" && names[1] === "Write") return "files"
  return "custom"
}

const PRESET_MATCHER: Record<Exclude<Preset, "custom">, string | null> = {
  all: null,
  shell: "Bash",
  files: "Edit|Write",
  startup: "startup",
  resume: "resume",
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
type Draft = HookDraft & { id: string | null }

function HookList({ directory }: { directory: string }) {
  const load = useHooks(directory)
  // `n` starts the form over when another starter is picked while it is open.
  const [adding, setAdding] = useState<(Draft & { n: number }) | null>(null)
  const [editing, setEditing] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [switching, setSwitching] = useState(false)
  const pwsh = usePowerShell(directory)
  return (
    <Loaded load={load} rows={3}>
      {(h) => {
        const mine = h.hooks.filter((x) => x.scope === "user")
        const managed = h.hooks.filter((x) => x.scope === "managed")
        const any = h.hooks.length > 0 || h.files.length > 0
        const add = (draft?: Omit<Draft, "id">) => {
          setEditing(null)
          setAdding((a) => ({ event: "PreToolUse", matcher: null, command: "", timeout: null, ...draft, id: null, n: (a?.n ?? 0) + 1 }))
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
                <ProjectFile key={f.path} file={f} directory={directory} off={h.disabled} onList={load.set} />
              ))}
            {adding && (
              <HookForm
                key={adding.n}
                directory={directory}
                draft={adding}
                pwsh={pwsh}
                onDone={(next) => {
                  setAdding(null)
                  if (next) load.set(next)
                }}
              />
            )}
            <Group
              label="Your hooks"
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
                <Empty adding={!!adding} pwsh={pwsh} onAdd={add} />
              ) : (
                mine.map((x, i) =>
                  x.id && editing === x.id ? (
                    <HookForm
                      key={x.id}
                      directory={directory}
                      draft={{ ...x, timeout: x.timeout === defaultTimeout(x.event) ? null : x.timeout, id: x.id }}
                      pwsh={pwsh}
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
              <ProjectFile key={f.path} file={f} directory={directory} off={h.disabled} onList={load.set} />
            ))}
            {managed.length > 0 && (
              <Group label="From your organization" note="Set by your organization. They always run, even with hooks off.">
                {managed.map((x, i) => (
                  <HookRow key={`${x.source}:${i}`} hook={x} directory={directory} />
                ))}
              </Group>
            )}
          </>
        )
      }}
    </Loaded>
  )
}

/** Whether hooks run in PowerShell here (Windows without a shell set), for the starters. */
function usePowerShell(directory: string) {
  const [pwsh, setPwsh] = useState(false)
  useEffect(() => {
    let live = true
    currentBridge()
      .configGet(directory)
      .then(
        (c) => live && setPwsh(/(pwsh|powershell)(\.exe)?$/i.test(c.shell ?? c.shellDetected ?? "")),
        () => {}
      )
    return () => {
      live = false
    }
  }, [directory])
  return pwsh
}

/** Three hooks people start with, written for the shell they will run in. */
function starters(pwsh: boolean): { label: string; hint: string; draft: Omit<Draft, "id"> }[] {
  // PowerShell gets its own spelling; sh, bash, zsh and Git Bash share the other.
  const failing = (run: string) =>
    pwsh
      ? `$out = ${run} 2>&1 | Out-String; if ($LASTEXITCODE) { [Console]::Error.Write($out); exit 2 }`
      : `${run} >&2 || exit 2`
  return [
    {
      label: "Block a command",
      hint: "Stop the agent from force-pushing with git",
      draft: {
        event: "PreToolUse",
        matcher: "Bash",
        timeout: null,
        command: pwsh
          ? `if ([Console]::In.ReadToEnd() -match 'git push.*(-f|--force)') { [Console]::Error.WriteLine('Force-pushing is off for the agent.'); exit 2 }`
          : `if grep -qE 'git push.*(-f|--force)'; then echo 'Force-pushing is off for the agent.' >&2; exit 2; fi`,
      },
    },
    {
      label: "Check the code after every edit",
      hint: "Run the linter after each file change and tell the agent what it found",
      draft: { event: "PostToolUse", matcher: "Edit|Write", timeout: null, command: failing("npm run lint --silent") },
    },
    {
      label: "Don’t finish until tests pass",
      hint: "Run the tests when the agent finishes, and keep it working while they fail",
      draft: { event: "Stop", matcher: null, timeout: null, command: failing("npm test --silent") },
    },
  ]
}

function Empty({ adding, pwsh, onAdd }: { adding: boolean; pwsh: boolean; onAdd: (d?: Omit<Draft, "id">) => void }) {
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
      {starters(pwsh).map((s) => (
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
  onEdit,
  onList,
}: {
  hook: Hook
  directory: string
  onEdit?: () => void
  onList?: (h: Hooks) => void
}) {
  // One that doesn't run is quieter; the switch or its file's state above says why.
  const meta = [h.timeout !== defaultTimeout(h.event) && `Time limit ${h.timeout} s`].filter(Boolean)
  const editable = !!h.id && !!onList
  return (
    <div className="flex flex-wrap items-start gap-x-4 gap-y-2 px-4 py-3">
      <div className="flex min-w-[min(100%,14rem)] flex-1 flex-col gap-1">
        <div className={cn("flex min-w-0 items-baseline gap-2", !h.active && "text-muted")}>
          <span className="truncate text-sm">{when(h.event, h.matcher)}</span>
          {meta.length > 0 && <span className="text-2xs shrink-0 text-muted">{meta.join(" · ")}</span>}
        </div>
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
}: {
  file: HookFile
  directory: string
  off: boolean
  onList: (h: Hooks) => void
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
        <HookRow key={i} hook={x} directory={directory} />
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
  pwsh,
  bare = false,
  onDone,
}: {
  directory: string
  draft: Draft
  pwsh: boolean
  /** In place of its row, inside the list: no heading or surface of its own. */
  bare?: boolean
  onDone: (h: Hooks | null) => void
}) {
  const [event, setEvent] = useState<HookEvent>(draft.event)
  const [choice, setChoice] = useState<Preset>(preset(draft.event, draft.matcher))
  const [custom, setCustom] = useState(preset(draft.event, draft.matcher) === "custom" ? (draft.matcher ?? "") : "")
  const [command, setCommand] = useState(draft.command)
  const [timeout, setTimeoutText] = useState(draft.timeout === null ? "" : String(draft.timeout))
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

  const tools = toolEvent(event)
  const matches = tools || event === "SessionStart"
  const pick = (next: HookEvent) => {
    // Tool names and session starts are different lists: a choice of one means nothing to the other.
    if (toolEvent(next) !== tools || (next === "SessionStart") !== (event === "SessionStart")) {
      setChoice("all")
      setCustom("")
    }
    setEvent(next)
  }
  const seconds = timeout.trim()
  const problems = {
    custom: matches && choice === "custom" && !custom.trim() ? (tools ? "Name the tools, like Bash or Edit|Write." : "Name the start, like startup.") : null,
    command: !command.trim() ? "Enter the command to run." : null,
    timeout: seconds && !/^[1-9]\d{0,5}$/.test(seconds) ? "A whole number of seconds." : null,
  }
  const valid = !Object.values(problems).some(Boolean)

  const submit = () => {
    setTried(true)
    if (!valid) return
    const hook: HookDraft = {
      event,
      matcher: !matches ? null : choice === "custom" ? custom.trim() : PRESET_MATCHER[choice],
      command: command.trim(),
      timeout: seconds ? Number(seconds) : null,
    }
    setSaving(true)
    setError(null)
    const b = currentBridge()
    ;(draft.id ? b.hookUpdate(directory, draft.id, hook) : b.hookAdd(directory, hook))
      .then(onDone, (e) => setError(message(e)))
      .finally(() => setSaving(false))
  }

  const options: { id: Preset; label: string }[] = tools
    ? [
        { id: "all", label: "All tools" },
        { id: "shell", label: "Shell commands" },
        { id: "files", label: "File changes" },
        { id: "custom", label: "Custom" },
      ]
    : [
        { id: "all", label: "Any" },
        { id: "startup", label: "New" },
        { id: "resume", label: "Reopened" },
      ]
  // A session-start matcher the form has no word for stays as typed.
  if (event === "SessionStart" && choice === "custom") options.push({ id: "custom", label: "Custom" })

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
                placeholder={tools ? "Bash|Edit|mcp__github__.*" : "startup"}
                aria-label={tools ? "Tool names" : "Session start"}
                spellCheck={false}
                aria-invalid={tried && !!problems.custom}
                className={cn(input, "font-mono")}
              />
              <span className={cn("text-2xs", tried && problems.custom ? "text-danger" : "text-muted")}>
                {tried && problems.custom
                  ? problems.custom
                  : tools
                    ? "Tool names separated by |, or a pattern."
                    : "startup or resume."}
              </span>
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
      <p className="text-2xs -mt-2.5 leading-relaxed text-muted">
        It gets the event as JSON on stdin. Exit code 2 {BLOCKS[event]}, and what it prints on stderr is the reason.
      </p>
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
