/**
 * The session header's git menu (branch, commit, push, pull request) and the Review panel: what
 * the session changed, file by file, with a way to put a file back and to comment on lines.
 */
import {
  ArrowDownToLine,
  ArrowUpFromLine,
  ArrowUpRight,
  Check,
  ChevronDown,
  ChevronRight,
  CircleAlert,
  ExternalLink,
  GitBranch,
  GitCommitHorizontal,
  GitPullRequest,
  RotateCcw,
  Sparkles,
  TriangleAlert,
  X,
} from "lucide-react"
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react"

import type { ApplyCheck, FileDiff, SessionWorktree } from "./bridge"
import { askDiscard, flash, setReviewing, useChrome } from "./chrome"
import {
  addComment,
  applyCheck,
  applyWorktree,
  undoApplyWorktree,
  bringBase,
  discardWorktree,
  addToComposer,
  hereOf,
  gitBranch,
  gitCommit,
  gitIdentity,
  gitPr,
  gitPull,
  gitPush,
  openExternal,
  openInEditor,
  openNow,
  readChanges,
  recreateWorktree,
  refreshGit,
  removeComment,
  restoreFile,
  suggestCommitMessage,
  useStore,
  useWorkDir,
  useWorktree,
  useWorktreeInfo,
  type Here,
  type Open,
  currentBridge,
} from "./store"
import { EditCard } from "./transcript"
import { btn, cn, Counts, DiffView, Popover, sameFolder, Spinner, Tooltip, useEscape } from "./ui"
import { BranchName, worktreeTrouble } from "./worktree"

// ---------------------------------------------------------------------------
// Git
// ---------------------------------------------------------------------------

const TRUNKS = new Set(["main", "master", "trunk", "develop"])

const GIT_DOWNLOAD = "https://git-scm.com/downloads"

/** The branch in the header; its menu takes the folder's changes to a pull request. */
export function GitMenu({ open, compact = false }: { open: Open; compact?: boolean }) {
  const git = useStore((s) => s.git)
  const gitError = useStore((s) => s.gitError)
  const worktree = useWorktree(open.sessionId)
  // Where the session works: its worktree's git state is read there.
  const where = useWorkDir(open)
  const [show, setShow] = useState(false)
  const [retrying, setRetrying] = useState(false)
  const close = useCallback(() => setShow(false), [])
  if (worktree) return <WorktreeMenu open={open} worktree={worktree} compact={compact} />
  if ((!git || !sameFolder(git.directory, where)) && gitError && sameFolder(gitError.directory, where))
    return (
      <div className="relative shrink-0">
        <Tooltip label="Git couldn’t be read">
          <button
            onClick={() => setShow(!show)}
            aria-expanded={show}
            aria-label="Git couldn’t be read"
            className={btn("ghost", "sm", "gap-1.5 text-muted", compact)}
          >
            <GitBranch className="size-3.5 shrink-0" />
            {!compact && <span className="text-xs">Git</span>}
            <span className="size-1.5 shrink-0 rounded-full bg-warning" />
          </button>
        </Tooltip>
        <Popover open={show} onClose={close} side="bottom" align="end" label="Git" className="w-72">
          <div className="flex flex-col gap-3 p-1.5">
            <p className="text-dense">This folder’s branch and changes couldn’t be read.</p>
            <p className="text-2xs max-h-24 overflow-y-auto whitespace-pre-wrap text-muted select-text">{gitError.message}</p>
            <button
              disabled={retrying}
              onClick={() => {
                setRetrying(true)
                void refreshGit(open.directory).finally(() => {
                  setRetrying(false)
                  close()
                })
              }}
              className={btn("secondary", "sm", "gap-1.5 self-start")}
            >
              {retrying && <Spinner className="size-3.5" />}
              Try again
            </button>
          </div>
        </Popover>
      </div>
    )
  if (!git || !sameFolder(git.directory, where)) return null
  if (!git.repo) {
    if (git.git !== "missing") return null
    return (
      <div className="relative shrink-0">
        <Tooltip label="Git isn’t installed">
          <button
            onClick={() => setShow(!show)}
            aria-expanded={show}
            aria-label="Git isn’t installed"
            className={btn("ghost", "sm", "gap-1.5 text-muted", compact)}
          >
            <GitBranch className="size-3.5 shrink-0" />
            {!compact && <span className="text-xs">No Git</span>}
          </button>
        </Tooltip>
        <Popover open={show} onClose={close} side="bottom" align="end" label="Git" className="w-72">
          <div className="flex flex-col gap-3 p-1.5">
            <p className="text-dense">Git isn’t installed, so branches and commits can’t be shown here.</p>
            <p className="text-2xs text-muted">Install it, then open this folder again.</p>
            <button
              onClick={() => {
                close()
                openExternal(GIT_DOWNLOAD)
              }}
              className={btn("secondary", "sm", "gap-1.5 self-start")}
            >
              Download Git
              <ArrowUpRight className="size-3.5" />
            </button>
          </div>
        </Popover>
      </div>
    )
  }
  const changed = git.files.length
  const conflicts = git.unmerged?.length ?? 0
  return (
    <div className="relative shrink-0">
      <Tooltip label="Branch and changes">
        <button
          onClick={() => setShow(!show)}
          aria-expanded={show}
          aria-label={`Branch ${git.branch || "none"}, ${changed} changed`}
          className={btn("ghost", "sm", cn("gap-1.5 text-muted", compact ? "max-w-36" : "max-w-56"))}
        >
          <GitBranch className="size-3.5 shrink-0" />
          <span className="min-w-0 flex-1 truncate font-mono text-xs">{git.branch || "no branch"}</span>
          {conflicts > 0 ? (
            <span className="text-2xs shrink-0 whitespace-nowrap text-warning tabular-nums">· {conflicts}!</span>
          ) : (
            changed > 0 && <span className="text-2xs shrink-0 whitespace-nowrap tabular-nums">· {changed}</span>
          )}
          {git.ahead > 0 && <span className="text-2xs shrink-0 tabular-nums">↑{git.ahead}</span>}
          {git.behind > 0 && <span className="text-2xs shrink-0 tabular-nums">↓{git.behind}</span>}
        </button>
      </Tooltip>
      <Popover open={show} onClose={close} side="bottom" align="end" label="Branch and changes" className="w-96">
        <GitPanel open={open} onDone={close} />
      </Popover>
    </div>
  )
}

/**
 * A worktree session's branch in the header: read-only (no commit, push, pull request or new
 * branch from here); its way out is Review & apply.
 */
