import { FolderClosed, FolderOpen, MessageSquare, X } from "lucide-react"
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"

import { Composer, Tasks } from "./composer"
import {
  closePeek,
  dismissNotice,
  newScratchSession,
  newSession,
  openLoginPage,
  signIn,
  start,
  useStore,
  type Open,
} from "./store"
import { changedFiles, GitMenu, Review } from "./review"
import { PlaceLabel, Sidebar } from "./sidebar"
import { Transcript } from "./transcript"
import { btn, Counts, Kbd, Mark, Spinner, TitleBar, UsageMeter } from "./ui"

export function App() {
  const auth = useStore((s) => s.auth)
  useEffect(() => {
    void start()
  }, [])
  useShortcuts()
  if (auth === "checking") return <div className="h-full bg-background" />
  if (auth === "signed-out") return <SignIn />
  return (
    <div className="flex h-full bg-background">
      <Sidebar />
      <Main />
      <Notice />
    </div>
  )
}

function useShortcuts() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "n") {
        e.preventDefault()
        void newSession()
      }
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [])
}

// ---------------------------------------------------------------------------
// Sign-in
// ---------------------------------------------------------------------------

function SignIn() {
  const login = useStore((s) => s.login)
  const error = useStore((s) => s.loginError)
  const [starting, setStarting] = useState(false)
  return (
    <div className="flex h-full flex-col bg-background">
      <TitleBar controls />
      <main className="flex flex-1 flex-col items-center justify-center gap-8 pb-16">
        <Mark className="h-7 text-foreground" />
        <h1 className="sr-only">Sign in to Anysola Code</h1>
        {login ? (
          <div className="flex flex-col items-center gap-5">
            <div className="flex flex-col items-center gap-2">
              <span className="text-2xs tracking-wide text-muted">Code</span>
              <span className="font-mono text-3xl tracking-[0.2em] select-all">{login.code}</span>
            </div>
            <div className="text-dense flex items-center gap-2 text-muted">
              <Spinner />
              Waiting for approval in your browser
            </div>
            <button onClick={openLoginPage} className={btn("ghost", "sm", "text-muted")}>
              Open the page again
            </button>
          </div>
        ) : (
          <div className="flex flex-col items-center gap-4">
            <button
              onClick={() => {
                setStarting(true)
                void signIn().finally(() => setStarting(false))
              }}
              disabled={starting}
              className={btn("primary", "md", "min-w-56")}
            >
              {starting ? "Starting…" : "Sign in with Anysola"}
            </button>
            {error && <p className="text-dense max-w-sm text-center text-danger">{error}</p>}
          </div>
        )}
      </main>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

function Main() {
  const open = useStore((s) => s.open)
  const [reviewing, setReviewing] = useState(false)
  useEffect(() => setReviewing(false), [open?.sessionId])
  return (
    <main className="relative m-2 ml-0 flex min-w-0 flex-1 flex-col overflow-hidden rounded-2xl bg-surface/40">
      {open ? (
        <>
          <SessionHeader open={open} onReview={() => setReviewing(true)} />
          <Conversation open={open} />
          <div className="mx-auto w-full max-w-[50rem] px-8 pb-5">
            <Tasks todos={open.todos} />
            <Composer open={open} />
            <div className="mt-2 flex items-center gap-3 px-2">
              <span className="text-2xs flex min-w-0 items-center gap-1 text-muted" title={open.directory}>
                <FolderClosed className="size-3 shrink-0" strokeWidth={1.75} />
                <span className="truncate">
                  <FooterPlace directory={open.directory} />
                </span>
              </span>
              <UsageMeter percent={open.info?.fiveHour ?? null} className="ml-auto w-44 shrink-0" />
            </div>
          </div>
          {reviewing && <Review open={open} onClose={() => setReviewing(false)} />}
          <PeekSheet />
        </>
      ) : (
        <>
          <TitleBar controls />
          <div className="flex flex-1 flex-col items-center justify-center gap-2 pb-16">
            <button onClick={() => void newSession()} className={btn("secondary", "md", "min-w-60 gap-2")}>
              <FolderOpen className="size-4" />
              <span className="flex-1 text-left">Open a folder</span>
              <Kbd>Ctrl N</Kbd>
            </button>
            <button onClick={() => void newScratchSession()} className={btn("ghost", "md", "min-w-60 gap-2 text-muted")}>
              <MessageSquare className="size-4" />
              <span className="flex-1 text-left">Start without a project</span>
            </button>
          </div>
        </>
      )}
    </main>
  )
}

function FooterPlace({ directory }: { directory: string }) {
  const scratch = useStore((s) => s.scratch)
  return <>{directory === scratch ? "No project · runs in the app’s own folder" : directory}</>
}

function SessionHeader({ open, onReview }: { open: Open; onReview: () => void }) {
  const files = changedFiles(open)
  const add = files.reduce((n, f) => n + f.add, 0)
  const del = files.reduce((n, f) => n + f.del, 0)
  return (
    <TitleBar className="pl-6" controls>
      <div data-tauri-drag-region className="flex min-w-0 items-center gap-3">
        <h1 className="min-w-0 truncate text-sm font-medium">{open.title}</h1>
        <span className="text-2xs flex shrink-0 items-center gap-1 text-muted">
          <FolderClosed className="size-3" strokeWidth={1.75} />
          <PlaceLabel directory={open.directory} />
        </span>
      </div>
      <div className="mr-2 ml-auto flex shrink-0 items-center gap-1">
        <GitMenu open={open} />
        {files.length > 0 && (
          <button onClick={onReview} className={btn("ghost", "sm", "shrink-0 gap-2 text-muted")}>
            <Counts add={add} del={del} />
            Review changes
          </button>
        )}
      </div>
    </TitleBar>
  )
}

function Conversation({ open }: { open: Open }) {
  const all = useStore((s) => s.permissions)
  const permissions = useMemo(() => all.filter((p) => p.rootId === open.sessionId), [all, open.sessionId])
  const asked = useStore((s) => s.questions)
  const questions = useMemo(() => asked.filter((q) => q.rootId === open.sessionId), [asked, open.sessionId])
  const ref = useRef<HTMLDivElement>(null)
  const stick = useRef(true)
  // A chat opens at its latest turn and follows new output while the reader is at the bottom.
  useLayoutEffect(() => {
    const el = ref.current
    if (el && stick.current) el.scrollTop = el.scrollHeight
  })
  useEffect(() => {
    stick.current = true
  }, [open.sessionId])
  // The composer grows (lines, images) and the area shrinks: stay at the bottom if there.
  useEffect(() => {
    const el = ref.current
    if (!el) return
    const watch = new ResizeObserver(() => {
      if (stick.current) el.scrollTop = el.scrollHeight
    })
    watch.observe(el)
    return () => watch.disconnect()
  }, [])
  return (
    <div
      ref={ref}
      onScroll={(e) => {
        const el = e.currentTarget
        stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 48
      }}
      className="min-h-0 flex-1 overflow-y-auto"
    >
      <div className="mx-auto max-w-[50rem] px-8 pt-6 pb-8">
        {open.loading ? (
          <div className="flex justify-center pt-16">
            <Spinner />
          </div>
        ) : (
          <Transcript open={open} permissions={permissions} questions={questions} />
        )}
      </div>
    </div>
  )
}

/** A subagent's transcript, read-only, over the session that ran it. */
function PeekSheet() {
  const peek = useStore((s) => s.peek)
  useEffect(() => {
    if (!peek) return
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && closePeek()
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [peek])
  if (!peek) return null
  const view: Open = {
    directory: peek.directory,
    sessionId: peek.sessionId,
    title: peek.title,
    items: peek.items,
    live: [],
    info: null,
    status: { kind: "idle" },
    loading: peek.loading,
    revert: null,
    todos: [],
    lastAgent: null,
    fill: null,
  }
  return (
    <div className="absolute inset-0 z-10 flex justify-end bg-black/40" onClick={closePeek}>
      <aside
        className="flex h-full w-[min(44rem,92%)] flex-col bg-overlay"
        onClick={(e) => e.stopPropagation()}
        aria-label="Subagent"
      >
        <div className="flex h-12 shrink-0 items-center gap-3 px-5">
          <span className="text-2xs text-muted">Subagent</span>
          <span className="min-w-0 truncate text-sm font-medium">{peek.title}</span>
          <button onClick={closePeek} className={btn("ghost", "sm", "ml-auto text-muted", true)} aria-label="Close">
            <X className="size-4" />
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-6 pb-8">
          {peek.loading ? (
            <div className="flex justify-center pt-16">
              <Spinner />
            </div>
          ) : (
            <Transcript open={view} permissions={[]} questions={[]} readOnly />
          )}
        </div>
      </aside>
    </div>
  )
}

function Notice() {
  const notice = useStore((s) => s.notice)
  if (!notice) return null
  return (
    <div className="text-dense fixed right-6 bottom-6 z-20 flex max-w-sm items-start gap-3 rounded-xl bg-popover px-4 py-3">
      <span className="flex-1">{notice}</span>
      <button onClick={dismissNotice} className="text-muted hover:text-foreground" aria-label="Dismiss">
        <X className="size-4" />
      </button>
    </div>
  )
}
