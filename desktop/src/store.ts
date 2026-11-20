/**
 * Everything the window shows, kept in one place and fed by the bridge: calls the user makes
 * and the core's events. The open transcript is re-read from the database whenever an event
 * says it changed; only streaming text lives here between reads.
 */
import { useSyncExternalStore } from "react"

import {
  getBridge,
  type Account,
  type Attachment,
  type BackgroundCommand,
  type Bridge,
  type Context,
  type CoreEvent,
  type ErrorKind,
  type FileDiff,
  type FormAnswer,
  type FormRequest,
  type GitStatus,
  type Hooks,
  type TrustItem,
  type TrustKind,
  type Info,
  type Item,
  type Mode,
  type Palette,
  type PendingRequest,
  type PermissionReply,
  type Queued,
  type Question,
  type Revert,
  type SessionRow,
  type Todo,
  type ToolInput,
  type ToolOutput,
  type UpdateCheck,
  type View,
} from "./bridge"
import { getPrefs, loadPrefs, onPrefsFailure, setPref } from "./prefs"
import { folderKey, onBeforeClose, runBeforeClose, sameFolder } from "./ui"

export type { Mode }

/** `sessionId` is where the ask came from (a subagent's session too); `rootId` where it shows. */
export type Asked = {
  requestId: string
  sessionId: string
  rootId: string
  directory: string
  questions: Question[]
}

export type Permission = {
  requestId: string
  sessionId: string
  rootId: string
  directory: string
  action: string
  resources: string[]
  canAlways: boolean
  files: FileDiff[] | null
}

/** What the open session is doing, under its last item. */
export type Status =
  | { kind: "idle" }
  | { kind: "working" }
  | { kind: "retrying"; attempt: number; until: number; error: string }
  /** `retry`: the latest turn failed, so sending its message again makes sense. `errorKind` picks
   * the words and the way out (a limit, no connection, a lost sign-in…). */
  | {
      kind: "failed"
      error: string
      retry?: boolean
      errorKind?: ErrorKind
      /** The user's hook stopped a message; `back`: it is in the message box again. */
      hook?: boolean
      back?: boolean
    }
  | { kind: "stopped"; reason: "user" | "budget" | "declined" }

/** A long paste, kept folded in the composer and sent as part of the message. */
export type Pasted = { id: number; text: string }

/**
 * What the composer holds for one session (or for a new one in a folder): it stays there when
 * the user goes elsewhere. `stamp` changes when the store puts text back (a rewind, a message
 * that didn't send), so the composer moves the caret to its end.
 */
export type Draft = { text: string; files: Attachment[]; comments: Comment[]; pasted: Pasted[]; stamp: number }

export const EMPTY_DRAFT: Draft = { text: "", files: [], comments: [], pasted: [], stamp: 0 }

const DRAFTS_KEY = "anysola.drafts"

/** A new session's draft that nobody came back to goes after this long. */
const DRAFT_DAYS = 21

/** When each draft last changed (read before `state` is made, so declared here). */
const draftAt = new Map<string, number>()

export type Open = {
  /** This view of the folder or session: a new session's requests answer to it alone. */
  key: string
  directory: string
  /** Null until the first prompt of a new session creates it. */
  sessionId: string | null
  title: string
  items: Item[]
  /** Streaming text not yet in the database, by `messageId:ordinal`. */
  live: { key: string; kind: "text" | "reasoning"; text: string }[]
  info: Info | null
  status: Status
  loading: boolean
  /** A staged rewind, until the next prompt makes it final or Redo puts it back. */
  revert: Revert | null
  todos: Todo[]
  /** The agent that wrote the last reply shown. */
  lastAgent: string | null
  /** Messages waiting for the running turn (or, after a stop, for the user to send them). */
  queued: Queued[]
  context: Context | null
  background: BackgroundCommand[]
  /** When Stop was pressed (ms); the turn hasn't ended yet. False when not stopping. */
  stopping: false | number
  /** When the running turn started (ms), for the elapsed time; null when unknown. */
  started: number | null
}

/** A remark on a diff line, sent with the next message. */
export type Comment = {
  id: number
  file: string
  line: number
  /** A removed line counts in the old file. */
  removed: boolean
  code: string
  text: string
}

export type SettingsSection =
  | "account"
  | "usage"
  | "billing"
  | "general"
  | "appearance"
  | "conversation"
  | "notifications"
  | "keyboard"
  | "permissions"
  | "hooks"
  | "mcp"
  | "skills"
  | "commands"
  | "instructions"
  | "advanced"
  | "discord"
  | "data"
  | "about"

/** A subagent's transcript, read alongside the open session. */
export type Peek = { directory: string; sessionId: string; title: string; items: Item[]; loading: boolean }

export type State = {
  auth: "checking" | "signed-out" | "signed-in"
  login: { url: string; code: string } | null
  loginError: string | null
  sessions: SessionRow[]
  busy: Set<string>
  permissions: Permission[]
  questions: Asked[]
  account: Account | null
  /** `waiting`: install was asked for while turns run; it starts when they end. `progress`: the
   * download so far. */
  update:
    | (UpdateCheck & {
        installing?: boolean
        error?: string
        waiting?: boolean
        progress?: { downloaded: number; total: number | null }
      })
    | null
  /** MCP forms waiting for an answer; each shows in the open session of its folder. */
  forms: FormRequest[]
  /** What the composers hold, by `draftKey`. */
  drafts: Record<string, Draft>
  /** The service couldn't be reached on the last try; cleared by the next request that works. */
  offline: boolean
  /** The window was closed while this many turns ran: the window asks before quitting. */
  closing: number | null
  /** Counts requests to open the model picker ("Choose another model"). */
  pickModel: number
  /** The app's own data can't be opened: signing in won't help, the folder needs a look. */
  fault: { message: string; folder: string } | null
  open: Open | null
  peek: Peek | null
  mode: Mode
  notice: string | null
  /** A notice that has a place in Settings to fix it (an MCP server's failure). */
  noticeLink: SettingsSection | null
  /** Settings, open over the window; the session stays as it was under it. */
  settings: SettingsSection | null
  /** Counts the core's `mcp` events: Settings reads the servers again when it changes. */
  mcpChanged: number
  /** Counts the core's `hooks` events: the folder's hooks are read again when it changes. */
  hooksChanged: number
  /** Each folder's hooks as last read (by `folderKey`), shared by Settings and the conversation. */
  hooks: Record<string, Hooks>
  /** Counts the core's `trust` events and the answers given here: what waits is read again. */
  trustChanged: number
  /** What each folder's project files want to run (by `folderKey`), as last read. */
  trust: Record<string, TrustItem[]>
  /** Sessions kept at the top of the sidebar, and project groups folded shut (by `folderKey`). */
  pinned: Set<string>
  collapsed: Set<string>
  /** The open session's folder in git, once read. */
  git: (GitStatus & { directory: string }) | null
  /** The last read of a folder's git state failed: the git menu says so, with a retry. */
  gitError: { directory: string; message: string } | null
  /** Asks (permissions, questions, forms) whose answer is on its way: their cards wait, disabled. */
  sending: Set<string>
  /** The open view's diff comments, waiting for its next message (kept in its draft). */
  comments: Comment[]
  /** The app's folder for sessions without a project, once known. */
  scratch: string | null
  /** Tool inputs the model is writing, by call id, for their `streaming` rows (any session). */
  inputs: Record<string, ToolInput>
  /** Running commands' output so far, by call id (any session). */
  outputs: Record<string, ToolOutput>
}

function readMode(): Mode {
  try {
    const m = localStorage.getItem("anysola.mode")
    if (m === "ask" || m === "edits" || m === "auto") return m
  } catch {
    // Storage can be unavailable; the default stands.
  }
  return "edits"
}

function readSet(key: string): Set<string> {
  try {
    const raw = JSON.parse(localStorage.getItem(key) ?? "[]")
    return new Set(Array.isArray(raw) ? raw.filter((x) => typeof x === "string") : [])
  } catch {
    return new Set()
  }
}

function saveSet(key: string, value: Set<string>) {
  try {
    localStorage.setItem(key, JSON.stringify([...value]))
  } catch {
    // Kept for this window only.
  }
}

const SECTIONS = new Set<string>([
  "account", "usage", "billing", "general", "appearance", "conversation", "notifications", "keyboard",
  "permissions", "hooks", "mcp", "skills", "commands", "instructions", "advanced", "discord", "data", "about",
])

/** `?settings=<section>` outside Tauri, for screenshots. */
function readSettingsParam(): SettingsSection | null {
  if (typeof location === "undefined") return null
  const s = new URLSearchParams(location.search).get("settings")
  return s && SECTIONS.has(s) ? (s as SettingsSection) : null
}

let state: State = {
  auth: "checking",
  login: null,
  loginError: null,
  sessions: [],
  busy: new Set(),
  permissions: [],
  questions: [],
  account: null,
  update: null,
  open: null,
  peek: null,
  mode: readMode(),
  notice: null,
  noticeLink: null,
  settings: readSettingsParam(),
  mcpChanged: 0,
  hooksChanged: 0,
  hooks: {},
  trustChanged: 0,
  trust: {},
  scratch: null,
  inputs: {},
  outputs: {},
  pinned: readSet("anysola.pinned"),
  // Folded projects by folder key; older entries were stored as spelled.
  collapsed: new Set([...readSet("anysola.collapsed")].map(folderKey)),
  git: null,
  gitError: null,
  sending: new Set(),
  comments: [],
  forms: [],
  drafts: readDrafts(),
  offline: false,
  closing: null,
  pickModel: 0,
  fault: null,
}

const listeners = new Set<() => void>()

const NO_COMMENTS: Comment[] = []

function set(patch: Partial<State> | ((s: State) => Partial<State>)) {
  const p = typeof patch === "function" ? patch(state) : patch
  // A new notice drops the last one's link unless it brings its own.
  state = { ...state, ...("notice" in p && !("noticeLink" in p) ? { noticeLink: null } : {}), ...p }
  // The comments shown are the open view's draft's: they follow it in and out of sessions.
  const comments = (state.open && state.drafts[draftKey(state.open)]?.comments) || NO_COMMENTS
  if (comments !== state.comments) state = { ...state, comments }
  if ("drafts" in p) saveDrafts()
  listeners.forEach((l) => l())
}

// ---------------------------------------------------------------------------
// Drafts
// ---------------------------------------------------------------------------

