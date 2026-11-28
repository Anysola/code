/**
 * The composer: the message box with its `/` and `@` menus, images, the permission mode (and
 * Plan), the model, and the session's task list above it.
 */
import {
  ArrowUp,
  Check,
  ChevronDown,
  ChevronUp,
  Circle,
  ClipboardList,
  CircleCheck,
  FileText,
  Folder,
  LoaderCircle,
  MessageSquareText,
  Paperclip,
  Sparkles,
  SquareSlash,
  X,
} from "lucide-react"
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"

import type { Attachment, MemoryFiles, MemoryScope, Palette, Todo } from "./bridge"
import { useBlocked } from "./conversation"
import { isSendKey, usePrefs } from "./prefs"
import {
  addMemory,
  askAside,
  closeAside,
  commandHead,
  memoryFiles,
  OWN_COMMAND,
  OWN_COMMAND_SAID,
  openInEditor,
  chooseAgent,
  draftKey,
  EMPTY_DRAFT,
  isRunning,
  setDraft,
  chooseModel,
  compact,
  newSession,
  palette as readPalette,
  projectFiles,
  redo,
  removeComment,
  reply,
  retry,
  runCommand,
  send,
  pickMode,
  stop,
  STOP_AGAIN,
  undo,
  useStore,
  useWorktree,
  workDir,
  type Mode,
  type Open,
} from "./store"
import { anyLayer, btn, cn, Popover, Spinner, Tooltip } from "./ui"
import { PlaceChip } from "./worktree"

export const modes: { id: Mode | "plan"; label: string; hint: string }[] = [
  { id: "ask", label: "Ask", hint: "Ask before every edit and command" },
  { id: "edits", label: "Edits", hint: "Edit files without asking; ask before commands" },
  { id: "auto", label: "Auto", hint: "Run without asking, except where a rule says to ask or refuse" },
  { id: "plan", label: "Plan", hint: "Read and plan; change nothing until you approve" },
]

/** Images only, as the model takes them; larger ones are refused before they are read. */
const MAX_IMAGE_BYTES = 5 * 1024 * 1024

function readImage(file: File): Promise<Attachment> {
  return new Promise((resolve, reject) => {
    if (!file.type.startsWith("image/")) return reject(new Error(`${file.name} isn’t an image.`))
    if (file.size > MAX_IMAGE_BYTES) return reject(new Error(`${file.name} is over 5 MB.`))
    const reader = new FileReader()
    reader.onload = () => {
      const url = String(reader.result)
      resolve({ mime: file.type, data: url.slice(url.indexOf(",") + 1), name: file.name || "image.png" })
    }
    reader.onerror = () => reject(new Error(`Couldn’t read ${file.name}.`))
    reader.readAsDataURL(file)
  })
}

// ---------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------

type Row = {
  key: string
  label: string
  detail: string | null
  kind: "action" | "command" | "skill" | "file" | "folder" | "memory"
  /** Text that replaces the typed `/…` or `@…`. */
  insert: string
  /** Runs at once instead of inserting (the window's own commands). */
  run?: () => void
  /** A word at the row's end (what Enter does). */
  tag?: string
}

/** The window's own commands, for the composer and for Settings' list. `redo` shows only after a
 * rewind, `plan` and `build` only where the folder has those agents. */
export const APP_COMMANDS: { name: string; detail: string }[] = [
  { name: "undo", detail: "Rewind the last turn and put its message back here" },
  { name: "redo", detail: "Put back what the rewind undid" },
  { name: "retry", detail: "Send the last message again" },
  { name: "compact", detail: "Summarize the session so far to free up context" },
  { name: "btw", detail: "Ask a side question; the answer isn’t kept in the session" },
  { name: "memory", detail: "Add a line to AGENTS.md, or open it" },
  { name: "new", detail: "Start a new session in this folder" },
  { name: "plan", detail: "Plan before changing anything" },
  { name: "build", detail: "Make changes (the default agent)" },
]

const detail = (name: string) => APP_COMMANDS.find((c) => c.name === name)?.detail ?? ""

/** The window's own commands; the folder's commands (`/init`, `/review`, its own) follow. */
function actions(open: Open): Row[] {
  const rows: Row[] = [
    { name: "undo", detail: detail("undo"), run: () => void undo() },
    { name: "retry", detail: detail("retry"), run: () => void retry() },
    { name: "compact", detail: detail("compact"), run: () => void compact() },
    { name: "new", detail: detail("new"), run: () => void newSession(open.directory) },
  ]
    // Before the first message there is nothing to rewind, send again or summarize.
    .filter((a) => open.sessionId || a.name === "new")
    .map((a) => ({ key: `a:${a.name}`, label: `/${a.name}`, detail: a.detail, kind: "action", insert: "", run: a.run }))
  // These take what follows them: picking one writes it out, ready for the rest.
  for (const name of ["btw", "memory"])
    if (name === "memory" || open.sessionId)
      rows.push({ key: `a:${name}`, label: `/${name}`, detail: detail(name), kind: "action", insert: `/${name} ` })
  if (open.revert)
    rows.splice(1, 0, {
      key: "a:redo",
      label: "/redo",
      detail: detail("redo"),
      kind: "action",
      insert: "",
      run: () => void redo(),
    })
  for (const agent of open.info?.agents ?? [])
    if (agent.id === "plan" || agent.id === "build")
      rows.push({
        key: `a:${agent.id}`,
        label: `/${agent.id}`,
        detail: detail(agent.id),
        kind: "action",
        insert: "",
        run: () => void chooseAgent(agent.id),
      })
  return rows
}

const ACTION_NAMES = new Set(APP_COMMANDS.map((c) => c.name))

