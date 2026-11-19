import { ArrowDown, FolderClosed, X } from "lucide-react"
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"

import { checkUpdated, resumeDeletes, setReviewing, useChrome } from "./chrome"
import { Composer, Tasks } from "./composer"
import { BackgroundRow, Banner, CloseConfirm, ContextMeter, FaultScreen, QueueChips } from "./conversation"
import { FormCard } from "./form-card"
import { TrustCards } from "./hooks-card"
import { SessionHeader, SidebarToggle, Toasts } from "./header"
import { Review } from "./review"
import { Settings, sectionLabel } from "./settings"
import { ShortcutsSheet, useShortcuts } from "./shortcuts"
import { Sidebar } from "./sidebar"
import { closePeek, dismissNotice, openSettings, start, useStore, type Open, type Peek } from "./store"
import { Transcript } from "./transcript"
import { btn, cn, folderKey, sameFolder, Spinner, TitleBar, Tooltip, useEscape } from "./ui"
import { Home, NewSessionStart, SignIn } from "./welcome"

export function App() {
  const auth = useStore((s) => s.auth)
  const settings = useStore((s) => s.settings)
  const sidebarHidden = useChrome((c) => c.sidebarHidden)
  const fault = useStore((s) => s.fault)
  useEffect(() => {
    void start()
  }, [])
  useEffect(() => {
    if (auth !== "signed-in") return
    void checkUpdated()
    void resumeDeletes()
  }, [auth])
  useShortcuts()
  if (auth === "checking") return <div className="h-full bg-background" />
  // The app's own data can't be opened: signing in wouldn't help.
  if (fault) return <FaultScreen fault={fault} />
  if (auth === "signed-out")
    return (
      <>
        <SignIn />
        <ShortcutsSheet />
      </>
    )
  return (
    <div className="relative h-full bg-background">
      {/* Settings covers the window; the session stays as it was (scroll, draft, open panels). */}
      <div className="flex h-full" inert={settings !== null} aria-hidden={settings !== null || undefined}>
        {!sidebarHidden && <Sidebar />}
        <Main />
      </div>
      {settings && <Settings section={settings} />}
      {/* Notices and toasts share one column under the title bar: they never overlap, and never cover the message box. */}
      <div
        className="pointer-events-none fixed inset-x-0 top-14 z-30 flex flex-col items-center gap-2 px-6"
        role="status"
        aria-live="polite"
      >
        <Notice />
        <Toasts />
      </div>
      <ShortcutsSheet />
      <CloseConfirm />
    </div>
  )
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

function Main() {
  const open = useStore((s) => s.open)
  const reviewing = useChrome((c) => c.reviewing)
  const sidebarHidden = useChrome((c) => c.sidebarHidden)
  useEffect(() => setReviewing(false), [open?.sessionId])
  return (
    <main
      className={cn(
        "relative m-2 flex min-w-0 flex-1 flex-col overflow-hidden rounded-2xl bg-surface/40",
        !sidebarHidden && "ml-0"
      )}
    >
      {open ? (
        <>
          <SessionHeader open={open} />
          <Conversation open={open} />
          <div className="mx-auto w-full max-w-(--conversation-width) px-8 pb-5">
            <BackgroundRow open={open} />
            <Tasks todos={open.todos} />
            <Banner open={open} />
            <QueueChips open={open} />
            <Composer open={open} />
            <div className="mt-2 flex items-center gap-3 px-2">
              <span className="text-2xs flex min-w-0 items-center gap-1 text-muted" title={open.directory}>
                <FolderClosed className="size-3 shrink-0" strokeWidth={1.75} />
                <span className="truncate">
                  <FooterPlace directory={open.directory} />
                </span>
              </span>
              <ContextMeter open={open} />
            </div>
          </div>
          {reviewing && <Review open={open} onClose={() => setReviewing(false)} />}
          <PeekSheet />
        </>
      ) : (
        <>
          <TitleBar className={sidebarHidden ? "pl-2" : undefined} controls>
            {sidebarHidden && <SidebarToggle hidden />}
          </TitleBar>
          <Home />
        </>
      )}
    </main>
  )
}

function FooterPlace({ directory }: { directory: string }) {
  const scratch = useStore((s) => s.scratch)
  return <>{sameFolder(directory, scratch) ? "No project · runs in the app’s own folder" : directory}</>
}

function Conversation({ open }: { open: Open }) {
  const all = useStore((s) => s.permissions)
  const permissions = useMemo(() => all.filter((p) => p.rootId === open.sessionId), [all, open.sessionId])
  const asked = useStore((s) => s.questions)
  const questions = useMemo(() => asked.filter((q) => q.rootId === open.sessionId), [asked, open.sessionId])
  const pendingForms = useStore((s) => s.forms)
  // A form belongs to a folder: it shows in whichever session of that folder is open.
  const forms = useMemo(() => pendingForms.filter((f) => sameFolder(f.directory, open.directory)), [pendingForms, open.directory])
  // What a project's files want to run, waiting for an answer: their card counts as output, so
  // the view follows it.
  useStore((s) => s.trust[folderKey(open.directory)]?.filter((i) => i.decision === null).length ?? 0)
  const ref = useRef<HTMLDivElement>(null)
  const stick = useRef(true)
  /** Away from the bottom; `fresh` once output arrived since. */
  const [away, setAway] = useState<{ fresh: boolean } | null>(null)
  const size = useRef(0)
  const top = useRef(0)
  // A chat opens at its latest turn and follows new output while the reader is at the bottom.
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    if (stick.current) el.scrollTop = el.scrollHeight
    else if (el.scrollHeight > size.current + 4) setAway((a) => (a && !a.fresh ? { fresh: true } : a))
    size.current = el.scrollHeight
  })
  useEffect(() => {
    stick.current = true
    setAway(null)
  }, [open.sessionId])
  // The composer grows (lines, images) and the area shrinks, or a row grows on its own (a tool's
  // input being written): stay at the bottom if there.
  useEffect(() => {
    const el = ref.current
    if (!el) return
    const watch = new ResizeObserver(() => {
      if (stick.current) el.scrollTop = el.scrollHeight
      else if (el.scrollHeight > size.current + 4) setAway((a) => (a && !a.fresh ? { fresh: true } : a))
      size.current = el.scrollHeight
    })
    watch.observe(el)
    if (el.firstElementChild) watch.observe(el.firstElementChild)
    return () => watch.disconnect()
  }, [])
  const jump = () => {
    const el = ref.current
    if (!el) return
    stick.current = true
    setAway(null)
    el.scrollTo({ top: el.scrollHeight, behavior: document.documentElement.hasAttribute("data-reduce-motion") ? "auto" : "smooth" })
  }
  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      <div
        ref={ref}
        onScroll={(e) => {
          const el = e.currentTarget
          // Only the reader scrolling up lets go of the bottom: a row that grew between the
          // follow and this event is output, not the reader moving away.
          const up = el.scrollTop < top.current - 1
          top.current = el.scrollTop
          if (el.scrollHeight - el.scrollTop - el.clientHeight < 48) stick.current = true
          else if (up) stick.current = false
          setAway((a) => (stick.current ? null : (a ?? { fresh: false })))
        }}
        className="min-h-0 flex-1 overflow-y-auto"
      >
        <div className="mx-auto max-w-(--conversation-width) px-8 pt-6 pb-8">
          {open.loading ? (
            <div className="flex justify-center pt-16">
              <Spinner />
            </div>
          ) : !open.sessionId && open.items.length === 0 && open.live.length === 0 && open.status.kind === "idle" ? (
            <>
              <NewSessionStart open={open} />
              <TrustCards directory={open.directory} className="mt-6 flex flex-col gap-5" />
              {forms.length > 0 && (
                <div className="mt-6 flex flex-col gap-5">
                  {forms.map((f) => (
                    <FormCard key={f.formId} form={f} />
                  ))}
                </div>
              )}
            </>
          ) : (
            <Transcript open={open} permissions={permissions} questions={questions} forms={forms} />
          )}
        </div>
      </div>
      {away && !open.loading && (
        <button
          onClick={jump}
          className={btn("secondary", "sm", "absolute bottom-3 left-1/2 z-10 -translate-x-1/2 gap-1.5 bg-overlay")}
        >
          <ArrowDown className="size-3.5" />
          {away.fresh ? "New output" : "Jump to latest"}
        </button>
      )}
    </div>
  )
}