/** A session's draft goes by its id; a new session's by its folder, so it waits there. */
export function draftKey(open: { sessionId: string | null; directory: string }) {
  return open.sessionId ?? `new:${folderKey(open.directory)}`
}

/** Only the text outlives the window: images can be megabytes, comments point at a diff. Stored
 * as `{ text, at }`; older versions stored the text alone. */
function readDrafts(): Record<string, Draft> {
  try {
    const raw = JSON.parse(localStorage.getItem(DRAFTS_KEY) ?? "{}") as Record<string, unknown>
    const out: Record<string, Draft> = {}
    const now = Date.now()
    for (const [k, v] of Object.entries(raw)) {
      const o = v && typeof v === "object" ? (v as { text?: unknown; at?: unknown }) : null
      const text = typeof v === "string" ? v : typeof o?.text === "string" ? o.text : ""
      const at = typeof o?.at === "number" ? o.at : now
      if (!text || (k.startsWith("new:") && now - at > DRAFT_DAYS * 86_400_000)) continue
      out[k] = { ...EMPTY_DRAFT, text }
      draftAt.set(k, at)
    }
    return out
  } catch {
    return {}
  }
}

let saveTimer: ReturnType<typeof setTimeout> | undefined

/** Writes the drafts now. */
function writeDrafts() {
  clearTimeout(saveTimer)
  saveTimer = undefined
  const texts: Record<string, { text: string; at: number }> = {}
  for (const [k, d] of Object.entries(state.drafts)) {
    // A folded paste is part of what was typed.
    const text = [d.text, ...d.pasted.map((p) => p.text)].filter((t) => t.trim()).join("\n\n")
    if (text) texts[k] = { text, at: draftAt.get(k) ?? Date.now() }
  }
  try {
    localStorage.setItem(DRAFTS_KEY, JSON.stringify(texts))
  } catch {
    // Kept for this window only.
  }
}

function saveDrafts() {
  clearTimeout(saveTimer)
  saveTimer = setTimeout(writeDrafts, 300)
}

// A window that goes without warning (Alt F4, a crash, the updater's restart) gives no time for
// the debounce: what is typed is written the moment the window is hidden or left.
if (typeof window !== "undefined") {
  window.addEventListener("pagehide", writeDrafts)
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") writeDrafts()
  })
  onBeforeClose(writeDrafts)
}

/** Drafts of sessions that are gone (deleted here or elsewhere) go with them. */
function pruneDrafts(sessions: SessionRow[]) {
  // A session made a moment ago may not be in a list read just before.
  if (creating > 0) return
  const ids = new Set(sessions.map((r) => r.id))
  const keep = state.open ? draftKey(state.open) : null
  const gone = Object.keys(state.drafts).filter((k) => !k.startsWith("new:") && !ids.has(k) && k !== keep && !unsent.has(k))
  if (gone.length === 0) return
  set((s) => {
    const drafts = { ...s.drafts }
    for (const k of gone) delete drafts[k]
    return { drafts }
  })
}

/** Changes one draft; one left empty is dropped. */
export function setDraft(key: string, patch: Partial<Draft> | ((d: Draft) => Partial<Draft>)) {
  set((s) => {
    const d = s.drafts[key] ?? EMPTY_DRAFT
    const next = { ...d, ...(typeof patch === "function" ? patch(d) : patch) }
    const drafts = { ...s.drafts }
    const blank = !next.text && !next.files.length && !next.comments.length && !next.pasted.length
    if (blank) delete drafts[key]
    else drafts[key] = next
    draftAt.set(key, Date.now())
    return { drafts }
  })
}

let stamps = 0

/** Puts text back into a draft (with its images and comments), after what is there already. */
function restoreDraft(key: string, back: { text: string; files?: Attachment[]; comments?: Comment[] }) {
  setDraft(key, (d) => ({
    text: d.text.trim() && back.text.trim() ? `${back.text}\n\n${d.text}` : back.text || d.text,
    files: [...(back.files ?? []), ...d.files],
    comments: [...(back.comments ?? []), ...d.comments.filter((c) => !back.comments?.some((b) => b.id === c.id))],
    stamp: ++stamps,
  }))
}

/** A draft that moves with its view: a new session's, once the session exists. */
function moveDraft(from: string, to: string) {
  if (from === to || !state.drafts[from]) return
  set((s) => {
    const drafts = { ...s.drafts }
    const d = drafts[from]
    delete drafts[from]
    const into = drafts[to]
    drafts[to] = into ? { ...into, text: [into.text, d.text].filter(Boolean).join("\n\n"), files: [...into.files, ...d.files], comments: [...into.comments, ...d.comments], pasted: [...into.pasted, ...d.pasted] } : d
    draftAt.set(to, Date.now())
    return { drafts }
  })
}

/** Updates the open view; with a target, only while that session (or that view) is open. */
function setOpen(patch: Partial<Open> | ((o: Open) => Partial<Open>), target?: string | { key: string }) {
  set((s) => {
    if (!s.open) return {}
    if (typeof target === "string" && s.open.sessionId !== target) return {}
    if (typeof target === "object" && s.open.key !== target.key) return {}
    return { open: { ...s.open, ...(typeof patch === "function" ? patch(s.open) : patch) } }
  })
}

export function useStore<T>(select: (s: State) => T): T {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l)
      return () => listeners.delete(l)
    },
    () => select(state)
  )
}

let bridge: Bridge

let views = 0

/** Where an answer for this view goes: its session wherever it is shown, a new one's own view. */
function whose(open: Open): string | { key: string } {
  return open.sessionId ?? { key: open.key }
}

function same(a: string | { key: string }, b: string | { key: string }) {
  return typeof a === "string" || typeof b === "string" ? a === b : a.key === b.key
}

function showing(target: string | { key: string }) {
  const o = state.open
  return !!o && (typeof target === "string" ? o.sessionId === target : o.key === target.key)
}

/** A request's failure: on its own view while shown, else in the notice, never on another. */
function failed(target: string | { key: string }, e: unknown) {
  // Its message went back to the composer: there is nothing to try again.
  if (showing(target)) setOpen({ status: { kind: "failed", error: message(e), retry: false } }, target)
  else set({ notice: message(e) })
}

function message(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : "That didn’t work."
}

// ---------------------------------------------------------------------------
// Start, sign-in
// ---------------------------------------------------------------------------

let started = false

/** How often a running window looks for a new version. */
const UPDATE_EVERY = 6 * 3_600_000

async function checkUpdate() {
  if (state.update?.installing || !getPrefs().general.checkUpdates) return
  try {
    set({ update: await bridge.updateCheck() })
  } catch {
    // Offline or no release yet: the next check tries again.
  }
}

/** "Check now" in Settings: the answer, or what went wrong. */
export async function checkUpdateNow(): Promise<UpdateCheck | string> {
  try {
    const update = await bridge.updateCheck()
    if (!state.update?.installing) set({ update })
    return update
  } catch (e) {
    return message(e)
  }
}

/** Installs and restarts; while turns run it waits for them (`now` skips the wait). */
export async function installUpdate(now = false) {
  const update = state.update
  if (!update?.available) return
  if (update.manual) {
    const page = update.manual
    void bridge.openUrl(page).catch(() => {
      if (state.update) set({ update: { ...state.update, error: `Your browser didn’t open. The download is at ${page}` } })
    })
    return
  }
  // A running turn would be cut off by the restart: the install waits for the last one to end,
  // unless the user says to go ahead now.
  if (state.busy.size > 0 && !now) {
    set({ update: { ...update, waiting: true, error: undefined } })
    return
  }
  set({ update: { ...update, installing: true, waiting: false, error: undefined } })
  try {
    // Restarts into the new version when it is done.
    await bridge.updateInstall()
  } catch (e) {
    set({ update: { ...update, installing: false, error: message(e) } })
  }
}

export async function start() {
  if (started) return
  started = true
  bridge = await getBridge()
  bridge.onEvent(onEvent)
  onPrefsFailure((notice) => set({ notice, noticeLink: null }))
  // The update check waits for the preference that may turn it off; the first session too, so
  // its file changes and command output open as the preferences say.
  const prefsRead = loadPrefs(bridge)
  void prefsRead.finally(() => void checkUpdate())
  setInterval(() => void checkUpdate(), UPDATE_EVERY)
  listeners.add(watchPresence)
  try {
    const { signedIn, fault } = await bridge.authStatus()
    if (fault) {
      // A broken or locked database: the sign-in screen would only send the user in circles.
      set({ auth: "signed-out", fault })
      return
    }
    set({ auth: signedIn ? "signed-in" : "signed-out" })
    // Names the app's folder "No project" in the sidebar; without it the folder shows its path.
    void bridge
      .scratchFolder()
      .then((scratch) => set({ scratch }))
      .catch(() => {})
    if (signedIn) {
      void loadAccount()
      await prefsRead
      await loadSessions(true)
    }
  } catch (e) {
    set({ auth: "signed-out", loginError: message(e) })
  }
}

export async function signIn() {
  set({ loginError: null })
  try {
    const login = await bridge.loginStart()
    set({ login })
    void openLogin(login.url)
    const { signedIn } = await bridge.loginWait()
    // Cancelled, or a newer sign-in took this one's place.
    if (!signedIn) return
    set({ auth: "signed-in", login: null, loginError: null })
    void loadAccount()
    await loadSessions(true)
  } catch (e) {
    set({ login: null, loginError: message(e) })
  }
}

export function cancelSignIn() {
  set({ login: null, loginError: null })
  void bridge.loginCancel().catch(() => {})
}

export function openLoginPage() {
  if (state.login) void openLogin(state.login.url)
}

/** Opens the sign-in page. A browser that won't open says so: the address stays on screen to copy. */
async function openLogin(url: string) {
  try {
    await bridge.openUrl(url)
    if (state.login) set({ loginError: null })
  } catch {
    if (state.login) set({ loginError: "Your browser didn’t open. Copy the link and open it there." })
  }
}

export async function loadAccount() {
  try {
    const account = await bridge.account()
    if (account.expired) {
      set({
        auth: "signed-out",
        account: null,
        loginError: account.rejected ?? "Your sign-in expired. Sign in again.",
      })
      return
    }
    set({ account, offline: false })
  } catch {
    // The plan and windows stay as last read; the banner says why they may be old.
    set({ offline: true })
  }
}

