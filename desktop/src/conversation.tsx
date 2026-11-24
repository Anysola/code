/**
 * What sits around the conversation: the banners over the composer (a full limit, no
 * connection, a lost sign-in, a spent budget, a missing model), a fork's note, the queued messages, the commands left running
 * in the background, the context meter, the close confirm and the screen for data that can't be
 * opened.
 */
import {
  ChevronDown,
  ChevronUp,
  CircleAlert,
  Clock,
  GitBranch,
  Image as ImageIcon,
  Pencil,
  SquareTerminal,
  Wallet,
  WifiOff,
  X,
} from "lucide-react"
import { useEffect, useRef, useState, type ReactNode } from "react"

import type { Account, Queued, SessionRow } from "./bridge"
import { atBudget, raiseTo, useUsage } from "./budget"
import { askDiscard, dismissForkNote, useChrome } from "./chrome"
import {
  backgroundStop,
  compact,
  currentBridge,
  forkSession,
  hereOf,
  isRunning,
  keepWorking,
  openSession,
  openModelPicker,
  openSettings,
  queueEdit,
  queueRemove,
  queueSend,
  queueSteer,
  quitNow,
  requireSignIn,
  retryConnection,
  revealFault,
  setAccount,
  setBudget,
  recreateWorktree,
  useStore,
  useWorktree,
  worktreeToFolder,
  type Open,
} from "./store"
import { btn, Mark, resetsIn, Spinner, TitleBar, Tooltip, useEscape } from "./ui"

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

/** "45s", "2m 14s", "1h 3m". */
export function duration(ms: number) {
  const s = Math.max(0, Math.round(ms / 1000))
  if (s < 60) return `${s}s`
  const m = Math.floor(s / 60)
  if (m < 60) return `${m}m ${s % 60}s`
  return `${Math.floor(m / 60)}h ${m % 60}m`
}

/** Time since `since`, ticking once a second while shown. */
export function Elapsed({ since }: { since: number }) {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(t)
  }, [])
  return <span className="tabular-nums">{duration(now - since)}</span>
}

// ---------------------------------------------------------------------------
// Banners over the composer
// ---------------------------------------------------------------------------

type Signed = Extract<Account, { expired: false }>

/** The window that refuses requests now: from the account, or from the turn that just hit it. */
export function useBlocked(open: Open): { scope: "five_hour" | "weekly"; resets: string | null } | null {
  const account = useStore((s) => (s.account && !s.account.expired ? (s.account as Signed) : null))
  const hit = open.status.kind === "failed" && open.status.errorKind === "limit"
  // The account is read again after a failure and after a reset is used: once it is known, it
  // decides. The failed turn only stands in while it isn't (it would keep the box shut for good).
  const scope = account ? account.blockedBy : hit ? "five_hour" : null
  if (!scope) return null
  const window = scope === "weekly" ? account?.weekly : account?.fiveHour
  return { scope, resets: resetsIn(window?.resetsAt ?? null) }
}

export function Strip({ icon, children, actions }: { icon: ReactNode; children: ReactNode; actions?: ReactNode }) {
  return (
    <div role="status" className="text-dense mb-2 flex min-h-10 items-center gap-2.5 rounded-xl bg-surface/70 py-1 pr-1.5 pl-3.5">
      <span className="shrink-0 text-muted">{icon}</span>
      <span className="min-w-0 flex-1 py-1">{children}</span>
      {actions && <span className="flex shrink-0 items-center gap-1">{actions}</span>}
    </div>
  )
}

