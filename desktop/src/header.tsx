/**
 * The session header (title, folder, git, Review, the editor and the session's own menu), the
 * sidebar's fold button, and the small notices at the window's foot: Undo for a delete, "Updated
 * to X", and short confirmations; and the confirm before deleting a worktree's changes that aren't applied.
 */
import {
  ArrowUpRight,
  Code2,
  Copy,
  Ellipsis,
  FileDown,
  FolderClosed,
  FolderOpen,
  PanelLeft,
  Pencil,
  Plus,
  Trash2,
  Wallet,
  X,
} from "lucide-react"
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react"

import {
  answerDelete,
  askDelete,
  dismissUpdated,
  editBudget,
  flash,
  loadEditors,
  preferredEditor,
  RELEASES,
  reviewInstead,
  setReviewing,
  toggleSidebar,
  undoDelete,
  useChrome,
  type DeleteAsk,
} from "./chrome"
import { HeaderBudget } from "./budget"
import { usePrefs } from "./prefs"
import { GitMenu, useChangeCounts } from "./review"
import { KEYS } from "./shortcuts"
import { PlaceLabel } from "./sidebar"
import {
  currentBridge,
  newSession,
  notify,
  openExternal,
  openInEditor,
  renameSession,
  reveal,
  useStore,
  useWorkDir,
  worktreeOf,
  type Open,
} from "./store"
import { btn, cn, Counts, MenuItem, MenuSeparator, Popover, sameFolder, TitleBar, Tooltip, useEscape } from "./ui"

/** A home folder as `~`. */
function tilde(path: string) {
  return path.replace(/^(\/home\/[^/]+|\/Users\/[^/]+|[A-Z]:\\Users\\[^\\]+)/i, "~")
}

/** Saves a session as JSON where the user picks; the notice says where. */
export async function exportSession(directory: string, sessionId: string) {
  try {
    const path = await currentBridge().exportSession(directory, sessionId)
    if (path) notify(`Saved to ${tilde(path)}`)
  } catch (e) {
    notify(e instanceof Error ? e.message : String(e))
  }
}

export function copyPath(directory: string) {
  navigator.clipboard.writeText(directory).then(
    () => flash("Path copied"),
    () => notify(`The path couldn’t be copied. It is ${directory}`)
  )
}

/** The editors, read once; the one files open in, or null when none was found. */
export function useEditor() {
  const editors = useChrome((c) => c.editors)
  // The preference picks among them; read as it changes.
  usePrefs((p) => p.general.editor)
  useEffect(() => void loadEditors(), [])
  return { editor: preferredEditor(editors), known: editors !== null }
}

/** Folds the sidebar away and back (Ctrl B). */
export function SidebarToggle({ hidden }: { hidden: boolean }) {
  return (
    <Tooltip label={hidden ? "Show sidebar" : "Hide sidebar"} keys={KEYS.sidebar}>
      <button
        type="button"
        onClick={() => toggleSidebar()}
        aria-label={hidden ? "Show sidebar" : "Hide sidebar"}
        aria-expanded={!hidden}
        className={btn("ghost", "sm", "text-muted", true)}
      >
        <PanelLeft className="size-4" strokeWidth={1.75} />
      </button>
    </Tooltip>
  )
}

/**
 * The header gives way as it narrows so the title stays readable: under ROOMY the editor button
 * keeps only its icon, under WIDE the folder goes, under NARROW "Review changes" keeps its counts.
 */
const WIDE = 960
const NARROW = 760
/** The editor's name needs this much more. */
const ROOMY = 1200