/** A subagent's transcript, read-only, over the session that ran it. */
function PeekSheet() {
  const peek = useStore((s) => s.peek)
  if (!peek) return null
  return <Peeking peek={peek} />
}

function Peeking({ peek }: { peek: Peek }) {
  const panel = useRef<HTMLDivElement>(null)
  useEscape(panel, closePeek)
  const view: Open = {
    key: "peek",
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
    queued: [],
    context: null,
    background: [],
    stopping: false,
    started: null,
  }
  return (
    <div ref={panel} className="absolute inset-0 z-10 flex justify-end bg-scrim" onClick={closePeek}>
      <aside
        className="flex h-full w-[min(44rem,92%)] flex-col bg-overlay"
        onClick={(e) => e.stopPropagation()}
        aria-label="Subagent"
      >
        <div className="flex h-12 shrink-0 items-center gap-3 px-5">
          <span className="text-2xs text-muted">Subagent</span>
          <span className="min-w-0 truncate text-sm font-medium">{peek.title}</span>
          <span className="ml-auto">
            <Tooltip label="Close" keys="Esc">
              <button onClick={closePeek} className={btn("ghost", "sm", "text-muted", true)} aria-label="Close">
                <X className="size-4" />
              </button>
            </Tooltip>
          </span>
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
  const link = useStore((s) => s.noticeLink)
  if (!notice) return null
  return (
    <div className="text-dense pointer-events-auto flex w-max max-w-md items-start gap-3 rounded-xl bg-popover px-4 py-3">
      <span className="flex max-h-40 min-w-0 flex-1 flex-col items-start gap-2 overflow-y-auto">
        <span className="select-text">{notice}</span>
        {link && (
          <button
            onClick={() => {
              dismissNotice()
              openSettings(link)
            }}
            className={btn("secondary", "sm")}
          >
            Open {sectionLabel(link)}
          </button>
        )}
      </span>
      <Tooltip label="Dismiss" side="top">
        <button onClick={dismissNotice} className="text-muted hover:text-foreground" aria-label="Dismiss">
          <X className="size-4" />
        </button>
      </Tooltip>
    </div>
  )
}