/** Prefix matches first, then matches inside, then letters in order (fuzzy). */
function rank(text: string, query: string): number {
  if (!query) return 1
  const t = text.toLowerCase()
  const q = query.toLowerCase()
  if (t.startsWith(q)) return 4
  const base = t.slice(t.lastIndexOf("/", t.length - 2) + 1)
  if (base.startsWith(q)) return 3
  if (t.includes(q)) return 2
  let at = 0
  for (const c of q) {
    at = t.indexOf(c, at)
    if (at < 0) return 0
    at++
  }
  return 1
}

function filter(rows: Row[], query: string, by: (r: Row) => string, limit: number) {
  return rows
    .map((r, i) => ({ r, i, score: rank(by(r), query) }))
    .filter((x) => x.score > 0)
    // Without a query the list keeps its order: the window's commands, then the folder's.
    .sort((a, b) => (query ? b.score - a.score || by(a.r).length - by(b.r).length : 0) || a.i - b.i)
    .slice(0, limit)
    .map((x) => x.r)
}

/** What the caret is typing: a command at the start of the message, or an `@` mention. */
function trigger(text: string, caret: number): { kind: "/" | "@"; query: string; start: number } | null {
  const before = text.slice(0, caret)
  const slash = /^\/(\S*)$/.exec(before)
  if (slash) return { kind: "/", query: slash[1], start: 0 }
  const at = /(^|\s)@(\S*)$/.exec(before)
  if (at) return { kind: "@", query: at[2], start: before.length - at[2].length - 1 }
  return null
}

const ICON = { action: SquareSlash, command: SquareSlash, skill: Sparkles, file: FileText, folder: Folder, memory: FileText }

/** `/memory`, then a space: its two files are the menu, and what follows is the note (matched
 * on the text with its leading spaces trimmed, as `/btw` and commands are). */
const MEMORY = /^\/memory\s([\s\S]*)$/

/** Why a recalled `/` line doesn't run on Enter. */
const RECALLED = "This line came back from before, so Enter doesn’t run it. Type its command again to run it."

/** How many characters two texts share from the start. */
function sharedStart(a: string, b: string) {
  let n = 0
  while (n < a.length && n < b.length && a[n] === b[n]) n++
  return n
}

/** The span `[from, to)` of `before` that an edit to `after` replaced (empty for an insertion):
 * what the two share at the start and at the end is outside it. */
function editedSpan(before: string, after: string): { from: number; to: number } {
  const from = sharedStart(before, after)
  let end = 0
  const room = Math.min(before.length, after.length) - from
  while (end < room && before[before.length - 1 - end] === after[after.length - 1 - end]) end++
  return { from, to: before.length - end }
}

/** Whether an edit (`span` of the old text) changed the command word at `[start, end)`: text
 * added or removed before or after it (spaces, a newline) only moves it. */
function touches(span: { from: number; to: number }, start: number, end: number) {
  return span.from === span.to ? span.from > start && span.from < end : span.from < end && span.to > start
}

/** Why Enter picked nothing in a menu. */
const PICK = "Pick a row with the arrows or Tab, or click it: this box holds text from before, so Enter doesn’t pick one."
const PICK_BARE = "Pick a command with the arrows or Tab, or type its name."

/** The menu `dismissed` marker for `/memory`'s files (a trigger's is where it starts). */
const MEMORY_MENU = -1

/** A home folder as `~`. */
function tilde(path: string) {
  return path.replace(/^(\/home\/[^/]+|\/Users\/[^/]+|[A-Z]:\\Users\\[^\\]+)/i, "~")
}

function Menu({
  rows,
  active,
  onPick,
  onHover,
  empty,
}: {
  rows: Row[]
  active: number
  onPick: (r: Row) => void
  onHover: (i: number) => void
  empty: string | null
}) {
  const list = useRef<HTMLDivElement>(null)
  useEffect(() => {
    list.current?.querySelector(`[data-i="${active}"]`)?.scrollIntoView({ block: "nearest" })
  }, [active])
  return (
    <div
      ref={list}
      role="listbox"
      aria-label="Suggestions"
      className="absolute right-0 bottom-full left-0 z-30 mb-2 max-h-72 overflow-y-auto rounded-xl bg-popover p-1.5"
    >
      {rows.length === 0 && empty && <p className="text-dense px-2.5 py-2 text-muted">{empty}</p>}
      {rows.map((r, i) => {
        const Icon = ICON[r.kind]
        return (
          <button
            key={r.key}
            data-i={i}
            role="option"
            aria-selected={i === active}
            // Keeps the caret in the message box.
            onMouseDown={(e) => e.preventDefault()}
            onClick={() => onPick(r)}
            onMouseMove={() => onHover(i)}
            className={cn(
              "flex h-9 w-full items-center gap-2.5 rounded-lg px-2.5 text-left",
              i === active && "bg-default-hover"
            )}
          >
            <Icon className="size-3.5 shrink-0 text-muted" strokeWidth={1.75} />
            <span className={cn("shrink-0 text-sm", (r.kind === "file" || r.kind === "folder") && "font-mono text-dense")}>
              {r.label}
            </span>
            {r.detail && <span className="text-2xs min-w-0 flex-1 truncate text-muted">{r.detail}</span>}
            {r.kind === "skill" && <span className="text-2xs ml-auto shrink-0 text-muted">Skill</span>}
            {r.tag && <span className="text-2xs ml-auto shrink-0 text-muted">{r.tag}</span>}
          </button>
        )
      })}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Composer
// ---------------------------------------------------------------------------

/** Over this many characters a paste is kept folded, as a chip, instead of filling the box. */
const LONG_PASTE = 1500

/** How long a second Escape has to stop a running turn. */
const ESC_AGAIN = 1000

/** Enter on an empty box allows a waiting ask, but not in the moment it appears: a message typed
 * with Enter just then would allow something unread. */
const ASK_SETTLE = 300

/** True once `ms` have gone by since `at` (false while `at` is unset). */
function usePast(at: number | false, ms: number) {
  const [past, setPast] = useState(false)
  useEffect(() => {
    setPast(false)
    if (!at) return
    const t = setTimeout(() => setPast(true), Math.max(0, at + ms - Date.now()))
    return () => clearTimeout(t)
  }, [at, ms])
  return past
}

/**
 * Two Escapes in a row stop the running turn, wherever focus is (the box, the transcript, the
 * sidebar, nowhere): the one place their presses are counted, so a press is never counted twice.
 * A press something else took is no press here: a panel, menu or sheet closing (layers, see
 * `useEscape`), a field it leaves (the title, a rename, the search), the sidebar's selection, the
 * box's own menu or the ask it denies. They all mark the key (`preventDefault`) or stop it before
 * the window. True while the first press waits for the second: the box shows the hint.
 */
function useDoubleEscape(running: boolean, sessionId: string | null, aside: boolean) {
  const [hint, setHint] = useState(false)
  const at = useRef(0)
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined)
  const runningRef = useRef(running)
  runningRef.current = running
  const asideRef = useRef<string | null>(null)
  asideRef.current = aside ? sessionId : null
  // Another session, or the turn ended: a press made before counts for nothing.
  useEffect(() => {
    at.current = 0
    clearTimeout(timer.current)
    setHint(false)
  }, [sessionId, running])
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // `anyLayer` too: a layer's close may already have dropped it by now, but not before.
      if (e.key !== "Escape" || e.defaultPrevented || e.repeat || e.isComposing || anyLayer()) return
      // A side question over the box closes first, and the press counts for nothing more. Not from
      // the terminal, whose programs (an editor) take Escape themselves.
      const open = asideRef.current
      if (open && !(e.target instanceof Element && e.target.closest(".xterm"))) {
        e.preventDefault()
        closeAside(open)
        requestAnimationFrame(() => document.querySelector<HTMLTextAreaElement>('textarea[aria-label="Message"]')?.focus())
        return
      }
      if (!runningRef.current) return
      clearTimeout(timer.current)
      if (Date.now() - at.current < ESC_AGAIN) {
        at.current = 0
        setHint(false)
        stop()
      } else {
        at.current = Date.now()
        setHint(true)
        timer.current = setTimeout(() => {
          at.current = 0
          setHint(false)
        }, ESC_AGAIN)
      }
    }
    window.addEventListener("keydown", onKey)
    return () => {
      window.removeEventListener("keydown", onKey)
      clearTimeout(timer.current)
    }
  }, [])
  return hint
}