/** One banner at a time, the one that explains why sending won't work now first. */
export function Banner({ open }: { open: Open }) {
  const blocked = useBlocked(open)
  const offline = useStore((s) => s.offline)
  const account = useStore((s) => (s.account && !s.account.expired ? (s.account as Signed) : null))
  const [redeeming, setRedeeming] = useState(false)
  const [problem, setProblem] = useState<string | null>(null)
  const usage = useUsage(open.sessionId)
  const status = open.status
  if (blocked) {
    const saved = account?.bank.find((r) => r.scope === "both" || (r.scope === "five_hour" && blocked.scope === "five_hour"))
    const redeem = async () => {
      if (!saved) return
      setRedeeming(true)
      setProblem(null)
      try {
        setAccount(await currentBridge().limitsRedeem())
      } catch (e) {
        setProblem(e instanceof Error ? e.message : String(e))
      } finally {
        setRedeeming(false)
      }
    }
    return (
      <Strip
        icon={<Clock className="size-3.5" />}
        actions={
          <>
            {saved && (
              <button onClick={() => void redeem()} disabled={redeeming} className={btn("secondary", "sm", "gap-2")}>
                {redeeming && <Spinner className="size-3" />}
                Use a saved reset
              </button>
            )}
            <button onClick={() => openSettings("usage")} className={btn("ghost", "sm", "text-muted")}>
              See usage
            </button>
          </>
        }
      >
        <span className="block">
          {blocked.scope === "weekly" ? "Weekly limit reached" : "5-hour limit reached"}
          {blocked.resets && <span className="text-muted"> · resets {blocked.resets}</span>}
        </span>
        <span className="text-2xs block text-muted">
          {problem ?? (saved ? "Sending waits until it resets, or until you use a saved reset." : "Sending waits until it resets.")}
        </span>
      </Strip>
    )
  }
  if (offline || (status.kind === "failed" && status.errorKind === "offline"))
    return (
      <Strip
        icon={<WifiOff className="size-3.5" />}
        actions={
          <button onClick={() => void retryConnection()} className={btn("ghost", "sm", "text-muted")}>
            Retry
          </button>
        }
      >
        Can’t reach Anysola. Check your connection.
      </Strip>
    )
  if (status.kind === "failed" && status.errorKind === "auth")
    return (
      <Strip
        icon={<CircleAlert className="size-3.5" />}
        actions={
          <button onClick={() => requireSignIn("Sign in again to keep working.")} className={btn("secondary", "sm")}>
            Sign in again
          </button>
        }
      >
        Your sign-in has ended. Your sessions are kept.
      </Strip>
    )
  if (atBudget(usage) && open.sessionId) {
    const { directory, sessionId } = open
    const raise = raiseTo(usage)
    return (
      <Strip
        icon={<Wallet className="size-3.5 text-warning" />}
        actions={
          <>
            {raise !== null && (
              <button onClick={() => void setBudget(directory, sessionId, raise)} className={btn("secondary", "sm")}>
                Raise to {raise}%
              </button>
            )}
            <button onClick={() => void setBudget(directory, sessionId, null)} className={btn("ghost", "sm", "text-muted")}>
              Remove budget
            </button>
          </>
        }
      >
        <span className="block">
          Budget reached <span className="text-muted">· {usage.budget}% of the 5-hour window</span>
        </span>
        <span className="text-2xs block text-muted">Raise it or remove it to keep going.</span>
      </Strip>
    )
  }
  if (status.kind === "failed" && status.errorKind === "model")
    return (
      <Strip
        icon={<CircleAlert className="size-3.5" />}
        actions={
          <button onClick={openModelPicker} className={btn("secondary", "sm")}>
            Choose another model
          </button>
        }
      >
        {status.error}
      </Strip>
    )
  return null
}

/** What a fork's files are: put back in its own worktree (and how well), or, outside git, as they are now. */
function forkFiles(worktree: SessionRow["worktree"]) {
  if (!worktree) return "Same folder. Files stay as they are now; nothing is rewound."
  const r = worktree.restored
  if (!r || r.exact) return "Its own worktree, with the files as they were then."
  if (r.fallback === "now") return "The files from that message are no longer kept, so this fork starts with the files as they are now."
  if (r.fallback === "edits")
    return "Starts from the main folder’s current commit with the original session’s edits as of that message. Other uncommitted changes in the main folder aren’t in it."
  const n = r.kept.length
  return `Files are as of that message, except ${n} large file${n === 1 ? "" : "s"}, which ${n === 1 ? "is" : "are"} as on the commit it started from.`
}