export function SessionHeader({ open }: { open: Open }) {
  const hidden = useChrome((c) => c.sidebarHidden)
  const { files, add, del } = useChangeCounts(open)
  const bar = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState(1280)
  const narrow = width < NARROW
  const wide = width >= WIDE
  const [renaming, setRenaming] = useState(false)
  // A worktree session's files (the editor, the folder) are in its worktree.
  const place = useWorkDir(open)
  // Its files are gone: nothing to review there.
  const gone = useStore((s) => {
    const w = worktreeOf(s, open.sessionId)
    return w?.state === "discarded" || w?.state === "missing"
  })
  useLayoutEffect(() => {
    const el = bar.current
    if (!el) return
    const watch = new ResizeObserver(() => setWidth(el.offsetWidth))
    watch.observe(el)
    return () => watch.disconnect()
  }, [])
  useEffect(() => setRenaming(false), [open.sessionId])
  return (
    <div ref={bar}>
      <TitleBar className={hidden ? "pl-2" : "pl-6"} controls>
        {hidden && (
          <div className="mr-3 flex shrink-0 items-center gap-0.5">
            <SidebarToggle hidden />
            <Tooltip label="New session" keys={KEYS.newSession}>
              <button
                type="button"
                onClick={() => void newSession()}
                aria-label="New session"
                className={btn("ghost", "sm", "text-muted", true)}
              >
                <Plus className="size-4" />
              </button>
            </Tooltip>
          </div>
        )}
        <div className="flex min-w-0 flex-1 items-center gap-3">
          {renaming && open.sessionId ? (
            <TitleInput open={open} onDone={() => setRenaming(false)} />
          ) : (
            <h1
              className="min-w-28 truncate text-sm font-medium"
              title={open.title}
            >
              {open.title}
            </h1>
          )}
          {wide && (
            <span className="text-2xs flex min-w-0 shrink-[2] items-center gap-1 text-muted" title={place}>
              <FolderClosed className="size-3 shrink-0" strokeWidth={1.75} />
              <span className="truncate">
                <PlaceLabel directory={open.directory} />
              </span>
            </span>
          )}
        </div>
        <div className="mr-2 ml-3 flex shrink-0 items-center gap-1">
          <EditorButton directory={place} compact={width < ROOMY} />
          <GitMenu open={open} compact={narrow} />
          <HeaderBudget open={open} />
          {files > 0 &&
            !gone &&
            (narrow ? (
              <Tooltip label="Review changes" keys={KEYS.review}>
                <button
                  onClick={() => setReviewing(true)}
                  aria-label={`Review changes, ${add} lines added, ${del} removed`}
                  className={btn("ghost", "sm", "shrink-0 text-muted")}
                >
                  <Counts add={add} del={del} />
                </button>
              </Tooltip>
            ) : (
              <button onClick={() => setReviewing(true)} className={btn("ghost", "sm", "shrink-0 gap-2 text-muted")}>
                <Counts add={add} del={del} />
                Review changes
              </button>
            ))}
          <SessionMenu open={open} onRename={() => setRenaming(true)} />
        </div>
      </TitleBar>
    </div>
  )
}

function TitleInput({ open, onDone }: { open: Open; onDone: () => void }) {
  const row = useStore((s) => s.sessions.find((r) => r.id === open.sessionId))
  return (
    <input
      autoFocus
      defaultValue={open.title}
      aria-label="Session name"
      onFocus={(e) => e.currentTarget.select()}
      onBlur={(e) => {
        const title = e.currentTarget.value.trim()
        onDone()
        if (row && title && title !== open.title) void renameSession(row, title)
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") e.currentTarget.blur()
        if (e.key === "Escape") {
          // Leaves the field only; nothing else takes this Escape.
          e.preventDefault()
          e.stopPropagation()
          e.currentTarget.value = open.title
          e.currentTarget.blur()
        }
      }}
      className="h-7 min-w-40 flex-1 rounded-md bg-default px-2 text-sm font-medium outline-none focus-visible:status-focused-field"
    />
  )
}

/** "Open in VS Code" (the folder in the preferred editor), else "Open folder". */
function EditorButton({ directory, compact }: { directory: string; compact: boolean }) {
  const scratch = useStore((s) => s.scratch)
  const { editor, known } = useEditor()
  if (sameFolder(directory, scratch) || !known) return null
  const label = editor ? `Open in ${editor.name}` : "Open folder"
  const run = () => (editor ? void openInEditor(directory) : void reveal(directory))
  const Icon = editor ? Code2 : FolderOpen
  if (compact)
    return (
      <Tooltip label={label}>
        <button type="button" onClick={run} aria-label={label} className={btn("ghost", "sm", "text-muted", true)}>
          <Icon className="size-4" strokeWidth={1.75} />
        </button>
      </Tooltip>
    )
  return (
    <button type="button" onClick={run} className={btn("ghost", "sm", "shrink-0 gap-1.5 text-muted")}>
      <Icon className="size-3.5" strokeWidth={1.75} />
      {label}
    </button>
  )
}