export function Composer({ open }: { open: Open }) {
  const mode = useStore((s) => s.mode)
  const sendKey = usePrefs((p) => p.conversation.send)
  const asking = useStore((s) => s.permissions.find((p) => p.rootId === open.sessionId) ?? null)
  const key = draftKey(open)
  const draft = useStore((s) => s.drafts[key] ?? EMPTY_DRAFT)
  const text = draft.text
  // A `/` line the user didn't type here (recalled, put back) opens no menu and runs nothing
  // while its command word is the one put there.
  const inert = !!draft.head && commandHead(text)?.word === draft.head
  const files = draft.files
  const pasted = draft.pasted
  const setText = (t: string) => setDraft(key, { text: t })
  const [caret, setCaret] = useState(0)
  const [problem, setProblem] = useState<string | null>(null)
  const [dragging, setDragging] = useState(false)
  const [pal, setPal] = useState<Palette | null>(null)
  const [paths, setPaths] = useState<string[] | null>(null)
  /** The row the arrows are on; -1 before they move with nothing typed after `/`. */
  const [active, setActive] = useState(0)
  /** The trigger the user closed with Escape, until they type elsewhere. */
  const [dismissed, setDismissed] = useState<number | null>(null)
  /** How far back ArrowUp went through the sent messages; null when not recalling. */
  const [recall, setRecall] = useState<number | null>(null)
  const area = useRef<HTMLTextAreaElement>(null)
  /** The selection before the latest edit: a select-all replace is the user's own text. */
  const selected = useRef({ from: 0, to: 0 })
  const picker = useRef<HTMLInputElement>(null)
  const running = isRunning(open)
  const asideShown = useStore((s) => !!open.sessionId && !!s.asides[open.sessionId])
  const escHint = useDoubleEscape(running, open.sessionId, asideShown)
  // A stop that hasn't ended the turn after a few seconds may be pressed again.
  const stopStuck = usePast(open.stopping, STOP_AGAIN)
  const blocked = useBlocked(open)
  const worktree = useWorktree(open.sessionId)
  // Turns refuse until the worktree is back (or for good, once discarded).
  const closed =
    worktree?.state === "discarded"
      ? "This session’s worktree was discarded"
      : worktree?.state === "missing"
        ? "Recreate the worktree to send"
        : worktree?.state === "failed"
          ? "Try again, or work in the main folder, to send"
          : worktree?.state === "applying"
            ? "Sending waits for the apply"
            : null
  const model = open.info?.models.find((m) => m.id === open.info?.modelId)
  const comments = useStore((s) => s.comments)
  const empty = !text.trim() && files.length === 0 && comments.length === 0 && pasted.length === 0
  const planning = open.info?.agent === "plan"
  const hasPlan = open.info?.agents.some((a) => a.id === "plan") ?? false
  const aside = useStore((s) => (open.sessionId ? (s.asides[open.sessionId] ?? null) : null))
  /** `/memory`'s files once read (null: not yet; "failed": the line under the box says why), and whether a note is being added. */
  const [memory, setMemory] = useState<MemoryFiles | null | "failed">(null)
  /** Enter on `/memory` after a failed read reads again. */
  const [memoryTry, setMemoryTry] = useState(0)
  const [noting, setNoting] = useState(false)

  useEffect(() => area.current?.focus(), [open.sessionId])
  // Text put back by the store (a rewind, a message that didn't send): the caret goes to its end.
  useEffect(() => {
    if (!draft.stamp) return
    requestAnimationFrame(() => {
      const el = area.current
      if (!el) return
      el.focus()
      el.setSelectionRange(el.value.length, el.value.length)
      setCaret(el.value.length)
    })
  }, [draft.stamp])
  useEffect(() => {
    setRecall(null)
    setProblem(null)
  }, [key])
  // When the ask in front of Enter appeared, or the box got focus back with it waiting (from the
  // palette, a menu): an Enter held or meant for there must not allow it.
  const askedAt = useRef(0)
  useEffect(() => {
    askedAt.current = Date.now()
  }, [asking?.requestId])
  // Grows with the text, up to 240px; in a short window less (what the rows around it leave),
  // so Send stays on screen.
  const [tall, setTall] = useState(() => window.innerHeight)
  useEffect(() => {
    const onResize = () => setTall(window.innerHeight)
    window.addEventListener("resize", onResize)
    return () => window.removeEventListener("resize", onResize)
  }, [])
  useLayoutEffect(() => {
    const el = area.current
    if (!el) return
    el.style.height = "auto"
    el.style.height = `${Math.min(el.scrollHeight, 240, Math.max(80, tall - 440))}px`
  }, [text, tall])
  // An answer for the folder open before a switch doesn't land in this one's menus.
  const folder = useRef(open.directory)
  folder.current = open.directory
  // Commands and skills of this folder; read again when the menu opens (MCP prompts come late).
  const loadPalette = useCallback(() => {
    const directory = open.directory
    void readPalette(directory)
      .then((p) => folder.current === directory && setPal(p))
      .catch((e) => folder.current === directory && setProblem(`This folder’s commands couldn’t be read. ${e instanceof Error ? e.message : String(e)}`))
  }, [open.directory])
  useEffect(() => {
    setPal(null)
    setPaths(null)
    loadPalette()
  }, [loadPalette])

  const typed = inert ? null : trigger(text, caret)
  const note = inert ? null : MEMORY.exec(text.trimStart())
  const menuOpen = typed !== null ? dismissed !== typed.start : note !== null && dismissed !== MEMORY_MENU
  // `/memory`'s files: read as its menu opens, for the session (its worktree's file) or the folder.
  const memoryOpen = note !== null
  useEffect(() => {
    if (!memoryOpen) return
    const directory = open.directory
    let live = true
    setMemory(null)
    memoryFiles(open)
      .then((m) => live && folder.current === directory && setMemory(m))
      .catch((e) => {
        if (!live) return
        setMemory("failed")
        setProblem(e instanceof Error ? e.message : String(e))
      })
    return () => {
      live = false
    }
    // Once per opening of the menu, per session, and per try after a failure.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [memoryOpen, open.directory, open.sessionId, memoryTry])
  useEffect(() => {
    if (!typed) return
    if (typed.kind === "/" ) loadPalette()
    // Read on every open: the agent may have added files (the store keeps them 30 s).
    if (typed.kind === "@") {
      // A worktree session's files are its worktree's.
      const directory = workDir(open)
      void projectFiles(directory)
        .then((list) => folder.current === open.directory && setPaths(list))
        .catch(() => folder.current === open.directory && setPaths([]))
    }
    // Only when a menu opens.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [typed?.kind, typed?.start])

  /** Adds the note to `scope`'s file, or with no note opens that file in the editor. */
  const remember = (scope: MemoryScope) => {
    const files = memory
    if (!files || files === "failed" || noting) return
    const said = (note?.[1] ?? "").trim()
    if (!said) {
      clear()
      void openInEditor(open.directory, scope === "project" ? files.project : files.personal)
      return
    }
    setNoting(true)
    setProblem(null)
    addMemory(open, scope, said)
      .then(clear, (e) => setProblem(e instanceof Error ? e.message : String(e)))
      .finally(() => setNoting(false))
  }

  const rows = useMemo<Row[]>(() => {
    if (note && !typed && menuOpen) {
      if (!memory || memory === "failed") return []
      const tag = (note[1] ?? "").trim() ? "Add" : "Open"
      return [
        { key: "m:project", label: "Project", detail: tilde(memory.project), kind: "memory", insert: "", tag, run: () => remember("project") },
        { key: "m:personal", label: "Personal", detail: tilde(memory.personal), kind: "memory", insert: "", tag, run: () => remember("personal") },
      ]
    }
    if (!typed || !menuOpen) return []
    if (typed.kind === "/") {
      const server: Row[] = (pal?.commands ?? [])
        .filter((c) => !ACTION_NAMES.has(c.name))
        .map((c) => ({
          key: `c:${c.name}`,
          label: `/${c.name}`,
          detail: c.description,
          kind: "command",
          insert: `/${c.name} `,
        }))
      const skills: Row[] = (pal?.skills ?? []).map((s) => ({
        key: `s:${s.name}`,
        label: `/${s.name}`,
        detail: s.description,
        kind: "skill",
        insert: `@${s.name} `,
      }))
      // A brand-new session has nothing to undo, retry or compact: the folder's commands lead.
      const all = open.sessionId ? [...actions(open), ...server, ...skills] : [...server, ...actions(open), ...skills]
      return filter(all, typed.query, (r) => r.label.slice(1), 50)
    }
    const skills: Row[] = (pal?.skills ?? []).map((s) => ({
      key: `s:${s.name}`,
      label: `@${s.name}`,
      detail: s.description,
      kind: "skill",
      insert: `@${s.name} `,
    }))
    const found: Row[] = (paths ?? []).map((p) => ({
      key: `f:${p}`,
      label: p,
      detail: null,
      kind: p.endsWith("/") ? "folder" : "file",
      insert: p.endsWith("/") ? `@${p}` : `@${p} `,
    }))
    if (!typed.query) return [...skills, ...found.filter((f) => !f.label.slice(0, -1).includes("/"))].slice(0, 50)
    return [
      ...filter(skills, typed.query, (r) => r.label.slice(1), 6),
      ...filter(found, typed.query, (r) => r.label, 40),
    ]
    // `remember` reads the same state.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [typed?.kind, typed?.query, menuOpen, pal, paths, open, note?.[1], memory, noting])
  // The menu on screen: `/` shows only when something matches, `@` also to say nothing does,
  // `/memory` while its files are read.
  const menuShown = menuOpen && (rows.length > 0 || typed?.kind === "@" || (note !== null && !typed && memory !== "failed"))
  // A bare `/` has no row picked: Enter there must not run whatever comes first.
  // In a box that held text put there (recalled, put back), no row is picked either: Enter alone
  // picks nothing; the arrows, Tab or a click do.
  const fromBefore = !!draft.recalled
  useEffect(
    () => setActive(fromBefore || (typed?.kind === "/" && !typed.query) ? -1 : 0),
    [typed?.kind, typed?.query, note !== null, fromBefore]
  )

  const pick = (r: Row) => {
    // A note stays in the box until it is added (an error keeps it there).
    if (r.kind === "memory") return r.run?.()
    if (!typed) return
    if (r.run) {
      setText("")
      setCaret(0)
      r.run()
      return
    }
    const next = text.slice(0, typed.start) + r.insert + text.slice(caret)
    const at = typed.start + r.insert.length
    setText(next)
    setCaret(at)
    requestAnimationFrame(() => area.current?.setSelectionRange(at, at))
  }

  const add = async (list: Iterable<File>) => {
    setProblem(null)
    for (const file of list) {
      try {
        const image = await readImage(file)
        setDraft(key, (d) => ({ files: [...d.files, image] }))
      } catch (e) {
        setProblem(e instanceof Error ? e.message : String(e))
      }
    }
  }

  const clear = () => {
    setDraft(key, { text: "", files: [], pasted: [], recalled: false, head: undefined })
    setCaret(0)
    setProblem(null)
    setRecall(null)
  }

  /** What the user sent before in this session, newest first: ArrowUp walks back through it. */
  const history = useMemo(
    () =>
      open.items
        .filter((i): i is Extract<typeof i, { kind: "user" }> => i.kind === "user" && !i.command && !!i.text.trim())
        .map((i) => i.text)
        .reverse(),
    [open.items]
  )
  const recallTo = (n: number | null) => {
    setRecall(n)
    const t = n === null ? "" : history[n]
    setDraft(key, { text: t, recalled: n !== null, head: n !== null ? commandHead(t)?.word : undefined })
    requestAnimationFrame(() => {
      const el = area.current
      if (!el) return
      el.setSelectionRange(t.length, t.length)
      setCaret(t.length)
    })
  }

  const submit = () => {
    if (empty) return
    if (inert) {
      setProblem(RECALLED)
      return
    }
    if (blocked) {
      setProblem("Sending waits for your limit to reset.")
      return
    }
    // `/btw`: answered over the box, also while a turn runs; nothing joins the conversation.
    const btw = /^\/btw(?:\s+([\s\S]*))?$/.exec(text.trim())
    if (btw) {
      const question = [btw[1] ?? "", ...pasted.map((p) => p.text)].filter((t) => t.trim()).join("\n\n").trim()
      if (!open.sessionId) return setProblem("/btw asks about a session. Send a message first.")
      if (files.length > 0) return setProblem("A side question can’t take images.")
      if (!question) return setProblem("Type a question after /btw.")
      clear()
      void askAside(question)
      return
    }
    // `/memory` goes through its menu; Enter with the menu put away brings it back. A bare
    // `/memory` (no space yet) gets its space, which opens the menu.
    if (!note && /^\/memory$/.test(text.trim())) {
      setText("/memory ")
      setDismissed(null)
      return
    }
    if (note) {
      setDismissed(null)
      if (memory === "failed") {
        setProblem(null)
        setMemoryTry((n) => n + 1)
      }
      return
    }
    if (closed) {
      setProblem(`${closed}.`)
      return
    }
    const command = /^\/(\S+)(?:\s+([\s\S]*))?$/.exec(text.trim())
    if (command && files.length === 0 && pasted.length === 0) {
      const [, name, args = ""] = command
      const own = actions(open).find((a) => a.label === `/${name}`)
      if (own?.run) {
        clear()
        own.run()
        return
      }
      if (pal?.commands.some((c) => c.name === name)) {
        if (running) {
          setProblem(`/${name} can run once this turn ends, or after you stop it.`)
          return
        }
        clear()
        void runCommand(name, args.trim())
        return
      }
    }
    const full = [text, ...pasted.map((p) => p.text)].filter((t) => t.trim()).join("\n\n")
    // Whatever path got here (a dismissed menu, a pasted chip), `/memory` and `/btw` lines are
    // never sent to the model as a message.
    if (OWN_COMMAND.test(full.trimStart())) {
      setProblem(OWN_COMMAND_SAID)
      return
    }
    const skills = (pal?.skills ?? [])
      .map((s) => s.name)
      .filter((name) => new RegExp(`(^|\\s)@${name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}(?=\\s|$)`).test(full))
    void send(full, files, skills)
    clear()
  }

  const shown = open.info?.agent === "plan" ? "plan" : mode

  return (
    <div className="relative">
      {menuShown && (
        <Menu
          rows={rows}
          active={Math.min(active, rows.length - 1)}
          onPick={pick}
          onHover={setActive}
          empty={!typed ? "Finding AGENTS.md…" : paths === null ? "Reading files…" : `Nothing matches “${typed?.query}”`}
        />
      )}
      <div
        onDragOver={(e) => {
          if (e.dataTransfer.types.includes("Files")) {
            e.preventDefault()
            setDragging(true)
          }
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(e) => {
          e.preventDefault()
          setDragging(false)
          void add(e.dataTransfer.files)
        }}
        className={cn("rounded-[1.25rem] bg-surface p-2 transition-colors", dragging && "bg-default")}
      >
        {comments.length > 0 && (
          <div className="flex max-h-40 flex-col gap-1 overflow-y-auto px-1.5 pt-1 pb-2">
            {comments.map((c) => (
              <div key={c.id} className="text-dense flex items-center gap-2 rounded-lg bg-default/50 py-1 pr-1 pl-2.5">
                <MessageSquareText className="size-3.5 shrink-0 text-muted" />
                <span className="shrink-0 font-mono text-xs text-muted">
                  {c.file.split("/").pop()}:{c.line}
                </span>
                <span className="min-w-0 flex-1 truncate">{c.text}</span>
                <button
                  onClick={() => removeComment(c.id)}
                  aria-label="Remove comment"
                  className="flex size-6 items-center justify-center rounded-md text-muted hover:text-foreground"
                >
                  <X className="size-3" />
                </button>
              </div>
            ))}
          </div>
        )}
        {pasted.length > 0 && (
          <div className="flex flex-col gap-1 px-1.5 pt-1 pb-2">
            {pasted.map((p) => (
              <PastedChip
                key={p.id}
                text={p.text}
                onRemove={() => setDraft(key, (d) => ({ pasted: d.pasted.filter((x) => x.id !== p.id) }))}
              />
            ))}
          </div>
        )}
        {files.length > 0 && (
          <div className="flex flex-wrap gap-2 px-1.5 pt-1 pb-2">
            {files.map((f, i) => (
              <div key={i} className="group relative">
                <img src={`data:${f.mime};base64,${f.data}`} alt={f.name} className="size-14 rounded-lg object-cover" />
                <button
                  onClick={() => setDraft(key, (d) => ({ files: d.files.filter((_, j) => j !== i) }))}
                  aria-label={`Remove ${f.name}`}
                  className="absolute -top-1.5 -right-1.5 flex size-5 items-center justify-center rounded-full bg-popover text-muted opacity-0 group-hover:opacity-100 hover:text-foreground focus-visible:opacity-100"
                >
                  <X className="size-3" />
                </button>
              </div>
            ))}
          </div>
        )}
        <textarea
          ref={area}
          value={text}
          rows={2}
          onFocus={() => {
            askedAt.current = Date.now()
          }}
          onChange={(e) => {
            // The command word put there stays inert until an edit touches it (or the line is
            // typed again): an edit elsewhere in the box changes nothing about it.
            const next = e.target.value
            // Everything in the box replaced (select all, then type or paste): the user's own text.
            const replaced = text.length > 0 && selected.current.from === 0 && selected.current.to >= text.length
            const before = draft.head ? commandHead(text) : null
            const kept =
              !replaced &&
              !!before &&
              !touches(editedSpan(text, next), before.end - before.word.length, before.end) &&
              commandHead(next)?.word === draft.head
            setDraft(key, replaced ? { text: next, head: undefined, recalled: false } : { text: next, head: kept ? draft.head : undefined })
            if ((problem === RECALLED && !kept) || problem === PICK || problem === PICK_BARE) setProblem(null)
            setCaret(e.target.selectionStart)
            setDismissed(null)
            setRecall(null)
          }}
          onSelect={(e) => {
            setCaret(e.currentTarget.selectionStart)
            selected.current = { from: e.currentTarget.selectionStart, to: e.currentTarget.selectionEnd }
          }}
          onPaste={(e) => {
            // Excel and Word put a picture of the cells next to their text: the text is what was meant.
            const pastedText = e.clipboardData.getData("text/plain")
            const images = [...e.clipboardData.files].filter((f) => f.type.startsWith("image/"))
            if (images.length && !pastedText.trim()) {
              e.preventDefault()
              void add(images)
              return
            }
            if (pastedText.length > LONG_PASTE) {
              e.preventDefault()
              setDraft(key, (d) => ({ pasted: [...d.pasted, { id: Date.now(), text: pastedText }] }))
            }
          }}
          onKeyDown={(e) => {
            if (menuOpen && rows.length > 0) {
              if (e.key === "ArrowDown" || e.key === "ArrowUp") {
                e.preventDefault()
                const step = e.key === "ArrowDown" ? 1 : -1
                setActive((a) => (a < 0 ? (step > 0 ? 0 : rows.length - 1) : (a + step + rows.length) % rows.length))
                return
              }
              if (e.key === "Tab") {
                e.preventDefault()
                pick(rows[Math.max(0, Math.min(active, rows.length - 1))])
                return
              }
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault()
                if (active >= 0) pick(rows[Math.min(active, rows.length - 1)])
                // Never a silent Enter: say why nothing was picked.
                else setProblem(fromBefore ? PICK : PICK_BARE)
                return
              }
            }
            // Only a menu that shows takes Escape: an unmatched `/foo` has none to close.
            if (menuShown && e.key === "Escape" && (typed || note)) {
              e.preventDefault()
              setDismissed(typed ? typed.start : MEMORY_MENU)
              return
            }
            // Then the side question over the box, before the waiting ask and a stop.
            if (e.key === "Escape" && aside && open.sessionId && !anyLayer() && !e.repeat && !e.nativeEvent.isComposing) {
              e.preventDefault()
              closeAside(open.sessionId)
              return
            }
            // ArrowUp in an empty box (or on a recalled message) walks back through what was sent.
            const el = e.currentTarget
            const recalled = recall !== null && text === history[recall]
            if (e.key === "ArrowUp" && !e.shiftKey && history.length > 0 && (text === "" || recalled)) {
              e.preventDefault()
              recallTo(recall === null ? 0 : Math.min(recall + 1, history.length - 1))
              return
            }
            if (e.key === "ArrowDown" && recalled && el.selectionStart === el.value.length) {
              e.preventDefault()
              recallTo(recall === 0 ? null : recall - 1)
              return
            }
            if (isSendKey(e, sendKey)) {
              e.preventDefault()
              // A held Enter sends once and allows nothing: its repeats are not presses.
              if (e.repeat) return
              if (empty && asking) {
                if (Date.now() - askedAt.current >= ASK_SETTLE) void reply(asking, "once")
              } else submit()
            } else if (e.key === "Escape" && asking && !anyLayer() && !e.repeat && !e.nativeEvent.isComposing) {
              // Denies the waiting ask, in the box only; taken, so it is no press toward a stop.
              // Every other Escape goes on to the window, where the stop's presses are counted.
              e.preventDefault()
              void reply(asking, "reject")
            }
          }}
          disabled={worktree?.state === "discarded"}
          placeholder={
            worktree?.state === "discarded"
              ? "Read-only"
              : sendKey === "enter"
              ? asking
                ? "Reply, or press Enter to allow"
                : running
                  ? "Add to the queue, or Esc twice to stop"
                  : planning
                    ? "Describe what to plan"
                    : "Ask for a change, / for commands, @ for files"
              : asking
                ? "Reply, or press Ctrl Enter to allow"
                : running
                  ? "Add to the queue, Ctrl Enter to send"
                  : planning
                    ? "Describe what to plan, Ctrl Enter to send"
                    : "Ask for a change, / for commands, @ for files, Ctrl Enter to send"
          }
          aria-label="Message"
          className="text-md block min-h-14 w-full resize-none bg-transparent px-2.5 pt-1.5 leading-relaxed outline-none placeholder:text-field-placeholder"
        />
        {(problem || escHint || (files.length > 0 && model && !model.images)) && (
          <p className={cn("text-dense px-2.5 pb-2", escHint && !problem ? "text-muted" : "text-warning")}>
            {problem ?? (escHint ? "Press Esc again to stop" : `${model?.name} doesn’t read images.`)}
          </p>
        )}
        <div className="flex flex-wrap items-center gap-x-1.5 gap-y-1">
          <Tooltip label="Attach images" side="top">
            <button
              onClick={() => picker.current?.click()}
              className={btn("ghost", "sm", "text-muted", true)}
              aria-label="Attach images"
            >
              <Paperclip className="size-4" strokeWidth={1.75} />
            </button>
          </Tooltip>
          <input
            ref={picker}
            type="file"
            accept="image/*"
            multiple
            hidden
            onChange={(e) => {
              if (e.target.files) void add(e.target.files)
              e.target.value = ""
            }}
          />
          <div className="flex h-8 shrink-0 items-center rounded-full bg-default/60 p-0.5" role="radiogroup" aria-label="Mode">
            {modes
              .filter((m) => m.id !== "plan" || hasPlan)
              .map((m) => (
                <button
                  key={m.id}
                  role="radio"
                  aria-checked={shown === m.id}
                  title={m.hint}
                  onClick={() => pickMode(m.id)}
                  className={cn(
                    "text-dense h-7 rounded-full px-3 transition-colors focus-visible:status-focused-field",
                    shown === m.id ? "bg-default-hover text-foreground" : "text-muted hover:text-foreground"
                  )}
                >
                  {m.label}
                </button>
              ))}
          </div>
          {/* Narrow (the preview beside it): the model and the send button take a line of their own,
              where the model's name gives way if it must; the send button always stays whole. */}
          <div className="ml-auto flex max-w-full min-w-0 items-center justify-end gap-2">
            <PlaceChip open={open} />
            <ModelPicker open={open} />
            {running && empty ? (
              open.stopping && !stopStuck ? (
                <button disabled className={btn("secondary", "sm", "shrink-0 gap-2 px-3")} aria-label="Stopping">
                  <Spinner className="size-3" />
                  Stopping…
                </button>
              ) : (
                <Tooltip label={open.stopping ? "Still stopping. Stop again" : "Stop"} keys="Esc Esc" side="top">
                  <button onClick={stop} className={btn("secondary", "sm", "shrink-0", true)} aria-label="Stop">
                    <span className="size-2.5 rounded-[2px] bg-current" />
                  </button>
                </Tooltip>
              )
            ) : (
              <Tooltip label={blocked ? "Sending waits for your limit to reset" : closed ?? (running ? "Queue for after this turn" : "Send")} side="top">
                <button onClick={submit} disabled={empty || !!blocked || !!closed} className={btn("primary", "sm", "shrink-0", true)} aria-label="Send">
                  <ArrowUp className="size-4" />
                </button>
              </Tooltip>
            )}
          </div>
        </div>
      </div>
    </div>
  )
}

/** A long paste, folded: its size, and the text on demand. */
function PastedChip({ text, onRemove }: { text: string; onRemove: () => void }) {
  const [shown, setShown] = useState(false)
  const lines = text.split("\n").length
  return (
    <div className="rounded-lg bg-default/50">
      <div className="text-dense flex items-center gap-2 py-1 pr-1 pl-2.5">
        <ClipboardList className="size-3.5 shrink-0 text-muted" />
        <button
          onClick={() => setShown(!shown)}
          aria-expanded={shown}
          className="min-w-0 flex-1 truncate text-left hover:text-foreground focus-visible:status-focused-field"
        >
          Pasted text <span className="text-muted">· {lines === 1 ? `${text.length.toLocaleString("en-US")} characters` : `${lines.toLocaleString("en-US")} lines`}</span>
        </button>
        {shown ? <ChevronUp className="size-3.5 text-muted" /> : <ChevronDown className="size-3.5 text-muted" />}
        <button
          onClick={onRemove}
          aria-label="Remove pasted text"
          className="flex size-6 items-center justify-center rounded-md text-muted hover:text-foreground"
        >
          <X className="size-3" />
        </button>
      </div>
      {shown && (
        <pre className="mx-1 mb-1 max-h-40 overflow-auto rounded-md bg-background/60 px-2.5 py-2 font-mono text-xs leading-relaxed whitespace-pre-wrap text-foreground/75">
          {text}
        </pre>
      )}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

const efforts: Record<string, string> = { xhigh: "Extra high" }

export function capitalize(s: string) {
  return s.charAt(0).toUpperCase() + s.slice(1)
}

function ModelPicker({ open }: { open: Open }) {
  const [show, setShow] = useState(false)
  const close = useCallback(() => setShow(false), [])
  // "Choose another model" from a banner opens this list.
  const asked = useStore((s) => s.pickModel)
  const seen = useRef(asked)
  useEffect(() => {
    if (asked === seen.current) return
    seen.current = asked
    setShow(true)
  }, [asked])
  const info = open.info
  if (!info?.model) return null
  const current = info.models.find((m) => m.id === info.modelId)
  return (
    <div className="relative min-w-0">
      <button
        onClick={() => setShow(!show)}
        aria-expanded={show}
        title={info.variant ? `${info.model} · ${efforts[info.variant] ?? capitalize(info.variant)}` : info.model}
        className="text-dense flex h-8 w-full max-w-56 min-w-0 items-center gap-1.5 rounded-full px-3 text-muted hover:bg-default/60 hover:text-foreground focus-visible:status-focused-field"
      >
        <span className="min-w-0 truncate">{info.model}</span>
        {info.variant && <span className="min-w-0 shrink-[4] truncate text-muted/70">· {efforts[info.variant] ?? capitalize(info.variant)}</span>}
        <ChevronDown className="size-3.5 shrink-0" />
      </button>
      <Popover open={show} onClose={close} side="top" align="end" className="w-72">
        <div role="listbox" aria-label="Model" className="flex max-h-[min(20rem,50vh)] flex-col overflow-y-auto">
          {info.models.map((m) => (
            <button
              key={m.id}
              role="option"
              aria-selected={m.id === info.modelId}
              onClick={() => {
                // The effort carries over when the new model has it.
                const variant = info.variant && m.efforts.includes(info.variant) ? info.variant : null
                void chooseModel(m.id, variant)
              }}
              className="flex items-start gap-2.5 rounded-lg px-2.5 py-2 text-left hover:bg-default-hover focus-visible:status-focused-field"
            >
              <span className="min-w-0 flex-1">
                <span className="block text-sm">{m.name}</span>
                {m.note && <span className="text-2xs block text-muted">{m.note}</span>}
              </span>
              {m.id === info.modelId && <Check className="mt-0.5 size-3.5 shrink-0" />}
            </button>
          ))}
        </div>
        {current && current.efforts.length > 0 && (
          <div className="mt-1 border-t border-separator px-2.5 pt-2.5 pb-1.5">
            <div className="text-2xs pb-2 text-muted">Effort</div>
            <div className="flex flex-wrap gap-1" role="radiogroup" aria-label="Effort">
              {[null, ...current.efforts].map((e) => (
                <button
                  key={e ?? "default"}
                  role="radio"
                  aria-checked={info.variant === e}
                  onClick={() => void chooseModel(current.id, e)}
                  className={cn(
                    "text-2xs h-7 rounded-full px-2.5 transition-colors focus-visible:status-focused-field",
                    info.variant === e ? "bg-default-hover text-foreground" : "text-muted hover:bg-default/60"
                  )}
                >
                  {e ? (efforts[e] ?? capitalize(e)) : "Default"}
                </button>
              ))}
            </div>
            <p className="text-2xs pt-2 text-muted">Higher thinks longer and uses more of your limit.</p>
          </div>
        )}
      </Popover>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Tasks
// ---------------------------------------------------------------------------

/** The session's todo list, folded to the task under way; gone once every task is done. */
export function Tasks({ todos }: { todos: Todo[] }) {
  const [open, setOpen] = useState(false)
  const done = todos.filter((t) => t.status === "completed").length
  const left = todos.filter((t) => t.status !== "completed" && t.status !== "cancelled")
  if (left.length === 0) return null
  const current = todos.find((t) => t.status === "in_progress") ?? left[0]
  return (
    <div className="mb-2 rounded-xl bg-surface/70 px-1.5 py-1">
      <button
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className="text-dense flex h-8 w-full items-center gap-2.5 rounded-lg px-2 text-left hover:bg-default/40 focus-visible:status-focused-field"
      >
        {open ? (
          <span className="min-w-0 flex-1 text-muted">Tasks</span>
        ) : (
          <>
            <TaskIcon status={current.status} />
            <span className="min-w-0 flex-1 truncate">{current.content}</span>
          </>
        )}
        <span className="text-2xs shrink-0 text-muted tabular-nums">
          {done} of {todos.length}
        </span>
        {open ? <ChevronDown className="size-3.5 text-muted" /> : <ChevronUp className="size-3.5 text-muted" />}
      </button>
      {open && (
        <ul className="flex max-h-60 flex-col overflow-y-auto pb-1">
          {todos.map((t, i) => (
            <li key={i} className="text-dense flex items-start gap-2.5 px-2 py-1">
              <TaskIcon status={t.status} />
              <span
                className={cn(
                  "min-w-0 flex-1",
                  t.status === "completed" && "text-muted line-through decoration-muted/60",
                  t.status === "cancelled" && "text-muted"
                )}
              >
                {t.content}
              </span>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}

function TaskIcon({ status }: { status: string }) {
  if (status === "completed") return <CircleCheck className="mt-px size-3.5 shrink-0 text-muted" />
  if (status === "in_progress") return <LoaderCircle className="mt-px size-3.5 shrink-0 animate-spin text-foreground/80 [animation-duration:2s]" />
  return <Circle className="mt-px size-3.5 shrink-0 text-muted/70" />
}