/** A fork's note until put away: where it came from, and how its files were put back. */
export function ForkNote({ open }: { open: Open }) {
  const row = useStore((s) => s.sessions.find((r) => r.id === open.sessionId) ?? null)
  const forkOf = row?.forkOf ?? null
  const listed = useStore((s) => (forkOf ? (s.sessions.find((r) => r.id === forkOf) ?? null) : null))
  // The list holds the latest sessions only: an older source is read on its own.
  const [read, setRead] = useState<{ id: string; row: SessionRow | null } | null>(null)
  useEffect(() => {
    if (!forkOf || listed) return
    let live = true
    currentBridge()
      .sessionRow(forkOf)
      .then(
        (r) => live && setRead({ id: forkOf, row: r }),
        () => live && setRead({ id: forkOf, row: null })
      )
    return () => {
      live = false
    }
  }, [forkOf, listed])
  // One waiting behind Undo is gone as far as the window goes.
  const deleting = useChrome((c) => (forkOf ? !!c.deleting?.ids.has(forkOf) : false))
  const source = deleting ? null : (listed ?? (read?.id === forkOf ? read.row : null))
  const gone = useChrome((c) => !!open.sessionId && c.forkNotes.has(open.sessionId))
  if (!row?.forkOf || gone) return null
  // Not known yet: nothing rather than a wrong "deleted".
  if (!source && !deleting && !listed && read?.id !== forkOf) return null
  return (
    <Strip
      icon={<GitBranch className="size-3.5" />}
      actions={
        <>
          {source && (
            <button onClick={() => void openSession(source)} className={btn("ghost", "sm", "text-muted")}>
              Open original
            </button>
          )}
          <Tooltip label="Dismiss" side="top">
            <button onClick={() => dismissForkNote(row.id)} aria-label="Dismiss" className={btn("ghost", "sm", "text-muted", true)}>
              <X className="size-3.5" />
            </button>
          </Tooltip>
        </>
      }
    >
      <span className="block truncate">
        {source ? (
          <>
            Branched from <span className="text-foreground">{source.title}</span>
          </>
        ) : (
          "Branched from a session that was deleted"
        )}
      </span>
      <span className="text-2xs block text-muted">{forkFiles(row.worktree)}</span>
    </Strip>
  )
}

/**
 * A worktree that isn't there to work in: being applied, failed to be made, gone, or discarded.
 * Over the composer, with the ways out.
 */
export function WorktreeStrip({ open }: { open: Open }) {
  const worktree = useWorktree(open.sessionId)
  const busy = useStore((s) => (open.sessionId ? s.busy.has(open.sessionId) : false))
  const [working, setWorking] = useState<null | "again" | "folder">(null)
  const here = hereOf(open)
  if (!worktree || !here) return null
  const act = (what: NonNullable<typeof working>, f: () => Promise<void>) => () => {
    setWorking(what)
    void f().finally(() => setWorking(null))
  }
  const fork = !busy && !isRunning(open) && (
    <button onClick={() => void forkSession(here.directory, here.sessionId, null)} className={btn("ghost", "sm", "text-muted")}>
      Branch
    </button>
  )
  switch (worktree.state) {
    case "applying":
      return (
        <Strip icon={<Spinner className="size-3.5" />}>
          Applying to {worktree.baseBranch ?? "the main folder"}…
        </Strip>
      )
    case "failed":
      return (
        <Strip
          icon={<CircleAlert className="size-3.5 text-warning" />}
          actions={
            <>
              <button disabled={working !== null} onClick={act("folder", () => worktreeToFolder(here))} className={btn("ghost", "sm", "gap-1.5 text-muted")}>
                {working === "folder" && <Spinner className="size-3" />}
                Work in the main folder instead
              </button>
              <button disabled={working !== null} onClick={act("again", () => recreateWorktree(here))} className={btn("secondary", "sm", "gap-1.5")}>
                {working === "again" && <Spinner className="size-3" />}
                Try again
              </button>
            </>
          }
        >
          <span className="block">The worktree couldn’t be made.</span>
          {worktree.error && <span className="text-2xs block text-muted select-text">{worktree.error}</span>}
        </Strip>
      )
    case "missing":
      return (
        <Strip
          icon={<CircleAlert className="size-3.5 text-warning" />}
          actions={
            <>
              <button onClick={() => askDiscard(true)} className={btn("ghost", "sm", "text-muted")}>
                Discard
              </button>
              {fork}
              <button disabled={working !== null} onClick={act("again", () => recreateWorktree(here))} className={btn("secondary", "sm", "gap-1.5")}>
                {working === "again" && <Spinner className="size-3" />}
                Recreate
              </button>
            </>
          }
        >
          <span className="block">This session’s folder is gone.</span>
          <span className="text-2xs block text-muted">
            Recreate makes it again from its branch; uncommitted work comes back when it can be.
            {worktree.bringing && " Bringing main in wasn’t finished: it’s dropped, so bring main in again after recreating."}
          </span>
        </Strip>
      )
    case "discarded":
      return (
        <Strip icon={<GitBranch className="size-3.5" />} actions={fork}>
          <span className="block">This session’s worktree was discarded.</span>
          <span className="text-2xs block text-muted">You can still read it, or branch it to keep going.</span>
        </Strip>
      )
    default:
      return null
  }
}