function WorktreeMenu({ open, worktree, compact }: { open: Open; worktree: SessionWorktree; compact: boolean }) {
  const git = useStore((s) => s.git)
  const [show, setShow] = useState(false)
  const here = hereOf(open)
  const undo = useUndoApply(here, () => setShow(false))
  const close = useCallback(() => {
    setShow(false)
    undo.reset()
  }, [undo])
  // A folder that's gone has no changes to count (the last read of it may linger).
  const read = git?.repo && sameFolder(git.directory, worktree.path) && worktree.state !== "missing" ? git : null
  const changed = read?.files.length ?? 0
  const conflicts = read?.unmerged?.length ?? 0
  const trouble = worktreeTrouble(worktree)
  const base = `${worktree.baseBranch ?? "a detached commit"}@${worktree.baseCommit.slice(0, 7)}`
  return (
    <div className="relative shrink-0">
      <Tooltip label="This session’s worktree">
        <button
          onClick={() => setShow(!show)}
          aria-expanded={show}
          aria-label={`Worktree branch ${worktree.branch}${trouble ? `, ${trouble.toLowerCase()}` : ""}`}
          className={btn("ghost", "sm", cn("gap-1.5 text-muted", compact ? "max-w-40" : "max-w-64"))}
        >
          <BranchName worktree={worktree} className="flex-1 text-xs" hint={false} />
          {/* The count stays whole: only the branch gives way. */}
          {conflicts > 0 ? (
            <span className="text-2xs shrink-0 whitespace-nowrap text-warning tabular-nums">· {conflicts}!</span>
          ) : (
            changed > 0 && <span className="text-2xs shrink-0 whitespace-nowrap tabular-nums">· {changed}</span>
          )}
          {trouble && <span className="size-1.5 shrink-0 rounded-full bg-warning" aria-hidden />}
        </button>
      </Tooltip>
      <Popover open={show} onClose={close} side="bottom" align="end" label="This session’s worktree" className="w-80">
        <div className="flex flex-col gap-3 p-1.5">
          <div className="flex flex-col gap-1">
            <BranchName worktree={worktree} className="text-xs text-foreground [&>span]:whitespace-normal [&>span]:[overflow-wrap:anywhere]" />
            <span className="text-2xs text-muted">
              From{" "}<span className="font-mono">{base}</span>
            </span>
          </div>
          <p className="text-2xs text-muted">
            {worktree.state === "discarded"
              ? "Discarded: the worktree and its branch are gone."
              : trouble
                ? `${trouble}.`
                : "This session works in its own copy of the folder. Your main folder changes only when you apply."}
          </p>
          {read && changed > 0 && (
            <span className="text-dense">
              {changed} changed file{changed === 1 ? "" : "s"}
            </span>
          )}
          {(worktree.state === "ready" || worktree.state === "applying") && (
            <div className="flex flex-wrap items-center gap-2">
              <button
                onClick={() => {
                  close()
                  setReviewing(true)
                }}
                className={btn("primary", "sm")}
              >
                Review & apply
              </button>
              {worktree.state === "ready" && worktree.appliedCommit && !undo.pushed && (
                <button disabled={undo.busy} onClick={() => void undo.run()} className={btn("ghost", "sm", "gap-1.5 text-muted")}>
                  {undo.busy ? <Spinner className="size-3" /> : <RotateCcw className="size-3.5" />}
                  Undo apply
                </button>
              )}
            </div>
          )}
          {undo.pushed && <PushedConfirm undo={undo} />}
          {undo.error && <p className="text-2xs text-danger select-text">{undo.error}</p>}
        </div>
      </Popover>
    </div>
  )
}

type Working = null | "commit" | "push" | "pull" | "pr" | "branch" | "suggest" | "identity"

