/**
 * The first screens: signing in, the window with no session open, and a new session before its
 * first message (what to ask, and what the mode lets the agent do).
 */
import { ArrowUpRight, FolderClosed, FolderOpen, MessageSquare } from "lucide-react"
import { useState } from "react"

import { SIGN_UP } from "./chrome"
import { modes } from "./composer"
import { usePrefs } from "./prefs"
import { useRecentFolders } from "./sidebar"
import {
  cancelSignIn,
  fillComposer,
  newScratchSession,
  newSession,
  openExternal,
  openLoginPage,
  setMode,
  signIn,
  useStore,
  type Open,
} from "./store"
import { btn, folderName, Kbd, Mark, sameFolder, Spinner, TitleBar } from "./ui"

/** What the app is, in one line: on the sign-in screen and the empty window. */
const WHAT =
  "An agent that works in a folder on your computer: it reads the code, makes the changes you ask for, and runs commands."

/** A home folder as `~`, for a path shown under a folder's name. */
function tilde(path: string) {
  return path.replace(/^(\/home\/[^/]+|\/Users\/[^/]+|[A-Z]:\\Users\\[^\\]+)/i, "~")
}

// ---------------------------------------------------------------------------
// Sign-in
// ---------------------------------------------------------------------------

export function SignIn() {
  const login = useStore((s) => s.login)
  const error = useStore((s) => s.loginError)
  const [starting, setStarting] = useState(false)
  const [copied, setCopied] = useState<"code" | "link" | null>(null)
  /** The clipboard refused: the text is selectable, so it says to copy it by hand. */
  const [refused, setRefused] = useState<"code" | "link" | null>(null)
  const copy = (what: "code" | "link", text: string) =>
    void navigator.clipboard.writeText(text).then(
      () => {
        setRefused(null)
        setCopied(what)
        setTimeout(() => setCopied(null), 1500)
      },
      () => setRefused(what)
    )
  return (
    <div className="flex h-full flex-col bg-background">
      <TitleBar controls />
      <main className="flex flex-1 flex-col items-center justify-center gap-8 overflow-y-auto px-8 pb-16">
        <div className="flex flex-col items-center gap-4">
          <Mark className="h-7 text-foreground" />
          <h1 className="text-lg font-medium">Anysola Code</h1>
          {!login && <p className="text-dense max-w-sm text-center text-pretty text-muted">{WHAT}</p>}
        </div>
        {login ? (
          <div className="flex flex-col items-center gap-5">
            <p className="text-sm">Enter this code on the page that opened</p>
            <button
              type="button"
              onClick={() => copy("code", login.code)}
              aria-label={`Code ${login.code}. Copy it`}
              className="flex flex-col items-center gap-1.5 rounded-2xl bg-surface px-8 pt-4 pb-3 transition-colors hover:bg-default/60 focus-visible:status-focused-field"
            >
              <span className="font-mono text-4xl font-medium tracking-[0.18em] select-all">{login.code}</span>
              <span className="text-2xs text-muted">{copied === "code" ? "Copied" : refused === "code" ? "Couldn’t copy. Select it and copy it instead" : "Click to copy"}</span>
            </button>
            <div className="text-dense flex items-center gap-2 text-muted">
              <Spinner />
              Waiting for you to approve it in the browser
            </div>
            <div className="flex flex-col items-center gap-1">
              <span className="text-2xs font-mono text-muted select-all">{login.url.replace(/^https?:\/\//, "")}</span>
              <div className="flex items-center gap-1">
                <button onClick={openLoginPage} className={btn("ghost", "sm", "text-muted")}>
                  Open the page again
                </button>
                <button onClick={() => copy("link", login.url)} className={btn("ghost", "sm", "text-muted")}>
                  {copied === "link" ? "Copied" : refused === "link" ? "Couldn’t copy. Select the address above" : "Copy link"}
                </button>
                <button onClick={cancelSignIn} className={btn("ghost", "sm", "text-muted")}>
                  Cancel
                </button>
              </div>
            </div>
            {error && <p className="text-dense max-w-sm text-center text-danger">{error}</p>}
          </div>
        ) : (
          <div className="flex flex-col items-center gap-3">
            <button
              onClick={() => {
                setStarting(true)
                void signIn().finally(() => setStarting(false))
              }}
              disabled={starting}
              className={btn("primary", "md", "min-w-60")}
            >
              {starting ? "Starting…" : "Sign in with Anysola"}
            </button>
            <button onClick={() => openExternal(SIGN_UP)} className={btn("ghost", "sm", "gap-1 text-muted")}>
              Create an account
              <ArrowUpRight className="size-3.5" />
            </button>
            {error && <p className="text-dense max-w-sm text-center text-danger">{error}</p>}
          </div>
        )}
      </main>
    </div>
  )
}

// ---------------------------------------------------------------------------
// No session open
// ---------------------------------------------------------------------------

/** The window with no session: open a folder (a recent one), or start with no project. */
export function Home() {
  const preferred = usePrefs((p) => p.general.defaultFolder)
  const recent = useRecentFolders(5)
  const folders = [...(preferred ? [preferred] : []), ...recent].slice(0, 5)
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center overflow-y-auto px-8">
      <div className="my-auto flex w-full max-w-md flex-col gap-6 py-8">
        <div className="flex flex-col gap-2 px-1">
          <h1 className="text-lg font-medium">Start a session</h1>
          <p className="text-dense text-pretty text-muted">{WHAT}</p>
        </div>
        <div className="flex flex-col gap-2">
          <button onClick={() => void newSession()} className={btn("primary", "md", "w-full gap-2")}>
            <FolderOpen className="size-4" />
            <span className="flex-1 text-left">{preferred ? "New session" : "Open a folder"}</span>
            <Kbd onPrimary>Ctrl N</Kbd>
          </button>
          <button onClick={() => void newScratchSession()} className={btn("ghost", "md", "w-full gap-2 text-muted")}>
            <MessageSquare className="size-4" />
            <span className="flex-1 text-left">Start with no project</span>
          </button>
        </div>
        {folders.length > 0 && (
          <section className="flex flex-col gap-1">
            <h2 className="text-2xs flex h-7 items-center px-1 font-medium tracking-wide text-muted">Recent folders</h2>
            <ul className="flex flex-col rounded-xl bg-surface p-1">
              {folders.map((d) => (
                <li key={d}>
                  <button
                    onClick={() => void newSession(d)}
                    title={d}
                    className="flex h-11 w-full items-center gap-3 rounded-lg px-3 text-left hover:bg-default/60 focus-visible:status-focused-field"
                  >
                    <FolderClosed className="size-4 shrink-0 text-muted" strokeWidth={1.75} />
                    <span className="shrink-0 text-sm">{folderName(d)}</span>
                    <span className="text-2xs min-w-0 flex-1 truncate font-mono text-muted">{tilde(d)}</span>
                    {sameFolder(d, preferred) && <span className="text-2xs shrink-0 text-muted">Default</span>}
                  </button>
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// A new session
// ---------------------------------------------------------------------------

type Starter = { label: string; detail: string; text: string }

function starters(git: boolean): Starter[] {
  return [
    {
      label: "Explain this project",
      detail: "How it’s organized and where to start",
      text: "Explain how this project is organized: the main parts, how they fit together, and where I should start reading.",
    },
    {
      label: "Find and fix a failing test",
      detail: "Runs the tests, then fixes what fails",
      text: "Run the tests, find one that fails, and fix it. Tell me what was wrong.",
    },
    {
      label: "Write instructions for the agent",
      detail: "/init creates AGENTS.md: commands, rules",
      text: "/init",
    },
    git
      ? {
          label: "Review my uncommitted changes",
          detail: "Bugs and risks in what changed",
          text: "/review",
        }
      : {
          label: "Suggest improvements",
          detail: "A short list, nothing changed yet",
          text: "Look through this project and suggest the three improvements that would help most. Don’t change anything yet.",
        },
  ]
}

/** Before a new session's first message: what to ask, and what the mode lets the agent do. */
export function NewSessionStart({ open }: { open: Open }) {
  const scratch = useStore((s) => s.scratch)
  const git = useStore((s) => s.git)
  const own = sameFolder(open.directory, scratch)
  const repo = !!git?.repo && sameFolder(git.directory, open.directory)
  return (
    // Lower in a tall window, toward the message box it fills; at the top of a short one.
    <div className="flex flex-col gap-5 pt-[clamp(0rem,calc(100vh-34rem),22rem)]">
      <div className="flex flex-col gap-1 px-1">
        <h2 className="text-sm font-medium">{own ? "No project" : folderName(open.directory)}</h2>
        <p className="text-dense text-muted">
          {own
            ? "This session works in the app’s own folder. Open a folder to work on a project."
            : "Ask for anything in this folder, or start with one of these."}
        </p>
      </div>
      {own ? (
        <div>
          <button onClick={() => void newSession()} className={btn("secondary", "sm", "gap-1.5")}>
            <FolderOpen className="size-3.5" />
            Open a folder
          </button>
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-2 [@media(min-width:720px)]:grid-cols-2">
          {starters(repo).map((s) => (
            <button
              key={s.label}
              onClick={() => {
                fillComposer(s.text)
                requestAnimationFrame(() =>
                  document.querySelector<HTMLTextAreaElement>('textarea[aria-label="Message"]')?.focus()
                )
              }}
              className="flex min-w-0 flex-col items-start gap-0.5 rounded-xl bg-surface px-3.5 py-2.5 text-left transition-colors hover:bg-default/60 focus-visible:status-focused-field"
            >
              <span className="w-full truncate text-sm">{s.label}</span>
              <span className="text-2xs w-full truncate text-muted">{s.detail}</span>
            </button>
          ))}
        </div>
      )}
      <ModeLine />
    </div>
  )
}

const MODE_WORDS: Record<string, string> = {
  ask: "every edit and command asks you first.",
  edits: "files change without asking; commands ask you first.",
  auto: "only what your rules name asks; the rest runs.",
}

/** The mode in words, with a one-click switch to the other usual one. */
function ModeLine() {
  const mode = useStore((s) => s.mode)
  const current = modes.find((m) => m.id === mode) ?? modes[1]
  const other = mode === "ask" ? "edits" : "ask"
  return (
    <p className="text-2xs flex flex-wrap items-center gap-x-2 gap-y-1 px-1 text-muted">
      <span>
        <span className="text-foreground/85">{current.label} mode:</span> {MODE_WORDS[mode] ?? current.hint}
      </span>
      <button
        onClick={() => setMode(other)}
        className="rounded underline decoration-muted/50 underline-offset-2 hover:text-foreground focus-visible:status-focused-field"
      >
        Switch to {other === "ask" ? "Ask" : "Edits"}
      </button>
    </p>
  )
}
