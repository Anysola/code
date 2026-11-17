/**
 * The session header's git menu (branch, commit, push, pull request) and the Review panel: what
 * the session changed, file by file, with a way to put a file back and to comment on lines.
 */
import {
  ArrowUpFromLine,
  Check,
  ExternalLink,
  GitBranch,
  GitCommitHorizontal,
  GitPullRequest,
  RotateCcw,
  Sparkles,
  X,
} from "lucide-react"
import { useCallback, useEffect, useRef, useState } from "react"

import type { FileDiff } from "./bridge"
import {
  addComment,
  gitBranch,
  gitCommit,
  gitPr,
  gitPush,
  openExternal,
  readChanges,
  removeComment,
  restoreFile,
  suggestCommitMessage,
  useStore,
  type Open,
} from "./store"
import { EditCard } from "./transcript"
import { btn, cn, Counts, DiffView, Popover, Spinner, useEscape } from "./ui"

// ---------------------------------------------------------------------------
// Git
// ---------------------------------------------------------------------------

const TRUNKS = new Set(["main", "master", "trunk", "develop"])

/** The branch in the header; its menu takes the folder's changes to a pull request. */
export function GitMenu({ open }: { open: Open }) {
  const git = useStore((s) => s.git)
  const [show, setShow] = useState(false)
  const close = useCallback(() => setShow(false), [])
  if (!git?.repo || git.directory !== open.directory) return null
  const changed = git.files.length
  return (
    <div className="relative shrink-0">
      <button
        onClick={() => setShow(!show)}
        aria-expanded={show}
        title="Branch and changes"
        className={btn("ghost", "sm", "max-w-56 gap-1.5 text-muted")}
      >
        <GitBranch className="size-3.5 shrink-0" />
        <span className="min-w-0 truncate font-mono text-xs">{git.branch || "no branch"}</span>
        {changed > 0 && <span className="text-2xs shrink-0 tabular-nums">· {changed}</span>}
        {git.ahead > 0 && <span className="text-2xs shrink-0 tabular-nums">↑{git.ahead}</span>}
      </button>
      <Popover open={show} onClose={close} side="bottom" align="end" className="w-80">
        <GitPanel open={open} onDone={close} />
      </Popover>
    </div>
  )
}

