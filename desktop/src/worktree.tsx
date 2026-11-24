/**
 * A session in its own git worktree of the main folder: the new session's place choice, the
 * worktree being made (steps and the setup command's output), and the branch as the header,
 * the sidebar and the footer show it.
 */
import { Check, ChevronDown, ChevronRight, CircleAlert, FolderClosed, GitBranch, X } from "lucide-react"
import { useCallback, useEffect, useRef, useState } from "react"

import type { SessionWorktree, SetupRun } from "./bridge"
import { duration, Elapsed } from "./conversation"
import {
  hereOf,
  openSettings,
  setPlace,
  setupRun,
  setupStop,
  useStore,
  useWorktree,
  useWorktreeInfo,
  worktreeToFolder,
  type Open,
} from "./store"
import { btn, cn, folderName, Popover, Spinner, Tooltip } from "./ui"

// ---------------------------------------------------------------------------
// The branch
// ---------------------------------------------------------------------------

/**
 * `⎇ anysola/x`: muted and small; struck once discarded. A long branch is cut short, the whole
 * name (and the folder) on hover; `hint` false where a tooltip already covers it.
 */
export function BranchName({ worktree, className, hint = true }: { worktree: SessionWorktree; className?: string; hint?: boolean }) {
  const gone = worktree.state === "discarded"
  return (
    <span
      className={cn("flex min-w-0 items-center gap-1 overflow-hidden", className)}
      title={hint ? (gone ? `${worktree.branch} (discarded)` : `${worktree.branch}\n${worktree.path}`) : undefined}
    >
      <GitBranch className="size-3 shrink-0" strokeWidth={1.75} />
      <span className={cn("min-w-0 truncate font-mono", gone && "line-through decoration-muted/70")}>{worktree.branch}</span>
    </span>
  )
}

/** The worktree's trouble in one word for a dot's label; null when there is none. */
export function worktreeTrouble(worktree: SessionWorktree | null): string | null {
  if (worktree?.state === "missing") return "Its folder is gone"
  if (worktree?.state === "failed") return "Its worktree couldn’t be made"
  return null
}

/** Under the composer: the main folder's name, and the session's branch when it has a worktree. */
export function FooterPlace({ open, label }: { open: Open; label: string }) {
  const worktree = useWorktree(open.sessionId)
  return (
    <span className="text-2xs flex min-w-0 items-center gap-1.5 text-muted" title={worktree?.path ?? open.directory}>
      <FolderClosed className="size-3 shrink-0" strokeWidth={1.75} />
      <span className="truncate">{worktree ? folderName(open.directory) : label}</span>
      {worktree && <BranchName worktree={worktree} className="ml-1" />}
    </span>
  )
}

// ---------------------------------------------------------------------------
// New session: where it works
// ---------------------------------------------------------------------------

/** Next to the model picker, only for a new session in a git folder with a commit. Not kept. */
export function PlaceChip({ open }: { open: Open }) {
  const info = useWorktreeInfo(open.directory)
  const [show, setShow] = useState(false)
  const close = useCallback(() => setShow(false), [])
  if (open.sessionId || !info?.available) return null
  const folder = open.place === "folder"
  const dirty = info.uncommitted
  const from = info.branch ?? "the current commit"
  const pick = (place: "worktree" | "folder") => {
    setPlace(place)
    close()
  }
  return (
    <div className="relative min-w-0 shrink-0">
      <button
        onClick={() => setShow(!show)}
        aria-expanded={show}
        aria-label={folder ? "Works in the main folder" : "Works in its own worktree"}
        className="text-dense flex h-8 items-center gap-1.5 rounded-full px-3 text-muted hover:bg-default/60 hover:text-foreground focus-visible:status-focused-field"
      >
        {folder ? <FolderClosed className="size-3.5 shrink-0" strokeWidth={1.75} /> : <GitBranch className="size-3.5 shrink-0" strokeWidth={1.75} />}
        <span>{folder ? "Main folder" : "Worktree"}</span>
        {!folder && dirty > 0 && <span className="size-1.5 shrink-0 rounded-full bg-warning/80" aria-hidden />}
        <ChevronDown className="size-3.5 shrink-0" />
      </button>
      <Popover open={show} onClose={close} side="top" align="end" label="Where this session works" className="w-80">
        <div role="radiogroup" aria-label="Where this session works" className="flex flex-col">
          <PlaceOption
            on={!folder}
            onPick={() => pick("worktree")}
            title="Its own worktree"
            detail={`A new branch from ${from}. Your folder stays as it is until you apply.`}
          />
          <PlaceOption
            on={folder}
            onPick={() => pick("folder")}
            title="Work in the main folder"
            detail="Edits land in your folder as they’re made."
          />
        </div>
        {dirty > 0 && (
          <p className="text-2xs mx-2.5 mt-1 mb-1.5 border-t border-separator pt-2.5 text-muted">
            {dirty} uncommitted change{dirty === 1 ? "" : "s"} in the main folder won’t be in the worktree.
          </p>
        )}
      </Popover>
    </div>
  )
}