/** The offline banner's Retry: reads the account again, and sends again a message that failed for it. */
export async function retryConnection() {
  await loadAccount()
  const o = state.open
  if (!state.offline && o?.status.kind === "failed" && o.status.errorKind === "offline" && o.status.retry) await retry()
}

/** Shows the folder whose data can't be opened. */
export function revealFault() {
  const fault = state.fault
  if (fault) void bridge.revealPath(fault.folder).catch((e) => set({ notice: message(e) }))
}

/** "Choose another model": the composer's picker opens. */
export function openModelPicker() {
  set((s) => ({ pickModel: s.pickModel + 1 }))
}

export async function signOut() {
  try {
    const { signedIn } = await bridge.logout()
    if (signedIn) {
      set({
        notice:
          "This computer signs in with a token set outside the app, so signing out here can’t end it. To sign out, remove the ANYSOLA_TOKEN environment variable, then open the app again.",
      })
      return
    }
    // What was typed belongs to the account that typed it.
    unsent.clear()
    draftAt.clear()
    set({
      auth: "signed-out",
      account: null,
      sessions: [],
      open: null,
      permissions: [],
      questions: [],
      forms: [],
      settings: null,
      drafts: {},
    })
    writeDrafts()
  } catch (e) {
    set({ notice: message(e) })
  }
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/** The list failed once and said so; it says so again only after a read that worked. */
let listFailed = false

/** Reads the session list; a failure keeps the list as it was and says so once. */
async function loadSessions(openFirst = false) {
  let sessions: SessionRow[]
  try {
    sessions = await bridge.sessions()
    listFailed = false
  } catch (e) {
    if (!listFailed) set({ notice: `Your sessions couldn’t be read. ${message(e)}` })
    listFailed = true
    return
  }
  set({ sessions })
  pruneDrafts(sessions)
  if (openFirst && !state.open && sessions[0]) await openSession(sessions[0])
}

function blankView(directory: string, sessionId: string | null, title: string): Open {
  return {
    key: `v${++views}`,
    directory,
    sessionId,
    title,
    items: [],
    live: [],
    info: null,
    status: { kind: "idle" },
    loading: sessionId !== null,
    revert: null,
    todos: [],
    lastAgent: null,
    queued: [],
    context: null,
    background: [],
    stopping: false,
    started: null,
  }
}

/** The mode a session runs under follows the composer's; if that fails, the user must know:
 * the turn would go on asking (or not asking) as before. */
function applyMode(directory: string, sessionId: string, mode: Mode) {
  void bridge
    .setMode(directory, sessionId, mode)
    .catch((e) => set({ notice: `The ${mode === "ask" ? "Ask" : mode === "edits" ? "Edits" : "Auto"} mode didn’t apply to this session. ${message(e)}` }))
}

/** When a running turn started, as far as the transcript tells: its message's time. */
function turnStart(items: Item[]): number | null {
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i]
    if (it.kind === "user") return it.time ?? null
  }
  return null
}

export async function openSession(row: SessionRow) {
  void refreshGit(row.directory)
  set({ open: blankView(row.directory, row.id, row.title) })
  const n = ++reads
  const since = asks
  try {
    const opened = await bridge.openSession(row.directory, row.id)
    // The composer's mode is the window's, the floor the session's: a turn still running here
    // would otherwise go on under the mode it started with while the composer shows another.
    applyMode(row.directory, row.id, state.mode)
    set((s) => ({ busy: toggle(s.busy, row.id, opened.busy) }))
    // What waited for an answer when the window last looked (or before it started) shows again.
    applyPending(row.directory, row.id, opened.pending ?? [], since)
    setOpen(
      (o) => ({
        ...(latest(n) ? viewPatch(opened.view, o) : {}),
        info: opened.info,
        title: opened.session.title,
        loading: false,
        status: o.status.kind === "idle" && opened.busy ? { kind: "working" } : o.status,
        started: opened.busy ? (o.started ?? turnStart(opened.view.items)) : null,
      }),
      row.id
    )
  } catch (e) {
    setOpen({ loading: false, status: { kind: "failed", error: message(e) } }, row.id)
    // A session deleted elsewhere ("This session no longer exists."): its row goes too.
    void loadSessions()
  }
}

/** A new session: in `directory`, else the default folder, else one the user picks. */
export async function newSession(directory?: string) {
  if (!directory) directory = getPrefs().general.defaultFolder ?? (await pickFolder()) ?? undefined
  if (!directory) return
  // A new session starts in the mode Settings names; "last" keeps the composer's.
  const startMode = getPrefs().general.startMode
  if (startMode !== "last" && startMode !== state.mode) set({ mode: startMode })
  void refreshGit(directory)
  const open = blankView(directory, null, "New session")
  const view = { key: open.key }
  set({ open })
  try {
    const info = await bridge.folderInfo(directory)
    setOpen({ info }, view)
  } catch (e) {
    setOpen({ status: { kind: "failed", error: message(e) } }, view)
  }
}

/** Opens the folder picker where the default folder is. */
export async function pickFolder() {
  return bridge.pickFolder(getPrefs().general.defaultFolder)
}

/** A session with no project: it runs in the app's own folder. */
export async function newScratchSession() {
  try {
    const scratch = state.scratch ?? (await bridge.scratchFolder())
    set({ scratch })
    await newSession(scratch)
  } catch (e) {
    set({ notice: message(e) })
  }
}