function GitPanel({ open, onDone }: { open: Open; onDone: () => void }) {
  const git = useStore((s) => s.git)
  const [message, setMessage] = useState("")
  const [branch, setBranch] = useState("")
  const [working, setWorking] = useState<null | "commit" | "push" | "pr" | "branch" | "suggest">(null)
  const [error, setError] = useState<string | null>(null)
  const [pr, setPr] = useState<{ url: string; created: boolean } | null>(null)
  if (!git?.repo) return null
  const changed = git.files.length
  const trunk = TRUNKS.has(git.branch)
  // Commits to push, or a branch the remote doesn't have yet.
  const canPush = !!git.remote && !git.detached && (git.ahead > 0 || !git.upstream)
  const canPr = !!git.upstream && git.ahead === 0 && changed === 0 && !trunk

  const step = async (what: NonNullable<typeof working>, run: () => Promise<string | true>) => {
    setWorking(what)
    setError(null)
    const result = await run()
    setWorking(null)
    if (result !== true) setError(result)
    return result === true
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
              : git.ahead > 0
                ? `${git.ahead} to push`
                : git.behind > 0
                  ? `${git.behind} behind`
                  : "Up to date"}
        </span>
      </div>

      {changed > 0 && (
        <div className="flex flex-col gap-2">
          <div className="text-2xs flex items-center justify-between text-muted">
            <span>
              {changed} changed file{changed === 1 ? "" : "s"}
            </span>
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
                className="flex items-center gap-1 hover:text-foreground disabled:opacity-60"
              >
                {working === "suggest" ? <Spinner className="size-3" /> : <Sparkles className="size-3" />}
                Write it for me
              </button>
            )}
          </div>
          <textarea
            value={message}
            onChange={(e) => setMessage(e.target.value)}
            rows={3}
            placeholder="Commit message"
            aria-label="Commit message"
            className="text-dense block w-full resize-none rounded-lg bg-field px-2.5 py-2 outline-none placeholder:text-field-placeholder focus-visible:status-focused-field"
          />
          {trunk && <p className="text-2xs text-muted">You’re on {git.branch}. Create a branch below first if this is for a pull request.</p>}
          <button
            disabled={!message.trim() || working !== null}
            onClick={() =>
              void step("commit", () => gitCommit(message)).then((ok) => ok && setMessage(""))
            }
            className={btn("primary", "sm", "gap-1.5 self-start")}
          >
            {working === "commit" ? <Spinner className="size-3.5" /> : <GitCommitHorizontal className="size-3.5" />}
            Commit all
          </button>
        </div>
      )}

      {(canPush || canPr || pr) && (
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
              className={btn("primary", "sm", "gap-1.5")}
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
      )}

      {changed === 0 && !canPush && !canPr && !pr && (
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

      {error && <p className="text-dense max-h-32 overflow-y-auto whitespace-pre-wrap text-danger">{error}</p>}
      {pr?.created && (
        <button onClick={onDone} className="text-2xs self-end text-muted hover:text-foreground">
          Close
        </button>
      )}
    </div>
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

export function Review({ open, onClose }: { open: Open; onClose: () => void }) {
  // `undefined` while reading; `null` outside git.
  const [net, setNet] = useState<FileDiff[] | null | undefined>(undefined)
  const comments = useStore((s) => s.comments)
  const reload = useCallback(() => void readChanges().then(setNet), [])
  useEffect(reload, [reload, open.items.length])
  const panel = useRef<HTMLDivElement>(null)
  useEscape(panel, onClose)
  const edits = changedFiles(open)
  const count = net ? net.length : edits.length
  return (
    <div ref={panel} className="absolute inset-0 z-10 flex justify-end bg-black/40" onClick={onClose}>
      <aside
        className="flex h-full w-[min(46rem,94%)] flex-col bg-overlay"
        onClick={(e) => e.stopPropagation()}
        aria-label="Changes"
      >
        <div className="flex h-12 shrink-0 items-center gap-3 px-5">
          <span className="text-sm font-medium">Changes</span>
          <span className="text-2xs text-muted">
            {count} file{count === 1 ? "" : "s"}
            {comments.length > 0 &&
              ` · ${comments.length} comment${comments.length === 1 ? "" : "s"} for your next message`}
          </span>
          <button onClick={onClose} className={btn("ghost", "sm", "ml-auto text-muted", true)} aria-label="Close">
            <X className="size-4" />
          </button>
        </div>
        <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-4 pb-6">
          {net === undefined && (
            <div className="flex justify-center pt-16">
              <Spinner />
            </div>
          )}
          {net === null && (
            <>
              <p className="text-dense px-1 text-muted">
                This folder isn’t a git repository, so these are the edits as they were made and files can’t be put back from here.
              </p>
              {edits.map((f) => (
                <div key={f.file} className="flex flex-col gap-2">
                  {f.diffs.map((d, i) => (
                    <EditCard key={i} file={d} />
                  ))}
                </div>
              ))}
            </>
          )}
          {net?.length === 0 && <p className="text-dense px-1 pt-4 text-muted">Every file is back as it was before this session.</p>}
          {net?.map((f) => <ReviewFile key={f.file} file={f} onRestored={reload} />)}
        </div>
      </aside>
    </div>
  )
}

const VERB = { added: "Added", modified: "Changed", deleted: "Deleted" } as const

function ReviewFile({ file, onRestored }: { file: FileDiff; onRestored: () => void }) {
  const all = useStore((s) => s.comments)
  const comments = all.filter((c) => c.file === file.file)
  const [confirm, setConfirm] = useState(false)
  const [working, setWorking] = useState(false)
  const [open, setOpen] = useState(true)
  return (
    <div className="overflow-hidden rounded-xl bg-surface">
      <div className="flex h-10 items-center gap-2.5 pr-2 pl-3">
        <button
          onClick={() => setOpen(!open)}
          aria-expanded={open}
          className="text-dense flex min-w-0 flex-1 items-center gap-2.5 text-left focus-visible:status-focused-field"
        >
          <span className="shrink-0 text-foreground/80">{VERB[file.status] ?? "Changed"}</span>
          <span className="min-w-0 truncate font-mono text-xs text-muted">{file.file}</span>
          {comments.length > 0 && <span className="text-2xs shrink-0 text-muted">· {comments.length} comment{comments.length === 1 ? "" : "s"}</span>}
        </button>
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
              {file.status === "added" ? "Delete it" : "Put it back"}
            </button>
            <button onClick={() => setConfirm(false)} className={btn("ghost", "sm")}>
              Cancel
            </button>
          </span>
        ) : (
          <button
            onClick={() => setConfirm(true)}
            title={file.status === "added" ? "Delete this new file" : "Put this file back as it was before the session"}
            className={btn("ghost", "sm", "shrink-0 gap-1.5 text-muted")}
          >
            <RotateCcw className="size-3.5" />
            Revert
          </button>
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