function PlaceOption({ on, onPick, title, detail }: { on: boolean; onPick: () => void; title: string; detail: string }) {
  return (
    <button
      role="radio"
      aria-checked={on}
      onClick={onPick}
      className="flex items-start gap-2.5 rounded-lg px-2.5 py-2 text-left hover:bg-default-hover focus-visible:status-focused-field"
    >
      <span className="flex size-4 shrink-0 items-center justify-center pt-0.5">{on && <Check className="size-3.5" />}</span>
      <span className="flex min-w-0 flex-col gap-0.5">
        <span className="text-dense">{title}</span>
        <span className="text-2xs text-muted">{detail}</span>
      </span>
    </button>
  )
}

// ---------------------------------------------------------------------------
// Being made, and the setup command
// ---------------------------------------------------------------------------

/** Lines the app wrote (the steps), and the command's own output. */
function split(run: SetupRun | null) {
  const steps = run?.lines.filter((l) => l.stream === "info").map((l) => l.text) ?? []
  const output = run?.lines.filter((l) => l.stream !== "info") ?? []
  return { steps, output }
}

/**
 * At the end of the conversation: the worktree being made (steps, elapsed, the setup output),
 * with Start now and Work in the main folder instead; once ready, only a setup run that failed
 * (one line, not a blocker) or one running again.
 */