export function setDefaultFolder(directory: string | null) {
  void setPref("general", { defaultFolder: directory })
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/** What had focus before Settings opened (the composer, mostly): it gets it back on return, so
 * Enter and Escape answer a waiting ask again without a click. */
let returnFocus: HTMLElement | null = null

export function openSettings(section: SettingsSection = "general") {
  if (state.auth !== "signed-in") return
  if (!state.settings) {
    const active = document.activeElement
    returnFocus = active instanceof HTMLElement && !active.closest("[data-settings]") ? active : null
  }
  set({ settings: section })
}

export function closeSettings() {
  if (!state.settings) return
  set({ settings: null })
  const back = returnFocus
  returnFocus = null
  // After React drops `inert` from the window behind.
  requestAnimationFrame(() => {
    if (back?.isConnected) back.focus()
    else document.querySelector<HTMLTextAreaElement>('textarea[aria-label="Message"]')?.focus()
  })
}

export function setAccount(account: Account) {
  if (!account.expired) set({ account })
}

/** For Settings: the bridge once started. */
export function currentBridge(): Bridge {
  return bridge
}

/** The folder the agent sections read: the open session's, else the app's own. */
export async function settingsFolder(): Promise<string> {
  if (state.open) return state.open.directory
  const scratch = state.scratch ?? (await bridge.scratchFolder())
  if (!state.scratch) set({ scratch })
  return scratch
}

/** The session list read again (Settings deleted sessions); an open one that went is closed. */
export async function reloadSessions() {
  try {
    await loadSessions()
    const open = state.open
    if (open?.sessionId && !state.sessions.some((r) => r.id === open.sessionId)) set({ open: null })
  } catch {
    // The next event reads it again.
  }
}

/** The sign-in screen, for a token Settings found expired. */
export function requireSignIn(reason?: string) {
  set({ auth: "signed-out", account: null, settings: null, loginError: reason ?? "Your sign-in expired. Sign in again." })
}

export function notify(notice: string) {
  set({ notice, noticeLink: null })
}

// ---------------------------------------------------------------------------
// Discord
// ---------------------------------------------------------------------------

let presenceKey = ""
let presenceTimer: ReturnType<typeof setTimeout> | undefined

/** What the window shows, for the Discord status: sent half a second after it settles, once. */
function watchPresence() {
  if (!bridge) return
  const o = state.open
  const p = {
    directory: o?.directory ?? null,
    title: o ? o.title : null,
    model: o?.info?.model ?? null,
    working: !!o && ((!!o.sessionId && state.busy.has(o.sessionId)) || o.status.kind === "working"),
  }
  const key = JSON.stringify(p)
  // A change that is undone within the wait sends nothing.
  clearTimeout(presenceTimer)
  if (key === presenceKey) return
  presenceTimer = setTimeout(() => {
    if (key === presenceKey) return
    presenceKey = key
    void bridge.presence(p).catch(() => {})
  }, 500)
}

/** A view read from the database; text it now holds stops streaming separately. */
function viewPatch(view: View, o: Open): Partial<Open> {
  const items = view.items
  return {
    items: keepSame(o.items, items),
    revert: view.revert,
    todos: view.todos,
    lastAgent: view.agent,
    // Messages held for a session still being made aren't the core's yet.
    queued: [...(view.queued ?? []), ...o.queued.filter((q) => q.id.startsWith(HELD))],
    context: view.context ?? null,
    background: view.background ?? [],
    // Only the same message's stored text ends its streaming: another turn may say the same words.
    live: o.live.filter((l) => {
      const message = l.key.slice(0, l.key.lastIndexOf(":") + 1)
      return !items.some((i) => "text" in i && i.text === l.text && i.id.startsWith(message))
    }),
  }
}

/** Items read again keep the objects already shown when nothing in them changed, so only the
 * rows that did change render again (a reply's markdown isn't parsed on every read). */
function keepSame(before: Item[], after: Item[]): Item[] {
  if (before.length === 0) return after
  const old = new Map(before.map((i) => [i.id, i]))
  return after.map((i) => {
    const was = old.get(i.id)
    return was && alike(was, i) ? was : i
  })
}

/** Equal as far as a row can show, without walking every character of a long output each read:
 * a long string compares by length and samples (output only grows; a changed one changes there). */
function alike(a: unknown, b: unknown): boolean {
  if (typeof a === "string" && typeof b === "string") {
    if (a.length !== b.length) return false
    if (a.length <= 2048) return a === b
    const n = a.length
    const mid = n >> 1
    return a.slice(0, 256) === b.slice(0, 256) && a.slice(n - 256) === b.slice(n - 256) && a.slice(mid - 128, mid + 128) === b.slice(mid - 128, mid + 128)
  }
  if (a === b) return true
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false
  if (Array.isArray(a) !== Array.isArray(b)) return false
  const ka = Object.keys(a)
  if (ka.length !== Object.keys(b).length) return false
  return ka.every((k) => alike((a as Record<string, unknown>)[k], (b as Record<string, unknown>)[k]))
}

/** Each read of the open view is numbered; an answer older than one already shown is dropped. */
let reads = 0
let shown = 0

function latest(n: number) {
  if (n < shown) return false
  shown = n
  return true
}

/** Drops the inputs of calls matching `gone`: their rows show what is stored from here. */
function dropInputs(gone: (i: { sessionId: string; callId: string }) => boolean) {
  const kept = Object.values(state.inputs).filter((i) => !gone(i))
  if (kept.length !== Object.keys(state.inputs).length) set({ inputs: Object.fromEntries(kept.map((i) => [i.callId, i])) })
  dropOutputs(gone)
}

function dropOutputs(gone: (o: ToolOutput) => boolean) {
  const kept = Object.values(state.outputs).filter((o) => !gone(o))
  if (kept.length !== Object.keys(state.outputs).length) set({ outputs: Object.fromEntries(kept.map((o) => [o.callId, o])) })
}

/** A call read back as anything but `streaming` has its input whole; one that isn't running
 * any more shows its stored output. */
function settleInputs(items: Item[]) {
  if (Object.keys(state.inputs).length === 0 && Object.keys(state.outputs).length === 0) return
  const tools = items.filter((i): i is Extract<Item, { kind: "tool" }> => i.kind === "tool")
  const whole = new Set(tools.filter((i) => i.status !== "streaming").map((i) => i.id))
  const ended = new Set(tools.filter((i) => i.status !== "streaming" && i.status !== "running").map((i) => i.id))
  const inputs = Object.values(state.inputs).filter((i) => !whole.has(i.callId))
  if (inputs.length !== Object.keys(state.inputs).length) set({ inputs: Object.fromEntries(inputs.map((i) => [i.callId, i])) })
  dropOutputs((o) => ended.has(o.callId))
}

async function refresh(sessionId: string, again = false) {
  const open = state.open
  if (!open || open.sessionId !== sessionId) return
  const n = ++reads
  try {
    const view = await bridge.transcript(open.directory, sessionId)
    settleInputs(view.items)
    if (latest(n)) setOpen((o) => viewPatch(view, o), sessionId)
  } catch (e) {
    // Once more after a moment; then the user hears that what shows may be behind.
    if (!again) setTimeout(() => void refresh(sessionId, true), 1500)
    else if (state.open?.sessionId === sessionId) set({ notice: `The conversation couldn’t be read again. ${message(e)}` })
  }
}

function toggle(set: Set<string>, id: string, on: boolean) {
  const next = new Set(set)
  if (on) next.add(id)
  else next.delete(id)
  return next
}

// ---------------------------------------------------------------------------
// Prompting
// ---------------------------------------------------------------------------

/** A message as the user sent it. `resolved`: the core took it (its turn may not have started). */
type Unsent = { text: string; files: Attachment[]; comments: Comment[]; resolved?: boolean }

/** Messages sent, by draft key in the order sent, until their turn starts: a send that fails or
 * is refused before then goes back into that session's draft instead of losing what was typed.
 * Each send is its own entry, so two in a row can't overwrite each other. */
const unsent = new Map<string, Unsent[]>()

function holdUnsent(key: string, u: Unsent) {
  unsent.set(key, [...(unsent.get(key) ?? []), u])
}

/** Takes `u` (or the oldest) out of a key's sends. */
function dropUnsent(key: string, u?: Unsent): Unsent | undefined {
  const list = unsent.get(key)
  if (!list?.length) return undefined
  const at = u ? list.indexOf(u) : 0
  if (at < 0) return undefined
  const [out] = list.splice(at, 1)
  if (list.length === 0) unsent.delete(key)
  return out
}

/** A turn of the session ended: every send the core took is delivered. */
function settleUnsent(key: string) {
  const list = unsent.get(key)?.filter((u) => !u.resolved)
  if (list?.length) unsent.set(key, list)
  else unsent.delete(key)
}

/** Messages a running turn took into its queue, by session, as sent: Edit on a queued chip gets
 * its images back from here (the queue itself only counts them). */
const sentQueued = new Map<string, Unsent[]>()

/**
 * The message a started turn took, by session, until that turn ends. `started` comes before the
 * core checks the prompt (UserPromptSubmit hooks, the model's route, the sign-in), so a turn can
 * still end `failed` + `unsent` and its message must be there to put back.
 */
const starting = new Map<string, Unsent>()

/** Puts a message that didn't send (`u`; else the one a started turn took, else the oldest send)
 * back in its session's draft; elsewhere, the notice says where. */
function putBack(key: string, u?: Unsent) {
  const taken = u ? undefined : starting.get(key)
  if (taken) starting.delete(key)
  const back = taken ?? dropUnsent(key, u)
  if (!back) return false
  restoreDraft(key, back)
  const o = state.open
  if (!o || draftKey(o) !== key) {
    const title = state.sessions.find((r) => r.id === key)?.title
    set({ notice: `Your message${title ? ` to “${title}”` : ""} didn’t send. It’s back in that session’s message box.` })
  }
  return true
}

/** Starts and ends per session (and sends), so a read of its busy state can tell whether one
 * came meanwhile and said more. Streaming text doesn't count: it says nothing about busy. */
const activity = new Map<string, number>()

function touch(sessionId: string) {
  activity.set(sessionId, (activity.get(sessionId) ?? 0) + 1)
}

/** First prompts in flight. Until each answers with its new session's id, that session's
 * events match nothing open, so its latest start or end is kept here to replay. */
let creating = 0
const early = new Map<string, CoreEvent>()
const LIFECYCLE = new Set(["started", "done", "failed", "interrupted", "refused", "settled"])

/** A message sent while its session is still being made: it waits, shown as queued, and goes
 * into the session once it exists. */
type Held = { id: string; sent: Unsent; text: string; skills: string[] }

/** A first prompt in flight, by view key: what was sent after it, and whether Stop was pressed. */
type First = { held: Held[]; stop: boolean }
const firsts = new Map<string, First>()

/** Ids of held messages' chips. */
const HELD = "held:"
let helds = 0

function beginFirst(open: Open): First {
  creating++
  const first: First = { held: [], stop: false }
  firsts.set(open.key, first)
  return first
}

function endFirst(open: Open) {
  firsts.delete(open.key)
  if (--creating === 0) early.clear()
}

/** The session exists: what was held goes into it (queued behind its first turn), then a Stop
 * pressed meanwhile stops it. */
async function release(open: Open, first: First, sessionId: string) {
  const view: Open = { ...open, sessionId, status: { kind: "working" } }
  for (const h of first.held.splice(0)) {
    setOpen((o) => ({ queued: o.queued.filter((q) => q.id !== h.id) }), sessionId)
    await deliver(view, h.sent, h.text, h.skills)
  }
  if (first.stop) stopSession(open.directory, sessionId)
}

/** The first prompt failed: what was held goes back to the draft, in the order it was sent. */
function abandon(open: Open, first: First, key: string) {
  const held = first.held.splice(0)
  for (const h of [...held].reverse()) restoreDraft(key, h.sent)
  if (held.length) setOpen((o) => ({ queued: o.queued.filter((q) => !held.some((h) => h.id === q.id)), stopping: false }), { key: open.key })
}

/** A held message, taken back (Edit or Remove on its chip). */
function takeHeld(id: string): Held | null {
  for (const first of firsts.values()) {
    const at = first.held.findIndex((h) => h.id === id)
    if (at >= 0) return first.held.splice(at, 1)[0]
  }
  return null
}

/** A new session's view takes the id its first prompt created, and any end that came first. */
function adopt(target: string | { key: string }, sessionId: string, directory: string) {
  // What was typed while the first prompt was on its way stays with the view.
  const from = `new:${folderKey(directory)}`
  if (showing(target)) moveDraft(from, sessionId)
  const moved = unsent.get(from)
  if (moved) {
    unsent.delete(from)
    unsent.set(sessionId, [...(unsent.get(sessionId) ?? []), ...moved])
  }
  setOpen({ sessionId }, target)
  void loadSessions()
  const e = early.get(sessionId)
  early.delete(sessionId)
  if (e && e.type !== "started") onEvent(e)
  return sessionId
}

/** Busy and status read back from the core, for when the events that would say so were lost
 * or never sent. A start or end that came meanwhile says more, so then only the view is taken. */
async function reconcile(directory: string, sessionId: string, again = false) {
  const since = activity.get(sessionId)
  const asked = asks
  const n = ++reads
  try {
    const opened = await bridge.openSession(directory, sessionId)
    const current = activity.get(sessionId) === since
    if (current) set((s) => ({ busy: toggle(s.busy, sessionId, opened.busy) }))
    if (state.open?.sessionId === sessionId) applyPending(directory, sessionId, opened.pending ?? [], asked)
    setOpen((o) => {
      const shown = o.status.kind === "working" || o.status.kind === "retrying"
      return {
        ...(latest(n) ? viewPatch(opened.view, o) : {}),
        // Nothing runs: a Stop still waiting for an end that won't come is over.
        ...(current && !opened.busy ? { stopping: false as const } : {}),
        ...(current && shown && !opened.busy
          ? { live: [], status: { kind: "idle" } as Status, stopping: false, started: null }
          : {}),
        ...(current && !shown && opened.busy
          ? { status: { kind: "working" } as Status, started: o.started ?? turnStart(opened.view.items) }
          : {}),
      }
    }, sessionId)
  } catch (e) {
    // Once more after a moment; then the user hears that the state shown may be behind.
    if (!again) setTimeout(() => void reconcile(directory, sessionId, true), 1500)
    else if (state.open?.sessionId === sessionId)
      set({ notice: `This session’s state couldn’t be read again. ${message(e)}` })
  }
}

/** True while a turn runs in the open view, including one being stopped. */
export function isRunning(o: Open) {
  return o.status.kind === "working" || o.status.kind === "retrying"
}

export async function send(text: string, files: Attachment[] = [], skills: string[] = []) {
  const open = state.open
  if (!open || (!text.trim() && files.length === 0 && state.comments.length === 0)) return
  const sent: Unsent = { text, files, comments: state.comments }
  const full = withComments(text)
  setDraft(draftKey(open), { comments: [] })
  const first = open.sessionId ? undefined : firsts.get(open.key)
  if (first) {
    // The session is still being made: a second send would make another. This one waits for it.
    const id = `${HELD}${++helds}`
    first.held.push({ id, sent, text: full, skills })
    setOpen((o) => ({ queued: [...o.queued, { id, text: full, images: files.length, delivery: "queue" }] }), { key: open.key })
    return
  }
  await deliver(open, sent, full, skills)
}

/** Sends into `open` as it was when the user sent (or as the held message's session now is). */
async function deliver(open: Open, sent: Unsent, text: string, skills: string[]) {
  let target = whose(open)
  const key = draftKey(open)
  holdUnsent(key, sent)
  // While a turn runs the message waits in its queue: the status stays as it is.
  const running = isRunning(open)
  // The prompt makes a staged rewind final.
  if (!running) setOpen({ status: { kind: "working" }, revert: null, started: Date.now(), stopping: false }, target)
  const first = open.sessionId ? null : beginFirst(open)
  if (open.sessionId) touch(open.sessionId)
  try {
    const { sessionId, queued } = await bridge.prompt(open.directory, {
      sessionId: open.sessionId,
      text,
      mode: state.mode,
      // A new session takes the model picked before it existed.
      model: first ? (open.info?.modelId ?? null) : null,
      variant: first ? (open.info?.variant ?? null) : null,
      agent: first ? (open.info?.agent ?? null) : null,
      files: sent.files,
      skills,
    })
    // Before the adoption, which may replay the turn's end.
    if (!queued) sent.resolved = true
    if (first) target = adopt(target, sessionId, open.directory)
    // A running turn took it: it is delivered, whatever that turn does next.
    if (queued && dropUnsent(sessionId, sent)) sentQueued.set(sessionId, [...(sentQueued.get(sessionId) ?? []), sent])
    else sent.resolved = true
    if (first) void release(open, first, sessionId)
    // Busy comes from `started`: the turn is spawned before this answer, and its end can
    // arrive first.
    await refresh(sessionId)
  } catch (e) {
    if (running) set({ notice: message(e) })
    else failed(target, e)
    if (first) abandon(open, first, key)
    putBack(typeof target === "string" ? target : key, sent)
  } finally {
    if (first) endFirst(open)
  }
}

// ---------------------------------------------------------------------------
// The queue
// ---------------------------------------------------------------------------

/** A queued message's text and images as sent, matched by text and image count. */
function takeSent(sessionId: string, q: Queued): Unsent {
  const list = sentQueued.get(sessionId) ?? []
  const at = list.findIndex((u) => commentsText(u.text, u.comments) === q.text && u.files.length === q.images)
  if (at < 0) return { text: q.text, files: [], comments: [] }
  const [u] = list.splice(at, 1)
  return u
}

/** Queue calls answer with the view; a failure says so and leaves the chips as they were. */
async function queueStep(step: (directory: string, sessionId: string) => Promise<View | void>) {
  const open = state.open
  if (!open?.sessionId) return false
  const sessionId = open.sessionId
  try {
    const view = await step(open.directory, sessionId)
    if (view) setOpen((o) => ({ queued: view.queued ?? [], context: view.context ?? o.context, background: view.background ?? o.background }), sessionId)
    else await refresh(sessionId)
    return true
  } catch (e) {
    set({ notice: message(e) })
    await refresh(sessionId)
    return false
  }
}

/** Edit: the queued message comes back to the composer (after what is typed there). */
export async function queueEdit(q: Queued) {
  const open = state.open
  if (open && q.id.startsWith(HELD)) {
    const h = takeHeld(q.id)
    setOpen((o) => ({ queued: o.queued.filter((x) => x.id !== q.id) }), { key: open.key })
    if (h) restoreDraft(draftKey(state.open ?? open), h.sent)
    return
  }
  if (!open?.sessionId) return
  const sessionId = open.sessionId
  const ok = await queueStep((d, s) => bridge.queueCancel(d, s, q.id))
  if (ok) {
    restoreDraft(sessionId, takeSent(sessionId, q))
  }
}

export async function queueRemove(q: Queued) {
  const open = state.open
  if (open && q.id.startsWith(HELD)) {
    takeHeld(q.id)
    setOpen((o) => ({ queued: o.queued.filter((x) => x.id !== q.id) }), { key: open.key })
    return
  }
  if (!open?.sessionId) return
  const sessionId = open.sessionId
  if (await queueStep((d, s) => bridge.queueCancel(d, s, q.id))) takeSent(sessionId, q)
}

/** Send now: the message joins the running turn at its next step. */
export async function queueSteer(q: Queued) {
  await queueStep((d, s) => bridge.queueSteer(d, s, q.id))
}

/** After a stop or a failure: what waits in the queue runs. */
export async function queueSend() {
  const open = state.open
  if (!open?.sessionId) return
  const sessionId = open.sessionId
  setOpen({ status: { kind: "working" }, started: Date.now(), stopping: false }, sessionId)
  touch(sessionId)
  try {
    await bridge.queueSend(open.directory, sessionId, state.mode)
    sentQueued.delete(sessionId)
    await refresh(sessionId)
  } catch (e) {
    setOpen({ status: { kind: "failed", error: message(e), retry: false } }, sessionId)
  }
}

/** Stops a command the agent left running in the background. */
export async function backgroundStop(id: string) {
  const open = state.open
  if (!open?.sessionId) return
  const sessionId = open.sessionId
  try {
    const stopped = await bridge.backgroundStop(open.directory, id)
    if (!stopped) set({ notice: "That command had already ended." })
  } catch (e) {
    set({ notice: message(e) })
  }
  setOpen((o) => ({ background: o.background.filter((b) => b.id !== id) }), sessionId)
  await refresh(sessionId)
}

/** A sent message's images, read once per message. */
const imageReads = new Map<string, Promise<string[]>>()

export function messageImages(directory: string, sessionId: string, messageId: string): Promise<string[]> {
  const key = `${sessionId}:${messageId}`
  let read = imageReads.get(key)
  if (!read) {
    read = bridge.messageImages(directory, sessionId, messageId)
    read.catch(() => imageReads.delete(key))
    imageReads.set(key, read)
  }
  return read
}

export async function revealFile(path: string, directory?: string) {
  if (elsewhere(path, directory)) return
  try {
    await bridge.revealPath(path)
  } catch (e) {
    set({ notice: message(e) })
  }
}

/**
 * A path on another computer (`\\host\share`, `//host/share`) outside the project folder: a
 * click on it would make the app reach that computer, so it isn't followed. Says so.
 */
function elsewhere(path: string, directory?: string) {
  if (!remotePath(path)) return false
  const key = folderKey(path)
  if (directory && remotePath(directory)) {
    const root = folderKey(directory)
    if (key === root || key.startsWith(`${root}\\`)) return false
  }
  set({ notice: "That file is on another computer, so it isn’t opened from here." })
  return true
}

/** A network path: two separators first, except a local `\\?\C:\…` one. */
export function remotePath(path: string) {
  const p = path.trim().replace(/\//g, "\\")
  return p.startsWith("\\\\") && !/^\\\\[?.]\\[a-z]:/i.test(p)
}

export async function chooseModel(modelId: string, variant: string | null) {
  const open = state.open
  if (!open?.info) return
  if (!open.sessionId) {
    const model = open.info.models.find((m) => m.id === modelId)
    setOpen({ info: { ...open.info, modelId, model: model?.name ?? modelId, variant } }, whose(open))
    return
  }
  try {
    const info = await bridge.selectModel(open.directory, open.sessionId, modelId, variant)
    setOpen((o) => ({ info: { ...info, fiveHour: info.fiveHour ?? o.info?.fiveHour ?? null } }), open.sessionId)
  } catch (e) {
    set({ notice: message(e) })
  }
}

// ---------------------------------------------------------------------------
// Rewind, retry, commands, agents
// ---------------------------------------------------------------------------

function lastUser(open: Open) {
  for (let i = open.items.length - 1; i >= 0; i--) {
    const item = open.items[i]
    if (item.kind === "user") return item
  }
  return null
}

/** Rewinds to before a message: its turns are hidden, their files put back, its text returns
 * to the composer. */
export async function rewind(messageId: string) {
  const open = state.open
  if (!open?.sessionId) return
  const sessionId = open.sessionId
  const n = ++reads
  try {
    const { text, view, filesKept } = await bridge.revert(open.directory, sessionId, messageId)
    latest(n)
    setOpen((o) => ({ ...viewPatch(view, o), live: [], status: { kind: "idle" } }), sessionId)
    // The message returns to the composer, in front of what was being typed there (kept, as are
    // folded pastes); Redo takes it out again.
    rewound.set(sessionId, text)
    if (text) restoreDraft(sessionId, { text })
    if (filesKept)
      set({ notice: "This folder isn’t a git repository, so the files those turns changed stay as they are." })
  } catch (e) {
    set({ notice: message(e) })
  }
}

/** Rewinds the last turn. */
export async function undo() {
  const open = state.open
  const last = open && lastUser(open)
  if (!last) {
    set({ notice: "Nothing to undo." })
    return
  }
  await rewind(last.id)
}

export async function redo() {
  const open = state.open
  if (!open?.sessionId || !open.revert) return
  const sessionId = open.sessionId
  const n = ++reads
  try {
    const view = await bridge.unrevert(open.directory, sessionId)
    latest(n)
    setOpen((o) => viewPatch(view, o), sessionId)
    // The rewound message leaves the composer unless the user changed it; what they had typed stays.
    const text = rewound.get(sessionId)
    rewound.delete(sessionId)
    const now = state.drafts[sessionId]?.text
    if (text && now === text) setDraft(sessionId, { text: "", stamp: ++stamps })
    else if (text && now?.startsWith(`${text}\n\n`)) setDraft(sessionId, { text: now.slice(text.length + 2), stamp: ++stamps })
  } catch (e) {
    set({ notice: message(e) })
  }
}

/** The text a rewind put in a session's composer, so Redo can take it out again. */
const rewound = new Map<string, string>()

/** Sends a message again from where it was, with new text, or as it was when `text` is null. */
export async function resend(messageId: string, text: string | null) {
  const open = state.open
  if (!open?.sessionId) return
  const sessionId = open.sessionId
  setOpen({ status: { kind: "working" }, live: [], revert: null, started: Date.now(), stopping: false }, sessionId)
  touch(sessionId)
  try {
    await bridge.resend(open.directory, sessionId, messageId, text, state.mode)
    await refresh(sessionId)
  } catch (e) {
    setOpen({ status: { kind: "failed", error: message(e) } }, sessionId)
  }
}

/** Sends the last message again: only after a turn that failed on its own (a message that never
 * started went back to the composer; resending the one before it would be a surprise). */
export async function retry() {
  const open = state.open
  if (!open) return
  const last = lastUser(open)
  const tail = open.items[open.items.length - 1]
  const refused =
    (open.status.kind === "failed" && open.status.retry === false) ||
    (open.status.kind !== "failed" && tail?.kind === "error" && tail.retry === false)
  if (refused) {
    set({ notice: "There’s nothing to try again: sending again won’t help with this one." })
    return
  }
  if (last) await resend(last.id, null)
}

const fileLists = new Map<string, { at: number; list: Promise<string[]> }>()

/** The folder's commands and skills; read again on each menu open, as MCP prompts arrive late. */
export function palette(directory: string): Promise<Palette> {
  return bridge.palette(directory)
}

/** The folder's files for `@`, read at most every 30 seconds. */
export function projectFiles(directory: string): Promise<string[]> {
  const cached = fileLists.get(directory)
  if (cached && Date.now() - cached.at < 30_000) return cached.list
  const list = bridge.projectFiles(directory)
  fileLists.set(directory, { at: Date.now(), list })
  list.catch(() => fileLists.delete(directory))
  return list
}

/** Runs `/<name> <arguments>`; a new session's command starts the session. */
export async function runCommand(name: string, args: string) {
  const open = state.open
  if (!open) return
  let target = whose(open)
  const key = draftKey(open)
  const sent: Unsent = { text: args ? `/${name} ${args}` : `/${name}`, files: [], comments: [] }
  holdUnsent(key, sent)
  setOpen({ status: { kind: "working" }, revert: null, started: Date.now(), stopping: false }, target)
  const first = open.sessionId ? null : beginFirst(open)
  if (open.sessionId) touch(open.sessionId)
  try {
    const { sessionId } = await bridge.runCommand(open.directory, {
      sessionId: open.sessionId,
      name,
      arguments: args,
      mode: state.mode,
      model: first ? (open.info?.modelId ?? null) : null,
      variant: first ? (open.info?.variant ?? null) : null,
      agent: first ? (open.info?.agent ?? null) : null,
    })
    sent.resolved = true
    if (first) target = adopt(target, sessionId, open.directory)
    if (first) void release(open, first, sessionId)
    // Busy comes from `started`: the turn is spawned before this answer, and its end can
    // arrive first.
    await refresh(sessionId)
  } catch (e) {
    failed(target, e)
    if (first) abandon(open, first, key)
    putBack(typeof target === "string" ? target : key, sent)
  } finally {
    if (first) endFirst(open)
  }
}

export async function compact() {
  const open = state.open
  if (!open?.sessionId) {
    set({ notice: "There’s nothing to compact yet." })
    return
  }
  const sessionId = open.sessionId
  setOpen({ status: { kind: "working" }, started: Date.now(), stopping: false }, sessionId)
  touch(sessionId)
  try {
    await bridge.compact(open.directory, sessionId)
  } catch (e) {
    setOpen({ status: { kind: "failed", error: message(e) } }, sessionId)
  }
}

/** The session's agent (Build, Plan…); a new session takes it with its first prompt. */
export async function chooseAgent(agent: string) {
  const open = state.open
  if (!open?.info || open.info.agent === agent) return
  if (!open.sessionId) {
    setOpen({ info: { ...open.info, agent } }, whose(open))
    return
  }
  const sessionId = open.sessionId
  // Shown at once; the core records the switch.
  setOpen((o) => ({ info: o.info && { ...o.info, agent } }), sessionId)
  try {
    const info = await bridge.selectAgent(open.directory, sessionId, agent)
    setOpen((o) => ({ info: { ...info, fiveHour: info.fiveHour ?? o.info?.fiveHour ?? null } }), sessionId)
  } catch (e) {
    set({ notice: message(e) })
  }
}

/** Plan approved: the session switches to Build and carries the plan out. */
export async function buildPlan() {
  const open = state.open
  if (!open?.sessionId) return
  const sessionId = open.sessionId
  await chooseAgent("build")
  // Another session may be open by now: the go-ahead still goes to the plan's own.
  const text = "Go ahead with the plan."
  if (state.open?.sessionId === sessionId) await send(text)
  else await deliver(open, { text, files: [], comments: [] }, text, [])
}

export async function peekAt(directory: string, sessionId: string, title: string) {
  set({ peek: { directory, sessionId, title, items: [], loading: true } })
  await refreshPeek()
}

export function closePeek() {
  set({ peek: null })
}

async function refreshPeek() {
  const peek = state.peek
  if (!peek) return
  try {
    const { items } = await bridge.transcript(peek.directory, peek.sessionId)
    settleInputs(items)
    set((s) => (s.peek?.sessionId === peek.sessionId ? { peek: { ...s.peek, items, loading: false } } : {}))
  } catch (e) {
    set({ peek: null, notice: message(e) })
  }
}

const GONE = "This request is no longer waiting."

/**
 * Sends an answer: its card stays, disabled, until the answer arrives (then goes), so a failure
 * leaves it as it was with what was typed or picked in it. Its id is marked closed from the start,
 * so a read of what is pending that began before can't bring the card back.
 */
async function answerAsk(id: string, drop: (s: State) => Partial<State>, call: () => Promise<boolean>) {
  if (state.sending.has(id)) return
  closed.add(id)
  set((s) => ({ sending: toggle(s.sending, id, true) }))
  try {
    const waiting = await call()
    set((s) => ({ ...drop(s), sending: toggle(s.sending, id, false), ...(waiting ? {} : { notice: GONE }) }))
  } catch (e) {
    // The answer didn't reach it: the card is there to answer again.
    closed.delete(id)
    set((s) => ({ sending: toggle(s.sending, id, false), notice: message(e) }))
  }
}

export async function answer(q: Asked, answers: string[][] | null) {
  await answerAsk(
    q.requestId,
    (s) => ({ questions: s.questions.filter((x) => x.requestId !== q.requestId) }),
    () => bridge.replyQuestion(q.directory, q.requestId, answers)
  )
}

/** An MCP form's answer; a form that is no longer waiting just goes. */
export async function replyForm(f: FormRequest, answer: FormAnswer) {
  await answerAsk(
    f.formId,
    (s) => ({ forms: s.forms.filter((x) => x.formId !== f.formId) }),
    () => bridge.replyForm(f.directory, f.formId, answer)
  )
}

/** Which asks arrived when, so a read of what is pending keeps the cards that came meanwhile. */
let asks = 0
const arrived = new Map<string, number>()

/** Asks answered (or ended with their turn) since the window started: a read that began before
 * still lists them, and must not bring their cards back. */
const closed = new Set<string>()

/** Reads what waits under a session that isn't open (after events were lost). */
async function readPending(directory: string, sessionId: string) {
  const since = asks
  try {
    applyPending(directory, sessionId, await bridge.pending(directory, sessionId), since)
  } catch {
    // The next event or opening the session reads it again.
  }
}

/** What waits for an answer under `rootId` (and its folder's forms), as the core says: the cards
 * shown for it are replaced, except ones whose event came after the read began. */
function applyPending(directory: string, rootId: string, list: PendingRequest[], since: number) {
  const newer = (id: string) => (arrived.get(id) ?? 0) > since
  const permissions: Permission[] = []
  const questions: Asked[] = []
  const forms: FormRequest[] = []
  for (const r of list) {
    if (closed.has(r.type === "form" ? r.formId : r.requestId)) continue
    if (r.type === "permission")
      permissions.push({ ...r, rootId: r.rootId ?? rootId, directory: r.directory || directory })
    else if (r.type === "question")
      questions.push({
        requestId: r.requestId,
        sessionId: r.sessionId,
        rootId: r.rootId ?? rootId,
        directory: r.directory || directory,
        questions: r.questions,
      })
    else forms.push({ ...r, directory: r.directory || directory })
  }
  set((s) => ({
    // A card whose answer is on its way stays: should the answer fail, it is answered again there.
    permissions: [
      ...s.permissions.filter((p) => p.rootId !== rootId || newer(p.requestId) || s.sending.has(p.requestId)),
      ...permissions.filter((p) => !s.permissions.some((x) => x.requestId === p.requestId && newer(x.requestId))),
    ],
    questions: [
      ...s.questions.filter((q) => q.rootId !== rootId || newer(q.requestId) || s.sending.has(q.requestId)),
      ...questions.filter((q) => !s.questions.some((x) => x.requestId === q.requestId && newer(x.requestId))),
    ],
    forms: [
      ...s.forms.filter((f) => !sameFolder(f.directory, directory) || newer(f.formId) || s.sending.has(f.formId)),
      ...forms.filter((f) => !s.forms.some((x) => x.formId === f.formId && newer(x.formId))),
    ],
  }))
}

export async function renameSession(row: SessionRow, title: string) {
  try {
    await bridge.renameSession(row.id, title)
    set((s) => ({ sessions: s.sessions.map((r) => (r.id === row.id ? { ...r, title: title.trim() } : r)) }))
    if (state.open?.sessionId === row.id) setOpen({ title: title.trim() }, row.id)
  } catch (e) {
    set({ notice: message(e) })
  }
}

export async function deleteSession(row: SessionRow) {
  await deleteSessions([row])
}

/** Deletes sessions one by one; running ones are kept and named in the notice. */
export async function deleteSessions(rows: SessionRow[]) {
  const gone = new Set<string>()
  const kept: string[] = []
  const errors: string[] = []
  for (const row of rows) {
    try {
      // False: it was already gone, which is what was asked.
      await bridge.deleteSession(row.directory, row.id)
      gone.add(row.id)
    } catch (e) {
      // The core refuses a running session; anything else is its own error.
      if (state.busy.has(row.id) || /\bstop\b/i.test(message(e))) kept.push(row.title)
      else errors.push(`“${row.title}” wasn’t deleted. ${message(e)}`)
    }
  }
  for (const id of gone) {
    unsent.delete(id)
    if (state.drafts[id]) setDraft(id, { ...EMPTY_DRAFT })
  }
  if (errors.length) set({ notice: errors.length === 1 ? errors[0] : `${errors.length} sessions weren’t deleted. ${errors[0]}` })
  if (state.open?.sessionId && gone.has(state.open.sessionId)) set({ open: null })
  set((s) => {
    const pinned = new Set([...s.pinned].filter((id) => !gone.has(id)))
    saveSet("anysola.pinned", pinned)
    return { sessions: s.sessions.filter((r) => !gone.has(r.id)), pinned }
  })
  if (kept.length)
    set({
      notice:
        kept.length === 1
          ? `“${kept[0]}” is still running, so it was kept. Stop it to delete it.`
          : `${kept.length} sessions are still running, so they were kept. Stop them to delete them.`,
    })
}

export function togglePin(id: string) {
  set((s) => {
    const pinned = new Set(s.pinned)
    if (!pinned.delete(id)) pinned.add(id)
    saveSet("anysola.pinned", pinned)
    return { pinned }
  })
}

export function toggleCollapsed(directory: string) {
  set((s) => {
    const collapsed = new Set(s.collapsed)
    const key = folderKey(directory)
    if (!collapsed.delete(key)) collapsed.add(key)
    saveSet("anysola.collapsed", collapsed)
    return { collapsed }
  })
}

export async function reveal(directory: string) {
  try {
    await bridge.reveal(directory)
  } catch (e) {
    set({ notice: message(e) })
  }
}

// ---------------------------------------------------------------------------
// Review and git
// ---------------------------------------------------------------------------

/** The session's net file changes (null outside git: the window sums the edits instead), or why
 * they couldn't be read. */
export async function readChanges(): Promise<FileDiff[] | null | { error: string }> {
  const open = state.open
  if (!open?.sessionId) return null
  try {
    return await bridge.changes(open.directory, open.sessionId)
  } catch (e) {
    return { error: message(e) }
  }
}

/** The open view, for a run of steps that must all go to where they started. */
export function openNow() {
  return state.open
}

/** Puts one file back as it was before the session changed it (`open`: the session it was asked
 * in, when one of several). */
export async function restoreFile(file: string, open = state.open) {
  if (!open?.sessionId) return false
  try {
    await bridge.restoreFile(open.directory, open.sessionId, file)
    void refreshGit(open.directory)
    return true
  } catch (e) {
    set({ notice: message(e) })
    return false
  }
}

let comments = 0

export function addComment(c: Omit<Comment, "id">) {
  const open = state.open
  if (open) setDraft(draftKey(open), (d) => ({ comments: [...d.comments, { ...c, id: ++comments }] }))
}

export function removeComment(id: number) {
  const open = state.open
  if (open) setDraft(draftKey(open), (d) => ({ comments: d.comments.filter((c) => c.id !== id) }))
}

/** The comments as text the model reads after the message. */
function withComments(text: string) {
  return commentsText(text, state.comments)
}

function commentsText(text: string, comments: Comment[]) {
  if (comments.length === 0) return text
  const list = comments
    .map((c) => {
      const where = c.removed ? `${c.file}, removed line ${c.line}` : `${c.file}:${c.line}`
      const code = c.code.trim() ? ` (\`${c.code.trim()}\`)` : ""
      return `- ${where}${code}: ${c.text}`
    })
    .join("\n")
  const head = text.trim() ? `${text}\n\n` : ""
  return `${head}Comments on the changes:\n${list}`
}

export async function refreshGit(directory = state.open?.directory) {
  if (!directory) return
  try {
    const status = await bridge.gitStatus(directory)
    if (state.open && !sameFolder(state.open.directory, directory)) return
    set({ git: { ...status, directory }, gitError: null })
  } catch (e) {
    if (state.open && !sameFolder(state.open.directory, directory)) return
    set({ git: null, gitError: { directory, message: message(e) } })
  }
}

/** A git step from the header menu; true when it worked. Errors are git's own words. */
async function gitStep(step: (directory: string) => Promise<unknown>): Promise<string | true> {
  const open = state.open
  if (!open) return "No folder is open."
  try {
    await step(open.directory)
    await refreshGit(open.directory)
    return true
  } catch (e) {
    await refreshGit(open.directory)
    return message(e)
  }
}

/** `files` null commits every change. */
export const gitCommit = (text: string, files: string[] | null = null) => gitStep((d) => bridge.gitCommit(d, text, files))
export const gitPush = () => gitStep((d) => bridge.gitPush(d))
export const gitPull = () => gitStep((d) => bridge.gitPull(d))
export const gitBranch = (name: string) => gitStep((d) => bridge.gitBranch(d, name))
export const gitIdentity = (name: string, email: string) => gitStep(() => bridge.gitIdentity(name, email))

/** The folder (`path` null) or a file of it in the preferred editor; a failure goes to the notice. */
export async function openInEditor(directory: string, path: string | null = null, line: number | null = null) {
  if (path && elsewhere(path, directory)) return
  try {
    await bridge.openInEditor(directory, path, line)
  } catch (e) {
    set({ notice: message(e) })
  }
}

/** Puts text in the open view's composer (a starter prompt); what the user typed is replaced. */
export function fillComposer(text: string) {
  const open = state.open
  if (open) setDraft(draftKey(open), { text, pasted: [], stamp: ++stamps })
}

/** Leaves the open session for the empty window (its delete waits behind an Undo). */
export function closeSession() {
  set({ open: null })
}

export async function gitPr(title: string, body: string): Promise<{ url: string; created: boolean } | string> {
  const open = state.open
  if (!open) return "No folder is open."
  try {
    return await bridge.gitPr(open.directory, title, body)
  } catch (e) {
    return message(e)
  }
}

export async function suggestCommitMessage(): Promise<string | null> {
  const open = state.open
  if (!open?.sessionId) return null
  try {
    return await bridge.suggestCommitMessage(open.directory, open.sessionId)
  } catch (e) {
    set({ notice: message(e) })
    return null
  }
}

export function openExternal(url: string) {
  void bridge.openUrl(url).catch(() => set({ notice: `Your browser didn’t open. The link is ${url}` }))
}

/** How long a stop waits for the turn's end before asking whether anything was running. */
const STOP_GRACE = 1500

/** After this long without an end, Stop can be pressed again (the first may have been lost). */
export const STOP_AGAIN = 4000

export function stop() {
  const open = state.open
  if (!open || (open.stopping && Date.now() - open.stopping < STOP_AGAIN)) return
  if (!open.sessionId) {
    // A new session's first prompt: there is nothing to stop until the session exists.
    const first = firsts.get(open.key)
    if (!first) return
    first.stop = true
    setOpen({ stopping: Date.now() }, { key: open.key })
    return
  }
  stopSession(open.directory, open.sessionId)
}

function stopSession(directory: string, sessionId: string) {
  const since = activity.get(sessionId)
  // Said at once; the turn's end (and the "Stopped" note the core adds) follows.
  setOpen({ stopping: Date.now() }, sessionId)
  bridge.interrupt(directory, sessionId).then(
    // Stopping a session with nothing running sends no event; "Working" would stay.
    () =>
      setTimeout(() => {
        if (activity.get(sessionId) === since) void reconcile(directory, sessionId)
      }, STOP_GRACE),
    (e) => {
      setOpen({ stopping: false }, sessionId)
      set({ notice: message(e) })
    }
  )
}

export function setMode(mode: Mode) {
  try {
    localStorage.setItem("anysola.mode", mode)
  } catch {
    // Kept for this window only.
  }
  set({ mode })
  // The open session's floor follows at once; a running turn picks it up at its next step.
  // An ask already waiting stays for the user: the floor only adds asks, so it came from a rule.
  const open = state.open
  if (open?.sessionId) applyMode(open.directory, open.sessionId, mode)
}

export async function reply(p: Permission, answer: PermissionReply) {
  await answerAsk(
    p.requestId,
    (s) => ({ permissions: s.permissions.filter((x) => x.requestId !== p.requestId) }),
    () => bridge.replyPermission(p.directory, p.requestId, answer)
  )
}

/** The close confirm's answers. */
export function quitNow() {
  set({ closing: null })
  void runBeforeClose()
    .then(() => bridge.quit())
    .catch((e) => set({ notice: message(e) }))
}

export function keepWorking() {
  set({ closing: null })
  // Harmless when it fails: closing again within a few seconds then closes without asking.
  void bridge.keepOpen().catch(() => {})
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

function directoryOf(sessionId: string) {
  if (state.open?.sessionId === sessionId) return state.open.directory
  return state.sessions.find((s) => s.id === sessionId)?.directory ?? ""
}

/** UserPromptSubmit hooks' reasons, by session, until the `failed` + `unsent` they explain arrives. */
const promptBlocked = new Map<string, string[]>()

/** A hook that failed outside a tool call, for the notice; the core's message names the cause. */
function hookFailure(command: string, message: string) {
  const short = command.length > 60 ? `${command.slice(0, 59)}…` : command
  // "The hook exited with code 1: …" reads on from "A hook failed" as "It exited…".
  const why = message.replace(/^The hook (?=exited|timed out|could not start|was ended|printed)/, "It ").replace(/\.?$/, ".")
  return `${short ? `A hook failed (${short}).` : "A hook failed."} ${why} The work went on.`
}

/**
 * A turn that ran with queued messages ended `failed` + `unsent`: the core removed one message and
 * doesn't say which. It is, of the messages this window sent for the
 * turn, the one in neither the conversation nor the queue now. Put back only when exactly one is.
 */
async function findUnsent(directory: string, sessionId: string) {
  const candidates = [...(starting.has(sessionId) ? [starting.get(sessionId)!] : []), ...(sentQueued.get(sessionId) ?? [])]
  starting.delete(sessionId)
  let view: View
  try {
    view = await bridge.transcript(directory, sessionId)
  } catch {
    return
  }
  const said = (u: Unsent) => commentsText(u.text, u.comments).trim()
  const users = view.items.flatMap((i) => (i.kind === "user" ? [i.text.trim()] : []))
  const waiting = (view.queued ?? []).map((q) => q.text.trim())
  const missing = candidates.filter((u) => !users.includes(said(u)) && !waiting.includes(said(u)))
  if (missing.length !== 1) return
  const [u] = missing
  const list = sentQueued.get(sessionId)
  if (list?.includes(u)) list.splice(list.indexOf(u), 1)
  restoreDraft(sessionId, u)
  const o = state.open
  if (o?.sessionId === sessionId) {
    setOpen((v) => (v.status.kind === "failed" ? { status: { ...v.status, back: true } } : {}), sessionId)
  } else {
    const title = state.sessions.find((r) => r.id === sessionId)?.title
    set({ notice: `Your message${title ? ` to “${title}”` : ""} didn’t send. It’s back in that session’s message box.` })
  }
}

/** Kinds of failure that sending the same message again won't fix. */
const NO_RETRY = new Set<ErrorKind>(["limit", "auth", "model", "elsewhere"])

/** A session's turn is over: the install that waited for it may go ahead. */
function turnEnded() {
  if (state.update?.waiting && state.busy.size === 0) void installUpdate(true)
}

function onEvent(e: CoreEvent) {
  switch (e.type) {
    case "notice":
      // An MCP server's trouble is fixed in Settings: the notice links there.
      set({ notice: e.message, noticeLink: /\bMCP\b/i.test(e.message) ? "mcp" : null })
      return
    case "mcp":
      set((s) => ({ mcpChanged: s.mcpChanged + 1 }))
      return
    case "hooks":
      set((s) => ({ hooksChanged: s.hooksChanged + 1 }))
      return
    case "trust":
      set((s) => ({ trustChanged: s.trustChanged + 1 }))
      return
    case "hook":
      // A blocked message comes back as `failed` + `unsent` right after: its line says why.
      // Several hooks can block one message: each sends its reason, `failed` has them all.
      if (e.event === "UserPromptSubmit" && e.outcome === "blocked")
        promptBlocked.set(e.sessionId, [...(promptBlocked.get(e.sessionId) ?? []), e.message])
      // Blocks, feedback and context reach the transcript as notes; these two are for the user.
      if (e.outcome === "notice") set({ notice: e.message, noticeLink: null })
      if (e.outcome === "error") set({ notice: hookFailure(e.command, e.message), noticeLink: "hooks" })
      return
    case "lagged": {
      // Lost events may have started or ended turns, or asked something: read busy, status and
      // what waits back, not only the text.
      const open = state.open
      if (open?.sessionId && sameFolder(open.directory, e.directory)) void reconcile(open.directory, open.sessionId)
      // The folder's other running sessions may have asked something meanwhile (the sidebar
      // marks them): only what waits is read for those.
      for (const id of state.busy) {
        const row = state.sessions.find((r) => r.id === id)
        if (id !== open?.sessionId && row && sameFolder(row.directory, e.directory)) void readPending(row.directory, id)
      }
      void loadSessions()
      return
    }
    case "form":
      arrived.set(e.formId, ++asks)
      set((s) => ({ forms: [...s.forms.filter((f) => f.formId !== e.formId), e] }))
      return
    case "formReplied":
      closed.add(e.formId)
      set((s) => ({ forms: s.forms.filter((f) => f.formId !== e.formId) }))
      return
    case "closeRequested":
      set({ closing: e.busy })
      return
    case "update":
      if (state.update) set({ update: { ...state.update, installing: true, progress: { downloaded: e.downloaded, total: e.total } } })
      return
  }
  const id = e.sessionId
  const here = state.open?.sessionId === id
  if (LIFECYCLE.has(e.type)) {
    touch(id)
    if (!here && creating > 0) early.set(id, e)
  }
  if (state.peek?.sessionId === id && (e.type === "changed" || e.type === "done" || e.type === "failed"))
    void refreshPeek()
  switch (e.type) {
    case "started": {
      // The oldest send's turn started. It isn't delivered yet: the core may still refuse it
      // (a hook, no route to the model), and then the turn ends `failed` + `unsent`.
      const took = dropUnsent(id)
      if (took) starting.set(id, took)
      set((s) => ({ busy: toggle(s.busy, id, true), offline: false }))
      if (here)
        setOpen((o) => ({ status: { kind: "working" }, stopping: false, started: isRunning(o) && o.started ? o.started : Date.now() }), id)
      break
    }
    case "text":
    case "reasoning":
      if (!here) break
      setOpen((o) => {
        const key = `${e.messageId}:${e.ordinal}`
        const live = [...o.live]
        const at = live.findIndex((l) => l.key === key)
        if (at >= 0) live[at] = { ...live[at], text: live[at].text + e.text }
        else live.push({ key, kind: e.type, text: e.text })
        return { live, status: { kind: "working" } }
      }, id)
      break
    case "changed":
      if (here) {
        // Reasoning isn't shown; its live entry only says "Thinking" while it streams, and its
        // block's end is a `changed`.
        if (state.open?.live.some((l) => l.kind === "reasoning"))
          setOpen((o) => ({ live: o.live.filter((l) => l.kind !== "reasoning") }), id)
        void refresh(id)
      }
      break
    case "toolInput":
      set((s) => ({ inputs: { ...s.inputs, [e.callId]: e } }))
      break
    case "toolOutput":
      set((s) => ({ outputs: { ...s.outputs, [e.callId]: e } }))
      break
    case "retrying":
      // The step starts over: what it was writing won't finish.
      dropInputs((i) => i.sessionId === id)
      if (here)
        setOpen(
          { status: { kind: "retrying", attempt: e.attempt, until: Date.now() + e.delayMs, error: e.error } },
          id
        )
      break
    case "permission": {
      // The mode is the session's floor in the core; an ask that reaches here is one a rule
      // (`read *.env`, the user's own) wants asked, whatever the mode.
      arrived.set(e.requestId, ++asks)
      const { type: _, ...rest } = e
      const p: Permission = { ...rest, rootId: e.rootId ?? id, directory: e.directory || directoryOf(id) }
      set((s) => ({ permissions: [...s.permissions.filter((x) => x.requestId !== e.requestId), p] }))
      break
    }
    case "question":
      arrived.set(e.requestId, ++asks)
      set((s) => ({
        questions: [
          ...s.questions.filter((x) => x.requestId !== e.requestId),
          {
            requestId: e.requestId,
            sessionId: id,
            rootId: e.rootId ?? id,
            directory: e.directory || directoryOf(id),
            questions: e.questions,
          },
        ],
      }))
      break
    case "permissionReplied":
      closed.add(e.requestId)
      set((s) => ({ permissions: s.permissions.filter((x) => x.requestId !== e.requestId) }))
      break
    case "questionReplied":
      closed.add(e.requestId)
      set((s) => ({ questions: s.questions.filter((x) => x.requestId !== e.requestId) }))
      break
    case "title":
      set((s) => ({ sessions: s.sessions.map((r) => (r.id === id ? { ...r, title: e.title } : r)) }))
      if (here) setOpen({ title: e.title }, id)
      break
    case "done":
    case "failed":
    case "interrupted": {
      dropInputs((i) => i.sessionId === id)
      const unsentFail = e.type === "failed" && !!e.unsent
      // Messages a running turn took into its queue: one of them may be what the core refused.
      const queuedSent = unsentFail && !!sentQueued.get(id)?.length
      // A message that never started goes back where it was typed; any other was delivered. With
      // queued messages in play the transcript tells which one it was (below).
      const back = unsentFail && !queuedSent ? putBack(id) : (settleUnsent(id), false)
      if (!unsentFail || !queuedSent) starting.delete(id)
      const hook = unsentFail && promptBlocked.has(id)
      if (e.type === "failed") promptBlocked.delete(id)
      if (queuedSent) void findUnsent(directoryOf(id), id)
      // Its asks end with it.
      for (const p of state.permissions) if (p.sessionId === id || p.rootId === id) closed.add(p.requestId)
      for (const q of state.questions) if (q.sessionId === id || q.rootId === id) closed.add(q.requestId)
      set((s) => ({
        busy: toggle(s.busy, id, false),
        permissions: s.permissions.filter((p) => p.sessionId !== id && p.rootId !== id),
        questions: s.questions.filter((q) => q.sessionId !== id && q.rootId !== id),
        ...(e.type === "failed" && e.kind === "offline" ? { offline: true } : {}),
      }))
      void loadAccount()
      if (here) {
        setOpen(
          {
            live: [],
            stopping: false,
            started: null,
            status:
              e.type === "done"
                ? { kind: "idle" }
                : e.type === "failed"
                  ? {
                      kind: "failed",
                      error: e.error,
                      retry: !back && !e.unsent && !(e.kind && NO_RETRY.has(e.kind)),
                      errorKind: e.kind,
                      hook,
                      back,
                    }
                  : { kind: "stopped", reason: e.reason },
          },
          id
        )
        void refresh(id)
        // The turn may have changed files: the header's git count follows.
        void refreshGit()
      }
      void loadSessions()
      turnEnded()
      break
    }
    case "refused":
      dropInputs((i) => i.sessionId === id)
      starting.delete(id)
      set((s) => ({ busy: toggle(s.busy, id, false) }))
      // Kept in that session's draft, open or not; the notice says so when it isn't.
      putBack(id)
      if (here) {
        setOpen({ live: [], stopping: false, started: null, status: { kind: "failed", error: e.error, retry: false, errorKind: e.kind } }, id)
        void refresh(id)
      }
      turnEnded()
      break
    case "settled":
      dropInputs((i) => i.sessionId === id)
      settleUnsent(id)
      starting.delete(id)
      set((s) => ({ busy: toggle(s.busy, id, false) }))
      if (here) {
        setOpen((o) => (isRunning(o) ? { live: [], status: { kind: "idle" }, stopping: false, started: null } : {}), id)
        void refresh(id)
        void refreshGit()
      }
      void loadSessions()
      turnEnded()
      break
    case "usage":
      if (here && e.fiveHour !== null) setOpen((o) => ({ info: o.info && { ...o.info, fiveHour: e.fiveHour } }), id)
      break
    case "created":
      void loadSessions()
      break
  }
}

/** What a folder's project files want to run, as just read or answered. */
export function setTrust(directory: string, items: TrustItem[]) {
  set((s) => ({ trust: { ...s.trust, [folderKey(directory)]: items } }))
}

/**
 * An answer about a project file was recorded here: every view that shows what it answered reads
 * again (the card, Settings' hooks and servers), whether or not the core's events follow.
 */
export function trustAnswered(kinds: TrustKind[]) {
  set((s) => ({
    trustChanged: s.trustChanged + 1,
    ...(kinds.includes("hooks") ? { hooksChanged: s.hooksChanged + 1 } : {}),
    ...(kinds.includes("mcp") ? { mcpChanged: s.mcpChanged + 1 } : {}),
  }))
}

/** A folder's hooks as just read or changed, for every view that shows them. */
export function setHooks(directory: string, hooks: Hooks) {
  set((s) => ({ hooks: { ...s.hooks, [folderKey(directory)]: hooks } }))
}

export function dismissNotice() {
  set({ notice: null, noticeLink: null })
}