function GitPanel({ open, onDone }: { open: Open; onDone: () => void }) {
  const git = useStore((s) => s.git)
  const [message, setMessage] = useState("")
  const [branch, setBranch] = useState("")
  const [working, setWorking] = useState<Working>(null)
  const [error, setError] = useState<string | null>(null)
  const [pr, setPr] = useState<{ url: string; created: boolean } | null>(null)
  // Files left out of the commit; a file that appears later starts in.
  const [skipped, setSkipped] = useState<Set<string>>(new Set())
  if (!git?.repo) return null
  const unmerged = git.unmerged ?? []
  // The session's own files first, then the rest as git lists them.
  const mine = new Set(changedFiles(open).map((f) => f.file))
  const files = [...git.files].sort((a, b) => Number(mine.has(b.file)) - Number(mine.has(a.file)))
  const changed = files.length
  const picked = files.filter((f) => !skipped.has(f.file))
  const trunk = TRUNKS.has(git.branch)
  // Commits to push, or a branch the remote doesn't have yet.
  const canPush = !!git.remote && !git.detached && (git.ahead > 0 || !git.upstream)
  const canPull = !!git.upstream && git.behind > 0
  const canPr = !!git.upstream && git.ahead === 0 && !trunk
  const blocked =
    unmerged.length > 0
      ? `Resolve the conflicts in ${unmerged.length} file${unmerged.length === 1 ? "" : "s"} first`
      : !git.identity
        ? "Add your name and email first"
        : null

  const step = async (what: NonNullable<Working>, run: () => Promise<string | true>) => {
    setWorking(what)
    setError(null)
    const result = await run()
    setWorking(null)
    if (result !== true) setError(result)
    return result === true
  }
  const toggle = (file: string) => {
    const next = new Set(skipped)
    if (!next.delete(file)) next.add(file)
    setSkipped(next)
  }

  return (
    <div className="flex flex-col gap-3 p-1.5">
      <div className="flex items-center gap-2">
        <GitBranch className="size-3.5 shrink-0 text-muted" />
        <span className="min-w-0 flex-1 truncate font-mono text-xs">{git.branch || "no branch"}</span>
        <span className="text-2xs shrink-0 text-muted">
          {!git.remote
            ? "No remote"
            : !git.upstream
              ? "Not pushed"
              : git.ahead > 0 && git.behind > 0
                ? `${git.ahead} to push · ${git.behind} behind`
                : git.ahead > 0
                  ? `${git.ahead} to push`
                  : git.behind > 0
                    ? `${git.behind} behind`
                    : "Up to date"}
        </span>
      </div>

      {canPull && (
        <div className="flex items-center gap-3 rounded-lg bg-default/40 px-2.5 py-2">
          <span className="text-dense min-w-0 flex-1">
            {git.behind} new commit{git.behind === 1 ? "" : "s"} on {git.upstream}
          </span>
          <button
            disabled={working !== null}
            onClick={() => void step("pull", gitPull)}
            className={btn("secondary", "sm", "shrink-0 gap-1.5")}
          >
            {working === "pull" ? <Spinner className="size-3.5" /> : <ArrowDownToLine className="size-3.5" />}
            Pull
          </button>
        </div>
      )}

      {unmerged.length > 0 && (
        <div className="flex flex-col gap-1.5">
          <span className="text-2xs text-warning">
            {unmerged.length} file{unmerged.length === 1 ? " has" : "s have"} conflicts. Open each, fix it, and save.
          </span>
          <ul className="flex max-h-28 flex-col overflow-y-auto">
            {unmerged.map((f) => (
              <li key={f}>
                <button
                  onClick={() => void openInEditor(git.root ?? git.directory, f)}
                  className="flex h-7 w-full items-center gap-2 rounded-md px-1.5 text-left hover:bg-default/60 focus-visible:status-focused-field"
                >
                  <TriangleAlert className="size-3.5 shrink-0 text-warning" />
                  <span className="min-w-0 flex-1 truncate font-mono text-xs">{f}</span>
                  <ArrowUpRight className="size-3 shrink-0 text-muted" />
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}

      {changed > 0 && (
        <div className="flex flex-col gap-2">
          <div className="text-2xs flex items-center gap-2 text-muted">
            <button
              role="checkbox"
              aria-checked={picked.length === changed ? true : picked.length === 0 ? false : "mixed"}
              aria-label="Commit every file"
              onClick={() => setSkipped(picked.length === changed ? new Set(files.map((f) => f.file)) : new Set())}
              className="flex items-center gap-2 rounded hover:text-foreground focus-visible:status-focused-field"
            >
              <Box on={picked.length === changed} mixed={picked.length > 0 && picked.length < changed} />
              {picked.length === changed ? `${changed} changed file${changed === 1 ? "" : "s"}` : `${picked.length} of ${changed} files`}
            </button>
            {open.sessionId && (
              <button
                disabled={working !== null}
                onClick={() =>
                  void (async () => {
                    setWorking("suggest")
                    const text = await suggestCommitMessage()
                    setWorking(null)
                    if (text) setMessage(text)
                  })()
                }
                className="ml-auto flex items-center gap-1 rounded hover:text-foreground focus-visible:status-focused-field disabled:opacity-60"
              >
                {working === "suggest" ? <Spinner className="size-3" /> : <Sparkles className="size-3" />}
                Write the message for me
              </button>
            )}
          </div>
          <ul className="-mx-1 flex max-h-40 flex-col overflow-y-auto">
            {files.map((f, i) => {
              // With files from outside the session too, the two parts are named.
              const heading =
                mine.size > 0 && mine.size < changed && (i === 0 || mine.has(files[i - 1].file) !== mine.has(f.file))
                  ? mine.has(f.file)
                    ? "This session"
                    : "Other changes"
                  : null
              return (
                <li key={f.file}>
                  {heading && <div className="text-2xs flex h-6 items-end px-1 pb-1 text-muted">{heading}</div>}
                  <button
                    role="checkbox"
                    aria-checked={!skipped.has(f.file)}
                    onClick={() => toggle(f.file)}
                    title={f.file}
                    className="flex h-7 w-full items-center gap-2 rounded-md px-1 text-left hover:bg-default/60 focus-visible:status-focused-field"
                  >
                    <Box on={!skipped.has(f.file)} />
                    <span className="min-w-0 flex-1 truncate font-mono text-xs">{f.file}</span>
                    <span className="text-2xs shrink-0 text-muted">{STATUS[f.status] ?? "Edited"}</span>
                  </button>
                </li>
              )
            })}
          </ul>
          {!git.identity && unmerged.length === 0 && (
            <Identity working={working === "identity"} onSave={(name, email) => step("identity", () => gitIdentity(name, email))} />
          )}
          <textarea
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            rows={3}
            placeholder="Commit message"
            aria-label="Commit message"
            className="text-dense block w-full resize-none rounded-lg bg-field px-2.5 py-2 outline-none placeholder:text-field-placeholder focus-visible:status-focused-field"
          />
          {trunk && !blocked && (
            <p className="text-2xs text-muted">You’re on {git.branch}. Create a branch below first if this is for a pull request.</p>
          )}
          {blocked && unmerged.length > 0 && <p className="text-2xs text-muted">{blocked}.</p>}
          <div className="flex items-center gap-3">
            <button
              disabled={!message.trim() || picked.length === 0 || working !== null || blocked !== null}
              onClick={() =>
                void step("commit", () =>
                  gitCommit(message, picked.length === changed ? null : picked.map((f) => f.file))
                ).then((ok) => {
                  if (!ok) return
                  setMessage("")
                  setSkipped(new Set())
                })
              }
              className={btn("primary", "sm", "min-w-0 gap-1.5")}
            >
              {working === "commit" ? <Spinner className="size-3.5 text-accent-foreground" /> : <GitCommitHorizontal className="size-3.5 shrink-0" />}
              <span className="truncate">
                Commit {picked.length} file{picked.length === 1 ? "" : "s"}{git.branch ? ` to ${git.branch}` : ""}
              </span>
            </button>
          </div>
        </div>
      )}

      {(canPush || canPr || pr) && (
        <div className="flex flex-col gap-1.5">
          <div className="flex flex-wrap gap-2">
            {canPush && (
              <button
                disabled={working !== null}
                onClick={() => void step("push", gitPush)}
                className={btn(changed === 0 ? "primary" : "secondary", "sm", "gap-1.5")}
              >
                {working === "push" ? <Spinner className="size-3.5" /> : <ArrowUpFromLine className="size-3.5" />}
                Push
              </button>
            )}
            {canPr && !pr && (
              <button
                disabled={working !== null}
                onClick={() =>
                  void (async () => {
                    setWorking("pr")
                    setError(null)
                    const result = await gitPr(open.title, "")
                    setWorking(null)
                    if (typeof result === "string") setError(result)
                    else setPr(result)
                  })()
                }
                className={btn(changed === 0 ? "primary" : "secondary", "sm", "gap-1.5")}
              >
                {working === "pr" ? <Spinner className="size-3.5" /> : <GitPullRequest className="size-3.5" />}
                {git.gh ? "Create pull request" : "Open a pull request"}
              </button>
            )}
            {pr && (
              <button onClick={() => openExternal(pr.url)} className={btn("secondary", "sm", "gap-1.5")}>
                <ExternalLink className="size-3.5" />
                {pr.created ? `Pull request #${pr.url.split("/").pop()}` : "Finish it in the browser"}
              </button>
            )}
          </div>
          {canPr && !pr && changed > 0 && (
            <p className="text-2xs text-muted">Uncommitted changes won’t be included in the pull request.</p>
          )}
        </div>
      )}

      {changed === 0 && unmerged.length === 0 && !canPush && !canPr && !canPull && !pr && (
        <p className="text-dense text-muted">Nothing to commit or push.</p>
      )}

      <form
        onSubmit={(e) => {
          e.preventDefault()
          void step("branch", () => gitBranch(branch)).then((ok) => ok && setBranch(""))
        }}
        className="flex gap-2 border-t border-separator pt-3"
      >
        <input
          value={branch}
          onChange={(e) => setBranch(e.target.value)}
          placeholder="New branch"
          aria-label="New branch name"
          className="text-dense h-8 min-w-0 flex-1 rounded-lg bg-field px-2.5 font-mono outline-none placeholder:font-sans placeholder:text-field-placeholder focus-visible:status-focused-field"
        />
        <button type="submit" disabled={!branch.trim() || working !== null} className={btn("secondary", "sm")}>
          {working === "branch" ? <Spinner className="size-3.5" /> : "Create"}
        </button>
      </form>

      {error && <p className="text-dense max-h-32 overflow-y-auto whitespace-pre-wrap text-danger select-text">{error}</p>}
      {pr?.created && (
        <button onClick={onDone} className="text-2xs self-end rounded text-muted hover:text-foreground focus-visible:status-focused-field">
          Close
        </button>
      )}
    </div>
  )
}

const STATUS: Record<string, string> = { added: "Added", modified: "Edited", deleted: "Deleted" }

/** A checkbox's box, as the sidebar's selection draws it. */
function Box({ on, mixed = false }: { on: boolean; mixed?: boolean }) {
  return (
    <span
      aria-hidden
      className={cn(
        "flex size-3.5 shrink-0 items-center justify-center rounded-[4px] border",
        on || mixed ? "border-foreground bg-foreground text-background" : "border-muted"
      )}
    >
      {on && <Check className="size-2.5" strokeWidth={3.5} />}
      {mixed && !on && <span className="h-0.5 w-1.5 rounded-full bg-background" />}
    </span>
  )
}

/** Git's name and email, asked for once before the first commit on this computer. */
function Identity({ working, onSave }: { working: boolean; onSave: (name: string, email: string) => Promise<boolean> }) {
  const [name, setName] = useState("")
  const [email, setEmail] = useState("")
  const field =
    "text-dense h-8 min-w-0 flex-1 rounded-lg bg-field px-2.5 outline-none placeholder:text-field-placeholder focus-visible:status-focused-field"
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault()
        if (name.trim() && email.trim()) void onSave(name.trim(), email.trim())
      }}
      className="flex flex-col gap-2 rounded-lg bg-default/40 p-2.5"
    >
      <p className="text-2xs text-muted">Commits carry your name and email. Git asks once, for every project on this computer.</p>
      <div className="flex gap-2">
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="Name" aria-label="Name" autoComplete="name" className={field} />
        <input
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="Email"
          aria-label="Email"
          type="email"
          autoComplete="email"
          className={field}
        />
      </div>
      <button type="submit" disabled={!name.trim() || !email.includes("@") || working} className={btn("secondary", "sm", "gap-1.5 self-start")}>
        {working && <Spinner className="size-3.5" />}
        Save
      </button>
    </form>
  )
}

// ---------------------------------------------------------------------------
// Review
// ---------------------------------------------------------------------------

/** Every file the session's edits touched, their line counts summed (the view outside git). */
export function changedFiles(open: Open): { file: string; diffs: FileDiff[]; add: number; del: number }[] {
  const by = new Map<string, FileDiff[]>()
  for (const i of open.items)
    if (i.kind === "tool" && i.status === "completed")
      for (const f of i.files ?? []) by.set(f.file, [...(by.get(f.file) ?? []), f])
  return [...by.entries()].map(([file, diffs]) => ({
    file,
    diffs,
    add: diffs.reduce((n, d) => n + d.additions, 0),
    del: diffs.reduce((n, d) => n + d.deletions, 0),
  }))
}

/**
 * What the header counts: in git, the session's net changes as the Review panel reads them (the
 * same line edited three times is one line), read again when an edit completes or the folder's
 * git state is read again (a file put back, a commit); outside git, or while the read fails, the
 * edits' own counts summed: their diffs carry only a few lines around each change, not the
 * file's text before and after, so they can't be netted.
 */
export function useChangeCounts(open: Open): { files: number; add: number; del: number } {
  const edits = changedFiles(open)
  const done = edits.reduce((n, f) => n + f.diffs.length, 0)
  const git = useStore((s) => s.git)
  const [net, setNet] = useState<{ sessionId: string; files: FileDiff[] | null | "failed" } | null>(null)
  const sessionId = open.sessionId
  useEffect(() => {
    if (!sessionId) return
    let live = true
    void readChanges().then((r) => {
      if (!live || openNow()?.sessionId !== sessionId) return
      setNet({ sessionId, files: r && "error" in r ? "failed" : r })
    })
    return () => {
      live = false
    }
  }, [sessionId, done, git])
  const read = net && net.sessionId === sessionId ? net.files : undefined
  // Not read yet for this session: nothing rather than a sum that is about to change.
  if (read === undefined) return { files: 0, add: 0, del: 0 }
  if (Array.isArray(read))
    return {
      files: read.length,
      add: read.reduce((n, f) => n + f.additions, 0),
      del: read.reduce((n, f) => n + f.deletions, 0),
    }
  return {
    files: edits.length,
    add: edits.reduce((n, f) => n + f.add, 0),
    del: edits.reduce((n, f) => n + f.del, 0),
  }
}

export function Review({ open, onClose }: { open: Open; onClose: () => void }) {
  // `undefined` while reading; `null` outside git; `error` when the read failed.
  const [read, setNet] = useState<FileDiff[] | null | undefined | { error: string }>(undefined)
  const net = read && "error" in read ? undefined : read
  const failed = read && "error" in read ? read.error : null
  const comments = useStore((s) => s.comments)
  const worktree = useWorktree(open.sessionId)
  const where = useWorkDir(open)
  const action = useChrome((c) => c.reviewAction)
  const reload = useCallback(() => {
    setNet((r) => (r && "error" in r ? undefined : r))
    void readChanges().then(setNet)
  }, [])
  // Its folder is gone (deleted outside the app, heard while Review is open): nothing to read.
  const gone = worktree?.state === "missing"
  // While its folder is being made again there is nothing to read yet; once ready it reads again.
  const readable = !worktree || (worktree.state !== "missing" && worktree.state !== "preparing")
  // A worktree's diff moves with its base (bringing the base in, an apply), and comes back with a recreated folder.
  useEffect(() => {
    if (readable) reload()
  }, [reload, readable, open.items.length, worktree?.baseCommit])
  const panel = useRef<HTMLDivElement>(null)
  useEscape(panel, onClose)
  const edits = changedFiles(open)
  const count = net ? net.length : failed ? 0 : edits.length
  const [recreating, setRecreating] = useState(false)
  // A few files open with their diffs; more start as rows, so the list reads first.
  const startOpen = count <= 6
  const add = net?.reduce((n, f) => n + f.additions, 0) ?? 0
  const del = net?.reduce((n, f) => n + f.deletions, 0) ?? 0
  // A worktree session: what applying would do, read when the panel opens and again on Apply.
  const [check, setCheck] = useState<ApplyCheck | { error: string } | null>(null)
  const [applying, setApplying] = useState(action === "apply")
  const here = hereOf(open)
  const ready = worktree?.state === "ready"
  useEffect(() => {
    if (!ready || !here) return
    let live = true
    void applyCheck(here).then((c) => live && setCheck(c))
    return () => {
      live = false
    }
    // Once per session and base.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready, open.sessionId, worktree?.baseCommit])
  useEffect(() => {
    if (action === "apply") setApplying(true)
  }, [action])
  const behind = check && "ok" in check && check.ok ? check.behind : 0
  const base = worktree ? (worktree.baseBranch ?? "base") : null
  return (
    <div ref={panel} className="absolute inset-0 z-10 flex justify-end bg-scrim" onClick={onClose}>
      <aside
        className="flex h-full w-[min(46rem,94%)] flex-col bg-overlay"
        onClick={(e) => e.stopPropagation()}
        aria-label="Changes"
      >
        <div className="flex min-h-12 shrink-0 flex-wrap items-center gap-x-3 gap-y-1 py-1.5 pr-3 pl-5">
          <span className="text-sm font-medium">Changes</span>
          {!gone && <span className="text-2xs min-w-0 text-muted">
            {count} file{count === 1 ? "" : "s"}
            {worktree && net && (
              <>
                {" · "}
                <span className="text-success">+{add}</span>{" "}<span className="text-danger">−{del}</span>{" vs "}
                <span className="font-mono">
                  {base}@{worktree.baseCommit.slice(0, 7)}
                </span>
              </>
            )}
            {behind > 0 && ` · ${base} has ${behind} new commit${behind === 1 ? "" : "s"}`}
            {comments.length > 0 &&
              ` · ${comments.length} comment${comments.length === 1 ? "" : "s"} for your next message`}
          </span>}
          <span className="ml-auto flex items-center gap-1">
            {net && net.length > 1 && !gone && <RevertAll files={net} onDone={reload} />}
            {worktree && worktree.state !== "discarded" && (
              <button onClick={() => askDiscard(true)} className={btn("ghost", "sm", "text-muted")}>
                Discard
              </button>
            )}
            {worktree && (ready || worktree.state === "applying") && !applying && (
              <button
                disabled={worktree.state === "applying" || !net?.length}
                onClick={() => setApplying(true)}
                className={btn("primary", "sm")}
              >
                Apply to main folder
              </button>
            )}
            <Tooltip label="Close" keys="Esc">
              <button onClick={onClose} className={btn("ghost", "sm", "text-muted", true)} aria-label="Close">
                <X className="size-4" />
              </button>
            </Tooltip>
          </span>
        </div>
        <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-4 pb-6">
          {gone && here && (
            <div className="flex flex-col items-start gap-3 px-1 pt-4" role="status">
              <div className="flex flex-col gap-1">
                <p className="text-dense flex items-center gap-2">
                  <CircleAlert className="size-3.5 shrink-0 text-warning" />
                  This session’s folder is gone, so its changes can’t be shown.
                </p>
                <p className="text-2xs pl-5.5 text-muted">
                  Recreate makes it again from its branch; uncommitted work comes back when it can be.
                  {worktree?.bringing && " Bringing main in wasn’t finished: it’s dropped, so bring main in again after recreating."}
                </p>
              </div>
              <button
                disabled={recreating}
                onClick={() => {
                  setRecreating(true)
                  void recreateWorktree(here).finally(() => setRecreating(false))
                }}
                className={btn("secondary", "sm", "ml-5.5 gap-1.5")}
              >
                {recreating && <Spinner className="size-3" />}
                Recreate
              </button>
            </div>
          )}
          {worktree && here && applying && !gone && (
            <ApplyBox
              here={here}
              worktree={worktree}
              directory={open.directory}
              files={net?.length ?? 0}
              ignored={check && "ok" in check && check.ok ? check.ignored : []}
              onClose={() => {
                setApplying(false)
                // The palette's Apply opens it again from here.
                if (action) setReviewing(true)
              }}
              onChecked={setCheck}
              onResolve={onClose}
            />
          )}
          {net === undefined && !failed && !gone && (
            <div className="flex justify-center pt-16">
              <Spinner />
            </div>
          )}
          {failed && !gone && (
            <div className="flex flex-col items-start gap-3 px-1 pt-4">
              <p className="text-dense">This session’s changes couldn’t be read.</p>
              <p className="text-2xs max-h-24 overflow-y-auto whitespace-pre-wrap text-muted select-text">{failed}</p>
              <button onClick={reload} className={btn("secondary", "sm")}>
                Try again
              </button>
            </div>
          )}
          {net === null && !gone && (
            <>
              <p className="text-dense px-1 text-muted">
                This folder isn’t a Git repository, so these are the edits as they were made, and files can’t be reverted.
              </p>
              {edits.map((f) => (
                <div key={f.file} className="flex shrink-0 flex-col gap-2">
                  {f.diffs.map((d, i) => (
                    <EditCard key={i} file={d} startOpen={startOpen} />
                  ))}
                </div>
              ))}
            </>
          )}
          {net?.length === 0 && !gone && (
            <div className="flex flex-col items-start gap-2.5 px-1 pt-4">
              <p className="text-dense text-muted">
                {worktree
                  ? worktree.appliedCommit && worktree.appliedCommit === worktree.baseCommit
                    ? `Everything is applied. New changes show here.`
                    : `No changes from ${base} yet.`
                  : "Every file is back as it was before this session."}
              </p>
              {worktree && worktree.state === "ready" && worktree.appliedCommit && here && (
                <UndoApplyButton here={here} onUndone={reload} />
              )}
            </div>
          )}
          {!gone && net?.map((f) => (
            <ReviewFile key={f.file} file={f} directory={where} base={base} onRestored={reload} startOpen={startOpen} />
          ))}
        </div>
      </aside>
    </div>
  )
}

const OPERATION = { merge: "merge", rebase: "rebase", "cherry-pick": "cherry-pick", revert: "revert" } as const

function UndoApplyButton({
  here,
  onUndone,
  onError,
  compact = false,
}: {
  here: Here
  onUndone: () => void
  onError?: (err: string | null) => void
  compact?: boolean
}) {
  const undo = useUndoApply(here, onUndone, onError)
  if (undo.pushed) return <PushedConfirm undo={undo} />
  return (
    <span className={cn("inline-flex flex-col items-start gap-1", compact && "items-end")}>
      <button disabled={undo.busy} onClick={() => void undo.run()} className={btn(compact ? "ghost" : "secondary", "sm", "gap-1.5 text-muted")}>
        {undo.busy ? <Spinner className="size-3" /> : <RotateCcw className="size-3.5" />}
        {compact ? "Undo" : "Undo apply"}
      </button>
      {undo.error && <span className="text-2xs text-danger select-text">{undo.error}</span>}
    </span>
  )
}

type UndoApply = ReturnType<typeof useUndoApply>

/**
 * Undoing the last apply, one way for every button that offers it: a commit a remote branch
 * already has is undone only once the user confirms.
 */
function useUndoApply(here: Here | null, onUndone: () => void, onError?: (err: string | null) => void) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [pushed, setPushed] = useState<string[] | null>(null)
  const run = async (confirmed = false) => {
    if (!here) return
    setBusy(true)
    setError(null)
    onError?.(null)
    const r = await undoApplyWorktree(here, confirmed)
    setBusy(false)
    if ("error" in r) {
      setPushed(null)
      if (onError) onError(r.error)
      else setError(r.error)
      return
    }
    if ("pushed" in r) return setPushed(r.pushed)
    setPushed(null)
    flash("Undid the last apply")
    onUndone()
  }
  const reset = () => {
    setPushed(null)
    setError(null)
  }
  return { busy, error, pushed, run, reset }
}

/** One line on what undoing a pushed commit does, and the confirm. */
function PushedConfirm({ undo }: { undo: UndoApply }) {
  const remotes = undo.pushed ?? []
  return (
    <span className="flex flex-col items-start gap-1.5" role="alert">
      <span className="text-2xs text-warning">
        Already pushed to <span className="font-mono">{remotes.join(", ")}</span>. Undoing moves only your local branch back.
      </span>
      <span className="flex items-center gap-1">
        <button disabled={undo.busy} onClick={() => void undo.run(true)} className={btn("secondary", "sm", "gap-1.5")}>
          {undo.busy ? <Spinner className="size-3" /> : <RotateCcw className="size-3.5" />}
          Undo anyway
        </button>
        <button disabled={undo.busy} onClick={undo.reset} className={btn("ghost", "sm", "text-muted")}>
          Keep it
        </button>
      </span>
    </span>
  )
}

type Phase =
  | { kind: "message" }
  | { kind: "checking" }
  | { kind: "confirm"; onto: string; stash: boolean }
  /** "Apply anyway" past conflict markers: the files listed, confirmed before the commit. */
  | { kind: "markers"; files: string[]; onto: string; stash: boolean }
  | { kind: "refused"; check: Exclude<ApplyCheck, { ok: true }> | { reason: "error"; message: string } }
  | { kind: "merged"; conflicts: string[]; onto: string }
  | { kind: "applying"; stash: boolean }
  | { kind: "done"; commit: string; onto: string; stashLeft: string | null }

/**
 * Apply to the main folder: the commit message (suggested once, editable), then the check, then
 * one commit. A refusal says why in a sentence and offers the way out it has.
 */
function ApplyBox({
  here,
  worktree,
  directory,
  files,
  ignored,
  onClose,
  onChecked,
  onResolve,
}: {
  here: Here
  worktree: SessionWorktree
  directory: string
  files: number
  /** From the last check: files the commit would carry that the main folder ignores (warned, not blocked). */
  ignored: string[]
  onClose: () => void
  onChecked: (c: ApplyCheck | { error: string }) => void
  onResolve: () => void
}) {
  const info = useWorktreeInfo(directory)
  // Git too old to apply is known before any check: no message to write for it.
  const old = info?.available === true && !info.canApply
  const [text, setText] = useState("")
  const [suggesting, setSuggesting] = useState(!old)
  const [phase, setPhase] = useState<Phase>(old ? { kind: "refused", check: { ok: false, reason: "git", message: "" } } : { kind: "message" })
  const [error, setError] = useState<string | null>(null)
  const typed = useRef(false)
  const box = useRef<HTMLTextAreaElement>(null)
  // One suggestion when the box opens (a model call on the session's budget); never over typing.
  useEffect(() => {
    if (old) return
    let live = true
    void suggestCommitMessage().then((m) => {
      if (!live) return
      setSuggesting(false)
      if (m && !typed.current) setText(m)
      box.current?.focus()
    })
    return () => {
      live = false
    }
  }, [])
  const base = worktree.baseBranch
  // Choices made in this box hold for its later tries: edits set aside, markers confirmed.
  const stashed = useRef(false)
  const marked = useRef(false)
  const apply = async (onto: string, stash: boolean) => {
    if (stash) stashed.current = true
    setPhase({ kind: "applying", stash })
    setError(null)
    const done = await applyWorktree(here, text.trim(), onto, stash || stashed.current, marked.current)
    if ("error" in done) {
      setError(done.error)
      setPhase({ kind: "message" })
      return
    }
    setPhase({ kind: "done", commit: done.commit, onto, stashLeft: done.stashLeft })
  }
  const run = async () => {
    setPhase({ kind: "checking" })
    setError(null)
    const c = await applyCheck(here)
    onChecked(c)
    if ("error" in c) return setPhase({ kind: "refused", check: { reason: "error", message: c.error } })
    if (!c.ok) return setPhase({ kind: "refused", check: c })
    if (base !== c.onto) return setPhase({ kind: "confirm", onto: c.onto, stash: false })
    // Ignored files not warned about yet: stop once so the warning is read; Apply again goes on.
    if (c.ignored.some((f) => !ignored.includes(f))) return setPhase({ kind: "message" })
    await apply(c.onto, false)
  }
  // The unresolved refusal names no branch: read where the main folder is now.
  const anyway = async (list: string[]) => {
    setPhase({ kind: "checking" })
    let onto: string | null = null
    try {
      const now = await currentBridge().worktreeInfo(directory)
      onto = now.available ? now.branch : null
    } catch (e) {
      return setPhase({ kind: "refused", check: { reason: "error", message: e instanceof Error ? e.message : String(e) } })
    }
    if (!onto) return setPhase({ kind: "refused", check: { ok: false, reason: "detached" } })
    setPhase({ kind: "markers", files: list, onto, stash: stashed.current })
  }
  const bring = async () => {
    setPhase({ kind: "checking" })
    const r = await bringBase(here)
    if ("error" in r) return setPhase({ kind: "refused", check: { reason: "error", message: r.error } })
    if (r.conflicts.length) return setPhase({ kind: "merged", conflicts: r.conflicts, onto: base ?? "the base" })
    await run()
  }
  // `merged`: right after bringing the base in here; otherwise markers were found with no merge
  // known to have left them, so the message doesn't name one.
  const ask = (list: string[], merged: boolean) => {
    addToComposer(
      merged
        ? `Bringing ${base ?? "the base branch"} in left conflict markers in ${list.join(", ")}. Resolve them, keeping what both sides meant, and run the tests that cover them.`
        : `These files still have conflict markers: ${list.join(", ")}. Resolve them, keeping what both sides meant, and run the tests that cover them.`
    )
    onResolve()
  }

  if (phase.kind === "done")
    return (
      <div className="flex shrink-0 flex-col gap-1.5 rounded-xl bg-surface px-4 py-3" role="status">
        <span className="text-dense flex items-center gap-2">
          <Check className="size-3.5 shrink-0 text-success" />
          <span>
            Applied as <span className="font-mono">{phase.commit.slice(0, 7)}</span> on {phase.onto}
          </span>
          <span className="ml-auto flex items-center gap-1">
            <UndoApplyButton here={here} onUndone={onClose} onError={setError} compact />
            <button onClick={onClose} className={btn("ghost", "sm", "text-muted")}>
              Done
            </button>
          </span>
        </span>
        {error && <span className="text-2xs text-danger select-text">{error}</span>}
        {phase.stashLeft && (
          <span className="text-2xs text-warning">
            Your main-folder edits couldn’t go back cleanly. They’re kept in <span className="font-mono">{phase.stashLeft}</span>.
          </span>
        )}
        <span className="text-2xs text-muted">The session goes on from this commit; you can apply again later.</span>
      </div>
    )

  const busy = phase.kind === "checking" || phase.kind === "applying"
  const refusal = phase.kind === "refused" ? phase.check : null
  return (
    <div className="flex shrink-0 flex-col gap-3 rounded-xl bg-surface p-3">
      <div className="flex items-center gap-2 px-1">
        <span className="text-dense">Apply to {base ?? "the main folder"}</span>
        <span className="text-2xs text-muted">
          One commit with {files} file{files === 1 ? "" : "s"}. Your hooks run as for any commit.
        </span>
      </div>
      {!old && <textarea
        ref={box}
        value={text}
        onChange={(e) => {
          typed.current = true
          setText(e.target.value)
        }}
        disabled={busy}
        rows={3}
        placeholder={suggesting ? "Writing a message…" : "Commit message"}
        aria-label="Commit message"
        className="text-dense block w-full resize-none rounded-lg bg-default/50 px-2.5 py-2 outline-none placeholder:text-field-placeholder focus-visible:status-focused-field disabled:opacity-70"
      />}
      {phase.kind === "confirm" && (
        <Refusal
          text={
            base
              ? `The main folder is on ${phase.onto}, not ${base}. Apply to ${phase.onto} instead?`
              : `This session started from a detached commit. Apply to ${phase.onto}?`
          }
        >
          <button onClick={() => void apply(phase.onto, phase.stash)} className={btn("primary", "sm")}>
            Apply to {phase.onto}
          </button>
        </Refusal>
      )}
      {phase.kind === "markers" && (
        <Refusal
          text={`${base && phase.onto !== base ? `The main folder is on ${phase.onto}, not ${base}. ` : ""}${phase.files.length === 1 ? "This file has" : "These files have"} conflict markers and will be committed as ${phase.files.length === 1 ? "it is" : "they are"}:`}
          files={phase.files}
          directory={worktree.path}
        >
          <button onClick={() => setPhase({ kind: "refused", check: { ok: false, reason: "unresolved", files: phase.files } })} className={btn("ghost", "sm")}>
            Back
          </button>
          <button
            disabled={!text.trim()}
            onClick={() => {
              marked.current = true
              void apply(phase.onto, phase.stash)
            }}
            className={btn("primary", "sm")}
          >
            {base && phase.onto !== base ? `Apply to ${phase.onto}` : "Apply with markers"}
          </button>
        </Refusal>
      )}
      {phase.kind === "merged" && (
        <Refusal text={`Brought ${phase.onto} in. ${phase.conflicts.length === 1 ? "1 file has conflicts: fix it" : `${phase.conflicts.length} files have conflicts: fix them`}, then apply.`} files={phase.conflicts} directory={worktree.path}>
          <button onClick={() => ask(phase.conflicts, true)} className={btn("secondary", "sm")}>
            Ask the agent to resolve
          </button>
        </Refusal>
      )}
      {refusal && <Refused check={refusal} worktree={worktree} main={directory} canApply={info?.available ? info.canApply : true} onBring={() => void bring()} onStash={(onto) => (base !== onto ? setPhase({ kind: "confirm", onto, stash: true }) : void apply(onto, true))} onAsk={(files) => ask(files, false)} onAnyway={text.trim() ? (files) => void anyway(files) : null} onAgain={() => void run()} />}
      {ignored.length > 0 && phase.kind !== "refused" && phase.kind !== "markers" && (
        <p className="text-2xs flex items-start gap-2 px-1 text-warning">
          <TriangleAlert className="mt-px size-3.5 shrink-0" />
          <span className="min-w-0 [overflow-wrap:anywhere]">
            {ignored.length === 1 ? "This file is" : "These files are"} ignored in the main folder but would be committed:{" "}
            <span className="font-mono">{ignored.join(", ")}</span>
          </span>
        </p>
      )}
      {error && <p className="text-dense max-h-32 overflow-y-auto px-1 whitespace-pre-wrap text-danger select-text">{error}</p>}
      <div className="flex items-center justify-end gap-2">
        <button onClick={onClose} disabled={phase.kind === "applying"} className={btn("ghost", "sm")}>
          Cancel
        </button>
        {phase.kind !== "confirm" && phase.kind !== "markers" && !old && (
          <button disabled={!text.trim() || busy || (suggesting && !typed.current)} onClick={() => void run()} className={btn("primary", "sm", "gap-1.5")}>
            {busy && <Spinner className="size-3.5 text-accent-foreground" />}
            {phase.kind === "applying" ? "Applying…" : phase.kind === "checking" ? "Checking…" : refusal ? "Check again" : "Apply"}
          </button>
        )}
      </div>
    </div>
  )
}

/** Why the apply can't go now, in one sentence, with the way out it has. */
function Refused({
  check,
  worktree,
  main,
  canApply,
  onBring,
  onStash,
  onAsk,
  onAnyway,
  onAgain: _again,
}: {
  check: Exclude<ApplyCheck, { ok: true }> | { reason: "error"; message: string }
  worktree: SessionWorktree
  /** The main folder: its edited files open from there. */
  main: string
  canApply: boolean
  onBring: () => void
  onStash: (onto: string) => void
  onAsk: (files: string[]) => void
  /** Apply past the markers after a confirm; null while there's no commit message. */
  onAnyway: ((files: string[]) => void) | null
  onAgain: () => void
}) {
  const base = worktree.baseBranch ?? "the base"
  switch (check.reason) {
    case "conflict":
      return (
        <Refusal
          text={`${check.files.length === 1 ? "This file" : `These ${check.files.length} files`} changed on ${check.onto} too, and the changes conflict.`}
          files={check.files}
          directory={worktree.path}
        >
          <Tooltip label={`Merges ${check.onto} into this session’s branch; conflicts stay there to fix`} side="top">
            <button onClick={onBring} className={btn("secondary", "sm")}>
              Bring {check.onto} in
            </button>
          </Tooltip>
        </Refusal>
      )
    case "unresolved":
      return (
        <Refusal
          text={check.files.length === 1 ? "This file still has conflict markers:" : `These ${check.files.length} files still have conflict markers:`}
          files={check.files}
          directory={worktree.path}
        >
          <button onClick={() => onAsk(check.files)} className={btn("secondary", "sm")}>
            Ask the agent to resolve
          </button>
          {onAnyway && check.anyway !== false && (
            <Tooltip label="For markers that are meant to be there, as in docs or test fixtures" side="top">
              <button onClick={() => onAnyway(check.files)} className={btn("ghost", "sm", "text-muted")}>
                Apply anyway
              </button>
            </Tooltip>
          )}
        </Refusal>
      )
    case "dirty":
      return (
        <Refusal
          text={`You have uncommitted edits in the main folder to ${check.files.length === 1 ? "a file" : "files"} this apply changes.`}
          files={check.files}
          directory={main}
        >
          <Tooltip label="Your edits are stashed, the commit is made, then they’re put back" side="top">
            <button onClick={() => onStash(check.onto)} className={btn("secondary", "sm")}>
              Set them aside and apply
            </button>
          </Tooltip>
        </Refusal>
      )
    case "staged":
      return <Refusal text="The main folder has staged changes, which would go into this commit. Commit or unstage them first." />
    case "detached":
      return <Refusal text="The main folder isn’t on a branch. Switch it to a branch to apply." />
    case "operation":
      return <Refusal text={`The main folder is in the middle of a ${OPERATION[check.what] ?? check.what}. Finish or abort it first.`} />
    case "empty":
      return <Refusal text={`Nothing to apply: the files match ${base}.`} />
    case "busy":
      return <Refusal text="The session is still working. Apply once it stops." />
    case "git":
      return !canApply ? (
        <Refusal text="Update Git to apply. Applying needs Git 2.40 or later; the worktree works as it is.">
          <button onClick={() => openExternal(GIT_DOWNLOAD)} className={btn("secondary", "sm", "gap-1")}>
            Download Git
            <ArrowUpRight className="size-3.5" />
          </button>
        </Refusal>
      ) : (
        <Refusal text={check.message} plain />
      )
    case "error":
      return <Refusal text={check.message} plain />
  }
}

function Refusal({
  text,
  files,
  directory,
  plain = false,
  children,
}: {
  text: string
  files?: string[]
  /** Files open in the editor from here; null: listed only. */
  directory?: string | null
  plain?: boolean
  children?: ReactNode
}) {
  return (
    <div className="flex flex-col gap-2 rounded-lg bg-default/40 px-3 py-2.5">
      <p className={cn("text-dense flex items-start gap-2", plain && "whitespace-pre-wrap select-text")}>
        <TriangleAlert className="mt-0.5 size-3.5 shrink-0 text-warning" />
        <span className="min-w-0">{text}</span>
      </p>
      {files && files.length > 0 && (
        <ul className="flex max-h-28 flex-col overflow-y-auto pl-5">
          {files.map((f) =>
            directory ? (
              <li key={f}>
                <button
                  onClick={() => void openInEditor(directory, f)}
                  className="flex h-7 w-full items-center gap-2 rounded-md px-1.5 text-left hover:bg-default/60 focus-visible:status-focused-field"
                >
                  <span className="min-w-0 flex-1 truncate font-mono text-xs">{f}</span>
                  <ArrowUpRight className="size-3 shrink-0 text-muted" />
                </button>
              </li>
            ) : (
              <li key={f} className="flex h-7 items-center px-1.5">
                <span className="min-w-0 truncate font-mono text-xs text-muted">{f}</span>
              </li>
            )
          )}
        </ul>
      )}
      {children && <div className="flex flex-wrap items-center gap-2 pl-5">{children}</div>}
    </div>
  )
}

/** Discard a worktree session's worktree and branch, after a confirm: it can't be undone. */
export function DiscardConfirm({ open }: { open: Open }) {
  const worktree = useWorktree(open.sessionId)
  const [files, setFiles] = useState<number | null>(null)
  const [working, setWorking] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const panel = useRef<HTMLDivElement>(null)
  const close = useCallback(() => askDiscard(false), [])
  useEscape(panel, close)
  useEffect(() => {
    let live = true
    void readChanges().then((r) => live && setFiles(Array.isArray(r) ? r.length : null))
    panel.current?.querySelector<HTMLButtonElement>("[data-cancel]")?.focus()
    return () => {
      live = false
    }
  }, [])
  const here = hereOf(open)
  if (!worktree || !here) return null
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-scrim p-6" onClick={close}>
      <div
        ref={panel}
        role="alertdialog"
        aria-labelledby="discard-title"
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-sm rounded-2xl bg-overlay p-5"
      >
        <h2 id="discard-title" className="text-sm font-medium">
          Discard this worktree?
        </h2>
        <p className="text-dense mt-1.5 text-muted">
          Deletes the worktree and branch <span className="font-mono text-foreground/85">{worktree.branch}</span>
          {files ? ` with ${files} changed file${files === 1 ? "" : "s"}` : ""}. The conversation stays.
        </p>
        {error && <p className="text-dense mt-3 whitespace-pre-wrap text-danger select-text">{error}</p>}
        <div className="mt-5 flex items-center justify-end gap-2">
          <button data-cancel onClick={close} className={btn("ghost", "sm")}>
            Cancel
          </button>
          <button
            disabled={working}
            onClick={() =>
              void (async () => {
                setWorking(true)
                setError(null)
                const r = await discardWorktree(here)
                setWorking(false)
                if (r !== true) return setError(r)
                close()
                setReviewing(false)
                flash("Worktree discarded")
              })()
            }
            className={btn("danger-soft", "sm", "gap-1.5")}
          >
            {working && <Spinner className="size-3.5" />}
            Discard
          </button>
        </div>
      </div>
    </div>
  )
}

/** Every file of the session back as it was before it, after a confirm in place. */
function RevertAll({ files, onDone }: { files: FileDiff[]; onDone: () => void }) {
  const [asking, setAsking] = useState(false)
  const [working, setWorking] = useState(false)
  const confirm = useRef<HTMLButtonElement>(null)
  useEffect(() => {
    if (asking) confirm.current?.focus()
  }, [asking])
  if (!asking)
    return (
      <button onClick={() => setAsking(true)} className={btn("ghost", "sm", "gap-1.5 text-muted")}>
        <RotateCcw className="size-3.5" />
        Revert all
      </button>
    )
  return (
    <span
      className="flex items-center gap-2"
      onKeyDown={(e) => {
        if (e.key !== "Escape") return
        e.preventDefault()
        e.stopPropagation()
        setAsking(false)
      }}
    >
      <span className="text-dense">Revert all {files.length} files?</span>
      <button
        ref={confirm}
        disabled={working}
        onClick={() =>
          void (async () => {
            setWorking(true)
            // One by one: a file that fails says so in the notice and the rest still go back,
            // all in this session even if another is opened meanwhile.
            const where = openNow()
            for (const f of files) await restoreFile(f.file, where)
            setWorking(false)
            setAsking(false)
            onDone()
          })()
        }
        className={btn("danger-soft", "sm", "gap-1.5")}
      >
        {working && <Spinner className="size-3.5" />}
        Revert all
      </button>
      <button onClick={() => setAsking(false)} className={btn("ghost", "sm")}>
        Cancel
      </button>
    </span>
  )
}

const VERB = { added: "Added", modified: "Edited", deleted: "Deleted" } as const

function ReviewFile({
  file,
  directory,
  base,
  onRestored,
  startOpen,
}: {
  file: FileDiff
  /** Where relative paths are: the worktree for a worktree session. */
  directory: string
  /** A worktree session's base branch: files go back as they are there. */
  base: string | null
  onRestored: () => void
  startOpen: boolean
}) {
  const all = useStore((s) => s.comments)
  const comments = all.filter((c) => c.file === file.file)
  const [confirm, setConfirm] = useState(false)
  const [working, setWorking] = useState(false)
  const [open, setOpen] = useState(startOpen)
  return (
    // shrink-0: the list scrolls; a card squeezed to fit it would cut its diff off.
    <div className="shrink-0 overflow-hidden rounded-xl bg-surface">
      <div className="flex h-10 items-center gap-2.5 pr-2 pl-3">
        <button
          onClick={() => setOpen(!open)}
          aria-expanded={open}
          aria-label={`${open ? "Hide" : "Show"} the changes to ${file.file}`}
          className="text-dense flex shrink-0 items-center gap-2.5 self-stretch rounded-md text-left focus-visible:status-focused-field"
        >
          {open ? (
            <ChevronDown className="size-3.5 shrink-0 text-muted" />
          ) : (
            <ChevronRight className="size-3.5 shrink-0 text-muted" />
          )}
          <span className="text-foreground/80">{VERB[file.status] ?? "Edited"}</span>
        </button>
        <span className="flex min-w-0 flex-1 items-center gap-2.5">
          {file.status === "deleted" ? (
            <span className="min-w-0 truncate font-mono text-xs text-muted">{file.file}</span>
          ) : (
            <Tooltip label="Open in the editor">
              <button
                onClick={() => void openInEditor(directory, file.file)}
                className="min-w-0 truncate rounded font-mono text-xs text-muted underline-offset-2 hover:text-foreground hover:underline focus-visible:status-focused-field"
              >
                {file.file}
              </button>
            </Tooltip>
          )}
          {comments.length > 0 && (
            <span className="text-2xs shrink-0 text-muted">
              · {comments.length} comment{comments.length === 1 ? "" : "s"}
            </span>
          )}
        </span>
        <Counts add={file.additions} del={file.deletions} />
        {confirm ? (
          <span className="flex shrink-0 items-center gap-1">
            <button
              disabled={working}
              onClick={() => {
                setWorking(true)
                void restoreFile(file.file).then((ok) => {
                  setWorking(false)
                  setConfirm(false)
                  if (ok) onRestored()
                })
              }}
              className={btn("danger-soft", "sm", "gap-1.5")}
            >
              {working ? <Spinner className="size-3.5" /> : <Check className="size-3.5" />}
              {file.status === "added" ? "Revert: delete it" : "Revert"}
            </button>
            <button onClick={() => setConfirm(false)} className={btn("ghost", "sm")}>
              Cancel
            </button>
          </span>
        ) : (
          <Tooltip label={file.status === "added" ? "Delete this new file" : base ? `Put this file back as on ${base}` : "Put this file back as it was before the session"}>
            <button onClick={() => setConfirm(true)} className={btn("ghost", "sm", "shrink-0 gap-1.5 text-muted")}>
              <RotateCcw className="size-3.5" />
              Revert
            </button>
          </Tooltip>
        )}
      </div>
      {open && (
        <DiffView
          patch={file.patch}
          className={cn("pb-2")}
          comments={{
            list: comments,
            add: (line, text) =>
              addComment({
                file: file.file,
                line: (line.kind === "del" ? line.old : line.new) ?? 0,
                removed: line.kind === "del",
                code: line.text,
                text,
              }),
            remove: removeComment,
          }}
        />
      )}
    </div>
  )
}