export function WorktreeProgress({ open }: { open: Open }) {
  const worktree = useWorktree(open.sessionId)
  const run = useStore((s) => (open.sessionId ? (s.setups[open.sessionId] ?? null) : null))
  const [shown, setShown] = useState(false)
  const [hidden, setHidden] = useState<number | null>(null)
  const [busy, setBusy] = useState<null | "start" | "folder" | "again">(null)
  useEffect(() => {
    setShown(false)
    setHidden(null)
  }, [open.sessionId])
  const here = hereOf(open)
  if (!worktree || !here) return null
  // Gone or discarded: its setup no longer matters.
  if (worktree.state !== "ready" && worktree.state !== "preparing" && worktree.state !== "applying") return null
  const preparing = worktree.state === "preparing"
  const running = run?.state === "running"
  const failed = run && (run.state === "failed" || run.state === "timeout")
  if (!preparing && !running && !failed) return null
  if (!preparing && !running && hidden === run?.started) return null
  const { steps, output } = split(run)
  const act = (what: NonNullable<typeof busy>, f: () => Promise<void>) => () => {
    setBusy(what)
    void f().finally(() => setBusy(null))
  }
  const outputToggle = output.length > 0 && (
    <button
      onClick={() => setShown(!shown)}
      aria-expanded={shown}
      className="text-2xs flex items-center gap-1 rounded text-muted hover:text-foreground focus-visible:status-focused-field"
    >
      {shown ? <ChevronDown className="size-3" /> : <ChevronRight className="size-3" />}
      {shown ? "Hide output" : "Show output"}
    </button>
  )
  const panel = shown && output.length > 0 && <Output run={run!} />
  const editSetup = (
    <button
      onClick={() => openSettings("advanced")}
      className="text-2xs rounded text-muted underline-offset-2 hover:text-foreground hover:underline focus-visible:status-focused-field"
    >
      Edit setup command
    </button>
  )

  if (!preparing && failed)
    return (
      <div className="mt-6 flex flex-col gap-2" role="status">
        <div className="text-dense flex min-h-9 items-center gap-2.5 rounded-xl bg-surface/70 py-1 pr-1.5 pl-3.5">
          <CircleAlert className="size-3.5 shrink-0 text-warning" />
          <span className="min-w-0 flex-1 py-1">
            {run!.state === "timeout" ? "Setup stopped after 20 minutes." : `Setup failed${run!.code !== null ? ` (exit ${run!.code})` : ""}.`}{" "}
            <span className="text-muted">The session goes on.</span>
          </span>
          <span className="flex shrink-0 items-center gap-1">
            <button disabled={busy !== null} onClick={act("again", () => setupRun(here))} className={btn("ghost", "sm", "gap-1.5 text-muted")}>
              {busy === "again" && <Spinner className="size-3" />}
              Run again
            </button>
            <Tooltip label="Dismiss" side="top">
              <button onClick={() => setHidden(run!.started)} aria-label="Dismiss" className={btn("ghost", "sm", "text-muted", true)}>
                <X className="size-3.5" />
              </button>
            </Tooltip>
          </span>
        </div>
        <div className="flex items-center gap-4 pl-[2.375rem]">
          {outputToggle}
          {editSetup}
        </div>
        {panel}
      </div>
    )

  // Preparing, or the setup command running again in a ready worktree.
  const current = steps.length ? steps[steps.length - 1] : "Starting"
  // Once the setup command runs (its name known, or its output arriving), stopping it starts the session.
  const canStart = preparing && running && (!!run?.command || output.length > 0)
  return (
    <div className="mt-6 rounded-xl bg-surface/70 px-3.5 pt-3 pb-3" role="status" aria-live="polite">
      <div className="flex items-center gap-2.5">
        <Spinner className="size-3.5 shrink-0" />
        <span className="text-dense min-w-0 flex-1">
          {preparing ? "Preparing worktree" : "Running setup"}
          {run && (
            <span className="text-muted">
              {" · "}
              <Elapsed since={run.started} />
            </span>
          )}
        </span>
        <span className="flex shrink-0 items-center gap-1">
          {preparing && (
            <button disabled={busy !== null} onClick={act("folder", () => worktreeToFolder(here))} className={btn("ghost", "sm", "gap-1.5 text-muted")}>
              {busy === "folder" && <Spinner className="size-3" />}
              Work in the main folder instead
            </button>
          )}
          {(canStart || (!preparing && running)) && (
            <Tooltip label={preparing ? "Stop the setup command and start the session" : "Stop the setup command"} side="top">
              <button disabled={busy !== null} onClick={act("start", () => setupStop(here))} className={btn("secondary", "sm", "gap-1.5")}>
                {busy === "start" && <Spinner className="size-3" />}
                {preparing ? "Start now" : "Stop"}
              </button>
            </Tooltip>
          )}
        </span>
      </div>
      <ol className="mt-2 flex flex-col gap-1 pl-6">
        {steps.map((s, i) => {
          const last = i === steps.length - 1
          return (
            <li key={i} className={cn("text-2xs flex min-w-0 items-center gap-2", last && running ? "text-foreground/80" : "text-muted")}>
              <span className="flex size-3 shrink-0 items-center justify-center" aria-hidden>
                {last && running ? <span className="size-1.5 rounded-full bg-foreground/60" /> : <Check className="size-3" />}
              </span>
              <span className="truncate">{s}</span>
            </li>
          )
        })}
        {steps.length === 0 && <li className="text-2xs text-muted">{current}…</li>}
      </ol>
      {(output.length > 0 || preparing) && (
        <div className="mt-2.5 flex items-center gap-4 pl-6">
          {outputToggle}
          {editSetup}
        </div>
      )}
      {panel && <div className="mt-2.5 pl-6">{panel}</div>}
    </div>
  )
}

/** The setup command's output: monospace, its errors tinted, following new lines while at the end. */
function Output({ run }: { run: SetupRun }) {
  const ref = useRef<HTMLPreElement>(null)
  const stick = useRef(true)
  const lines = run.lines.filter((l) => l.stream !== "info")
  useEffect(() => {
    const el = ref.current
    if (el && stick.current) el.scrollTop = el.scrollHeight
  }, [lines.length])
  return (
    <div className="flex flex-col gap-1">
      <pre
        ref={ref}
        onScroll={(e) => {
          const el = e.currentTarget
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 24
        }}
        className="max-h-56 overflow-auto rounded-lg bg-background/60 px-3 py-2.5 font-mono text-xs leading-relaxed whitespace-pre-wrap text-foreground/75 select-text"
      >
        {lines.map((l, i) => (
          <div key={i} className={cn(l.stream === "err" && "text-warning/90")}>
            {l.text || " "}
          </div>
        ))}
      </pre>
      <span className="text-2xs px-1 text-muted">
        {run.command && <span className="font-mono">$ {run.command}</span>}
        {run.ended !== null && ` · ${duration(run.ended - run.started)}`}
        {run.state === "stopped" && " · stopped"}
      </span>
    </div>
  )
}