// ---------------------------------------------------------------------------
// Queue
// ---------------------------------------------------------------------------

/** Messages sent while a turn ran. During the turn: Send now, Edit, Remove. After a stop or a
 * failure they wait for the user: nothing fires on its own. */
export function QueueChips({ open }: { open: Open }) {
  const busy = useStore((s) => (open.sessionId ? s.busy.has(open.sessionId) : false))
  const worktree = useWorktree(open.sessionId)
  const queued = open.queued
  if (queued.length === 0) return null
  // Waiting for its worktree: it runs once that is ready (or in the main folder, if moved there).
  const waiting = !!worktree && (worktree.state === "preparing" || worktree.state === "failed" || worktree.state === "missing")
  const live = busy || isRunning(open) || waiting
  return (
    <div className="mb-2 flex flex-col gap-1">
      {!live && (
        <div className="text-2xs flex items-center gap-2 px-2 pb-0.5 text-muted">
          <span className="flex-1">
            {queued.length === 1 ? "This message wasn’t sent: the turn ended first." : `These ${queued.length} messages weren’t sent: the turn ended first.`}
          </span>
          <button onClick={() => void queueSend()} className={btn("primary", "sm", "h-7")}>
            {queued.length === 1 ? "Send" : "Send all"}
          </button>
        </div>
      )}
      {/* A short window keeps room for the conversation and the Send row. */}
      <ul className="flex max-h-[min(9rem,max(4.5rem,calc(100vh_-_30rem)))] flex-col gap-1 overflow-y-auto">
        {queued.map((q) => (
          <Chip key={q.id} q={q} live={live} waiting={waiting} />
        ))}
      </ul>
    </div>
  )
}

function Chip({ q, live, waiting }: { q: Queued; live: boolean; waiting: boolean }) {
  const [busy, setBusy] = useState(false)
  const act = (run: () => Promise<void>) => async () => {
    setBusy(true)
    try {
      await run()
    } finally {
      setBusy(false)
    }
  }
  // A message waiting for its worktree is held as a steer by the core; it is still the user's to edit.
  const steering = q.delivery === "steer" && !waiting
  return (
    <li className="text-dense flex h-9 shrink-0 items-center gap-2.5 rounded-xl bg-surface/70 pr-1 pl-3.5">
      <span className="shrink-0 text-2xs text-muted">{steering ? "Next step" : waiting ? "Waiting" : live ? "Queued" : "Not sent"}</span>
      <span className="min-w-0 flex-1 truncate" title={q.text}>
        {q.text || (q.images ? "" : "(empty)")}
      </span>
      {q.images > 0 && (
        <span className="text-2xs flex shrink-0 items-center gap-1 text-muted">
          <ImageIcon className="size-3" />
          {q.images}
        </span>
      )}
      <span className="flex shrink-0 items-center">
        {/* A message held for a session still being made has no turn to hand it to yet. */}
        {live && !waiting && !steering && !q.id.startsWith("held:") && (
          <Tooltip label="Hand it to the running turn at its next step" side="top">
            <button onClick={act(() => queueSteer(q))} disabled={busy} className={btn("ghost", "sm", "h-7 text-muted")}>
              Send now
            </button>
          </Tooltip>
        )}
        {!steering && (
          <Tooltip label="Edit" side="top">
            <button onClick={act(() => queueEdit(q))} disabled={busy} aria-label="Edit" className={btn("ghost", "sm", "size-7 text-muted", true)}>
              <Pencil className="size-3.5" />
            </button>
          </Tooltip>
        )}
        {!steering && (
          <Tooltip label="Remove" side="top">
            <button onClick={act(() => queueRemove(q))} disabled={busy} aria-label="Remove" className={btn("ghost", "sm", "size-7 text-muted", true)}>
              <X className="size-3.5" />
            </button>
          </Tooltip>
        )}
        {steering && <Spinner className="mr-2 size-3" />}
      </span>
    </li>
  )
}

