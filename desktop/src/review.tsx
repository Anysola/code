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
  ExternalLink,
  GitBranch,
  GitCommitHorizontal,
  GitPullRequest,
  RotateCcw,
  Sparkles,
  TriangleAlert,
  X,
} from "lucide-react"
import { useCallback, useEffect, useRef, useState } from "react"

import type { FileDiff } from "./bridge"
import {
  addComment,
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
  refreshGit,
  removeComment,
  restoreFile,
  suggestCommitMessage,
  useStore,
  type Open,
} from "./store"
import { EditCard } from "./transcript"
import { btn, cn, Counts, DiffView, Popover, sameFolder, Spinner, Tooltip, useEscape } from "./ui"

// ---------------------------------------------------------------------------
// Git
// ---------------------------------------------------------------------------

const TRUNKS = new Set(["main", "master", "trunk", "develop"])

const GIT_DOWNLOAD = "https://git-scm.com/downloads"

/** The branch in the header; its menu takes the folder's changes to a pull request. */
export function GitMenu({ open, compact = false }: { open: Open; compact?: boolean }) {
  const git = useStore((s) => s.git)
  const gitError = useStore((s) => s.gitError)
  const [show, setShow] = useState(false)
  const [retrying, setRetrying] = useState(false)
  const close = useCallback(() => setShow(false), [])
  if ((!git || !sameFolder(git.directory, open.directory)) && gitError && sameFolder(gitError.directory, open.directory))
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
  if (!git || !sameFolder(git.directory, open.directory)) return null
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
          <span className="min-w-0 truncate font-mono text-xs">{git.branch || "no branch"}</span>
          {conflicts > 0 ? (
            <span className="text-2xs shrink-0 text-warning tabular-nums">· {conflicts}!</span>
          ) : (
            changed > 0 && <span className="text-2xs shrink-0 tabular-nums">· {changed}</span>
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
                  onClick={() => void openInEditor(git.root ?? open.directory, f)}
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
                Commit {picked.length} file{picked.length === 1 ? "" : "s"} to {git.branch || "this commit"}
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
  const reload = useCallback(() => {
    setNet((r) => (r && "error" in r ? undefined : r))
    void readChanges().then(setNet)
  }, [])
  useEffect(reload, [reload, open.items.length])
  const panel = useRef<HTMLDivElement>(null)
  useEscape(panel, onClose)
  const edits = changedFiles(open)
  const count = net ? net.length : failed ? 0 : edits.length
  // A few files open with their diffs; more start as rows, so the list reads first.
  const startOpen = count <= 6
  return (
    <div ref={panel} className="absolute inset-0 z-10 flex justify-end bg-scrim" onClick={onClose}>
      <aside
        className="flex h-full w-[min(46rem,94%)] flex-col bg-overlay"
        onClick={(e) => e.stopPropagation()}
        aria-label="Changes"
      >
        <div className="flex min-h-12 shrink-0 flex-wrap items-center gap-x-3 gap-y-1 py-1.5 pr-3 pl-5">
          <span className="text-sm font-medium">Changes</span>
          <span className="text-2xs min-w-0 text-muted">
            {count} file{count === 1 ? "" : "s"}
            {comments.length > 0 &&
              ` · ${comments.length} comment${comments.length === 1 ? "" : "s"} for your next message`}
          </span>
          <span className="ml-auto flex items-center gap-1">
            {net && net.length > 1 && <RevertAll files={net} onDone={reload} />}
            <Tooltip label="Close" keys="Esc">
              <button onClick={onClose} className={btn("ghost", "sm", "text-muted", true)} aria-label="Close">
                <X className="size-4" />
              </button>
            </Tooltip>
          </span>
        </div>
        <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-4 pb-6">
          {net === undefined && !failed && (
            <div className="flex justify-center pt-16">
              <Spinner />
            </div>
          )}
          {failed && (
            <div className="flex flex-col items-start gap-3 px-1 pt-4">
              <p className="text-dense">This session’s changes couldn’t be read.</p>
              <p className="text-2xs max-h-24 overflow-y-auto whitespace-pre-wrap text-muted select-text">{failed}</p>
              <button onClick={reload} className={btn("secondary", "sm")}>
                Try again
              </button>
            </div>
          )}
          {net === null && (
            <>
              <p className="text-dense px-1 text-muted">
                This folder isn’t a git repository, so these are the edits as they were made and files can’t be reverted from here.
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
          {net?.length === 0 && <p className="text-dense px-1 pt-4 text-muted">Every file is back as it was before this session.</p>}
          {net?.map((f) => (
            <ReviewFile key={f.file} file={f} directory={open.directory} onRestored={reload} startOpen={startOpen} />
          ))}
        </div>
      </aside>
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
  onRestored,
  startOpen,
}: {
  file: FileDiff
  directory: string
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
          <Tooltip label={file.status === "added" ? "Delete this new file" : "Put this file back as it was before the session"}>
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