/** The session's own menu: the sidebar row's actions, here where the session is. */
function SessionMenu({ open, onRename }: { open: Open; onRename: () => void }) {
  const files = useWorkDir(open)
  const [show, setShow] = useState(false)
  const close = useCallback(() => setShow(false), [])
  const scratch = useStore((s) => s.scratch)
  const row = useStore((s) => s.sessions.find((r) => r.id === open.sessionId) ?? null)
  const budget = useStore((s) => (open.sessionId ? (s.usage[open.sessionId]?.budget ?? null) : null))
  const { editor } = useEditor()
  const own = sameFolder(open.directory, scratch)
  const go = (f: () => void) => () => {
    close()
    f()
  }
  return (
    <div className="relative">
      <Tooltip label="Session actions">
        <button
          type="button"
          onClick={() => setShow(!show)}
          aria-label="Session actions"
          aria-haspopup="menu"
          aria-expanded={show}
          className={btn("ghost", "sm", "text-muted", true)}
        >
          <Ellipsis className="size-4" />
        </button>
      </Tooltip>
      <Popover open={show} onClose={close} side="bottom" align="end" menu label="Session actions" className="w-60">
        {open.sessionId && (
          <MenuItem onSelect={go(onRename)}>
            <Pencil className="size-3.5 text-muted" /> Rename
          </MenuItem>
        )}
        {!own && editor && (
          <MenuItem onSelect={go(() => void openInEditor(files))}>
            <Code2 className="size-3.5 text-muted" /> Open in {editor.name}
          </MenuItem>
        )}
        {!own && (
          <MenuItem onSelect={go(() => void reveal(files))}>
            <FolderOpen className="size-3.5 text-muted" /> Open folder
          </MenuItem>
        )}
        <MenuItem onSelect={go(() => copyPath(files))}>
          <Copy className="size-3.5 text-muted" /> Copy path
        </MenuItem>
        {open.sessionId && (
          <MenuItem onSelect={go(() => void exportSession(open.directory, open.sessionId!))}>
            <FileDown className="size-3.5 text-muted" /> Export…
          </MenuItem>
        )}
        {open.sessionId && (
          // After the menu has gone and given focus back, or the popover would close at once.
          <MenuItem onSelect={go(() => setTimeout(() => editBudget(true), 0))}>
            <Wallet className="size-3.5 text-muted" /> {budget === null ? "Set budget…" : "Change budget…"}
          </MenuItem>
        )}
        {row && (
          <>
            <MenuSeparator />
            <MenuItem danger onSelect={go(() => void askDelete([row], row.id))}>
              <Trash2 className="size-3.5" /> Delete
            </MenuItem>
          </>
        )}
      </Popover>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Deleting a worktree's work
// ---------------------------------------------------------------------------

function files(n: number) {
  return `${n} changed file${n === 1 ? "" : "s"}`
}

/**
 * Before deleting sessions whose worktrees hold changes not applied to the main folder: what
 * goes with them, and for one session the way to keep it (Review & apply).
 */
export function DeleteConfirm() {
  const ask = useChrome((c) => c.deleteAsk)
  if (!ask) return null
  return <DeleteAsking key={ask.rows.map((r) => r.id).join()} ask={ask} />
}

function DeleteAsking({ ask }: { ask: DeleteAsk }) {
  const panel = useRef<HTMLDivElement>(null)
  const cancel = useCallback(() => answerDelete(false), [])
  useEscape(panel, cancel)
  useEffect(() => panel.current?.querySelector<HTMLButtonElement>("[data-cancel]")?.focus(), [])
  const one = ask.rows.length === 1 && ask.lost.length === 1 ? ask.lost[0] : null
  const n = ask.rows.length
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-scrim p-6" onClick={cancel}>
      <div
        ref={panel}
        role="alertdialog"
        aria-labelledby="delete-title"
        aria-describedby="delete-detail"
        onClick={(e) => e.stopPropagation()}
        className="w-full max-w-sm rounded-2xl bg-overlay p-5"
      >
        <h2 id="delete-title" className="text-sm font-medium">
          {one ? "Delete this session?" : `Delete ${n} session${n === 1 ? "" : "s"}?`}
        </h2>
        {one ? (
          <p id="delete-detail" className="text-dense mt-1.5 text-muted">
            Deletes the worktree and branch <span className="font-mono [overflow-wrap:anywhere] text-foreground/85">{one.row.worktree?.branch}</span>
            {one.files === null
              ? ". Its changes couldn’t be counted, so some may not be applied."
              : ` with ${files(one.files)} that ${one.files === 1 ? "isn’t" : "aren’t"} applied.`}
          </p>
        ) : (
          <>
            <p id="delete-detail" className="text-dense mt-1.5 text-muted">
              {ask.lost.length === 1 ? "This worktree has" : "These worktrees have"} changes that aren’t applied. Deleting removes{" "}
              {ask.lost.length === 1 ? "it and its branch" : "them and their branches"}.
            </p>
            <ul className="mt-3 flex max-h-44 flex-col gap-0.5 overflow-y-auto">
              {ask.lost.map(({ row, files: count }) => (
                <li key={row.id} className="text-dense flex min-w-0 items-baseline gap-3 rounded-md bg-default/30 px-2.5 py-1.5">
                  <span className="min-w-0 flex-1 truncate" title={row.title}>
                    {row.title}
                  </span>
                  <span className="text-2xs shrink-0 text-muted tabular-nums">
                    {count === null ? "Changes not counted" : `${count} file${count === 1 ? "" : "s"} not applied`}
                  </span>
                </li>
              ))}
            </ul>
          </>
        )}
        <div className="mt-5 flex items-center justify-end gap-2">
          <button data-cancel onClick={cancel} className={btn("ghost", "sm")}>
            Cancel
          </button>
          {one && (
            <button onClick={() => void reviewInstead(one.row)} className={btn("secondary", "sm")}>
              Review & apply
            </button>
          )}
          <button onClick={() => answerDelete(true)} className={btn("danger-soft", "sm")}>
            {n === 1 ? "Delete" : `Delete ${n}`}
          </button>
        </div>
      </div>
    </div>
  )
}

// ---------------------------------------------------------------------------
// The window's foot
// ---------------------------------------------------------------------------

/** Undo for a delete, "Updated to X", and short confirmations, at the window’s foot. */
export function Toasts() {
  const deleting = useChrome((c) => c.deleting)
  const updated = useChrome((c) => c.updated)
  const note = useChrome((c) => c.flash)
  if (!deleting && !updated && !note) return null
  const n = deleting?.rows.length ?? 0
  // In the window's foot column (app.tsx), under the notice.
  return (
    <>
      {deleting && (
        <Toast>
          <span className="min-w-0 truncate">
            {n === 1 ? `Deleted “${deleting.rows[0].title}”` : `Deleted ${n} sessions`}
          </span>
          <button onClick={undoDelete} className={btn("secondary", "sm", "shrink-0")}>
            Undo
          </button>
        </Toast>
      )}
      {updated && (
        <Toast>
          <span className="min-w-0 truncate">Updated to {updated}</span>
          <button
            onClick={() => {
              dismissUpdated()
              openExternal(RELEASES)
            }}
            className={btn("secondary", "sm", "shrink-0 gap-1")}
          >
            What’s new
            <ArrowUpRight className="size-3.5" />
          </button>
          <button onClick={dismissUpdated} aria-label="Dismiss" className="shrink-0 text-muted hover:text-foreground">
            <X className="size-4" />
          </button>
        </Toast>
      )}
      {note && <Toast plain>{note.text}</Toast>}
    </>
  )
}

function Toast({ children, plain = false }: { children: ReactNode; plain?: boolean }) {
  return (
    <div
      className={cn(
        "text-dense pointer-events-auto flex max-w-md min-w-0 items-center gap-3 rounded-xl bg-popover pl-4",
        plain ? "py-2.5 pr-4" : "py-2 pr-2"
      )}
    >
      {children}
    </div>
  )
}