// ---------------------------------------------------------------------------
// Background commands
// ---------------------------------------------------------------------------

export function BackgroundRow({ open }: { open: Open }) {
  const [shown, setShown] = useState(false)
  const list = open.background
  if (list.length === 0) return null
  return (
    <div className="mb-2 rounded-xl bg-surface/70 px-1.5 py-1">
      <button
        onClick={() => setShown(!shown)}
        aria-expanded={shown}
        className="text-dense flex h-8 w-full items-center gap-2.5 rounded-lg px-2 text-left hover:bg-default/40 focus-visible:status-focused-field"
      >
        <SquareTerminal className="size-3.5 shrink-0 text-muted" strokeWidth={1.75} />
        <span className="min-w-0 flex-1 truncate">
          {list.length === 1 ? "1 command running in the background" : `${list.length} commands running in the background`}
        </span>
        {shown ? <ChevronDown className="size-3.5 text-muted" /> : <ChevronUp className="size-3.5 text-muted" />}
      </button>
      {shown && (
        <ul className="flex max-h-48 flex-col overflow-y-auto pb-1">
          {list.map((b) => (
            <BackgroundCommandRow key={b.id} id={b.id} command={b.command} started={b.started} />
          ))}
        </ul>
      )}
    </div>
  )
}

function BackgroundCommandRow({ id, command, started }: { id: string; command: string; started: number }) {
  const [stopping, setStopping] = useState(false)
  return (
    <li className="text-dense flex h-8 shrink-0 items-center gap-2.5 px-2">
      <span className="min-w-0 flex-1 truncate font-mono text-xs text-foreground/80" title={command}>
        {command}
      </span>
      <span className="text-2xs shrink-0 text-muted">
        <Elapsed since={started} />
      </span>
      <button
        onClick={() => {
          setStopping(true)
          void backgroundStop(id).finally(() => setStopping(false))
        }}
        disabled={stopping}
        className={btn("ghost", "sm", "h-7 shrink-0 text-muted")}
      >
        {stopping ? "Stopping…" : "Stop"}
      </button>
    </li>
  )
}

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

/** Whether a folder's sessions compact on their own, read once per folder. */
const autoCompactRead = new Map<string, Promise<boolean>>()

function useAutoCompact(directory: string, wanted: boolean) {
  const [auto, setAuto] = useState<boolean | null>(null)
  useEffect(() => {
    if (!wanted) return
    let read = autoCompactRead.get(directory)
    if (!read) {
      read = currentBridge()
        .configGet(directory)
        .then((c) => c.autoCompact)
      // Unknown: offer Compact, the way that can't go wrong.
      read.catch(() => autoCompactRead.delete(directory))
      autoCompactRead.set(directory, read)
    }
    let live = true
    read.then((a) => live && setAuto(a)).catch(() => live && setAuto(false))
    return () => {
      live = false
    }
  }, [directory, wanted])
  return auto
}

/** How full the model's context is; near full, Compact (or a word that it happens on its own). */
export function ContextMeter({ open }: { open: Open }) {
  const ctx = open.context
  const percent = ctx && ctx.limit > 0 ? Math.min(100, Math.round((ctx.tokens / ctx.limit) * 100)) : null
  const full = percent !== null && percent >= 80
  const auto = useAutoCompact(open.directory, full)
  if (percent === null || !ctx) return null
  const r = 5.5
  const c = 2 * Math.PI * r
  const busy = isRunning(open)
  return (
    <span className="text-2xs ml-auto flex shrink-0 items-center gap-2 text-muted">
      {full &&
        (auto ? (
          <span>Compacts on its own when full</span>
        ) : (
          <button
            onClick={() => void compact()}
            disabled={busy}
            className="rounded-md px-1.5 py-0.5 text-foreground/80 hover:bg-default/60 hover:text-foreground focus-visible:status-focused-field disabled:opacity-50"
          >
            Compact
          </button>
        ))}
      <Tooltip label={`${ctx.tokens.toLocaleString("en-US")} of ${ctx.limit.toLocaleString("en-US")} tokens in context`} side="top">
        <span tabIndex={0} className="flex items-center gap-1.5 rounded-md outline-none focus-visible:status-focused-field">
          <svg viewBox="0 0 14 14" className="size-3.5 -rotate-90" aria-hidden>
            <circle cx="7" cy="7" r={r} fill="none" strokeWidth="2" className="stroke-default" />
            <circle
              cx="7"
              cy="7"
              r={r}
              fill="none"
              strokeWidth="2"
              strokeLinecap="round"
              strokeDasharray={`${(c * percent) / 100} ${c}`}
              className={percent >= 90 ? "stroke-warning" : "stroke-foreground/60"}
            />
          </svg>
          <span className="tabular-nums">Context {percent}%</span>
        </span>
      </Tooltip>
    </span>
  )
}

// ---------------------------------------------------------------------------
// Close while busy, data that can't be opened
// ---------------------------------------------------------------------------

export function CloseConfirm() {
  const closing = useStore((s) => s.closing)
  if (closing === null) return null
  return <Closing busy={closing} />
}

function Closing({ busy }: { busy: number }) {
  const panel = useRef<HTMLDivElement>(null)
  useEscape(panel, keepWorking)
  useEffect(() => panel.current?.querySelector<HTMLButtonElement>("[data-keep]")?.focus(), [])
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-scrim p-6" onClick={keepWorking}>
      <div
        ref={panel}
        role="alertdialog"
        aria-labelledby="closing-title"
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-sm rounded-2xl bg-overlay p-5"
      >
        <h2 id="closing-title" className="text-sm font-medium">
          {busy === 1 ? "A turn is still running. Close anyway?" : `${busy} turns are still running. Close anyway?`}
        </h2>
        <p className="text-dense mt-1.5 text-muted">Closing stops {busy === 1 ? "it" : "them"}. Finished work stays.</p>
        <div className="mt-5 flex items-center justify-end gap-2">
          <button data-keep onClick={keepWorking} className={btn("ghost", "sm")}>
            Keep working
          </button>
          <button onClick={quitNow} className={btn("danger-soft", "sm")}>
            Stop and close
          </button>
        </div>
      </div>
    </div>
  )
}

export function FaultScreen({ fault }: { fault: { message: string; folder: string } }) {
  return (
    <div className="flex h-full flex-col bg-background">
      <TitleBar controls />
      <main className="flex flex-1 flex-col items-center justify-center gap-6 px-8 pb-16">
        <Mark className="h-7 text-foreground" />
        <div className="flex max-w-md flex-col items-center gap-2 text-center">
          <h1 className="text-base font-medium">Can’t open your data</h1>
          <p className="text-dense text-muted select-text">{fault.message}</p>
          <p className="text-2xs font-mono text-muted/80 select-all">{fault.folder}</p>
        </div>
        <div className="flex items-center gap-2">
          <button onClick={revealFault} className={btn("secondary", "sm")}>
            Open folder
          </button>
          <button onClick={() => location.reload()} className={btn("ghost", "sm", "text-muted")}>
            Try again
          </button>
        </div>
      </main>
    </div>
  )
}
