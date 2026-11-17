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
  type Bridge,
  type CoreEvent,
  type FileDiff,
  type GitStatus,
  type Info,
  type Item,
  type Mode,
  type Palette,
  type PermissionReply,
  type Question,
  type Revert,
  type SessionRow,
  type Todo,
  type UpdateCheck,
  type View,
} from "./bridge"
import { folderKey, sameFolder } from "./ui"

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
  /** `retry`: the latest turn failed, so sending its message again makes sense. */
  | { kind: "failed"; error: string; retry?: boolean }
  | { kind: "stopped"; reason: "user" | "budget" | "declined" }

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
  /** Text for the composer to take (a rewound message, or one that didn't send); `n` tells
   * refills apart. With `replacing`, only while the composer still holds that text. */
  fill: { text: string; n: number; replacing?: string; files?: Attachment[] } | null
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
  update: (UpdateCheck & { installing?: boolean; error?: string }) | null
  open: Open | null
  peek: Peek | null
  mode: Mode
  notice: string | null
  /** The folder new sessions start in; null asks each time. */
  defaultFolder: string | null
  /** Sessions kept at the top of the sidebar, and project groups folded shut (by `folderKey`). */
  pinned: Set<string>
  collapsed: Set<string>
  /** The open session's folder in git, once read. */
  git: (GitStatus & { directory: string }) | null
  /** Diff comments waiting for the next message, by session (`""` for a new one). */
  comments: Comment[]
  /** The app's folder for sessions without a project, once known. */
  scratch: string | null
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

function readDefaultFolder(): string | null {
  try {
    return localStorage.getItem("anysola.defaultFolder") || null
  } catch {
    return null
  }
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
  defaultFolder: readDefaultFolder(),
  scratch: null,
  pinned: readSet("anysola.pinned"),
  // Folded projects by folder key; older entries were stored as spelled.
  collapsed: new Set([...readSet("anysola.collapsed")].map(folderKey)),
  git: null,
  comments: [],
}

const listeners = new Set<() => void>()

function set(patch: Partial<State> | ((s: State) => Partial<State>)) {
  state = { ...state, ...(typeof patch === "function" ? patch(state) : patch) }
  listeners.forEach((l) => l())
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
  if (showing(target)) setOpen({ status: { kind: "failed", error: message(e) } }, target)
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
  if (state.update?.installing) return
  try {
    set({ update: await bridge.updateCheck() })
  } catch {
    // Offline or no release yet: the next check tries again.
  }
}

export async function installUpdate() {
  const update = state.update
  if (!update?.available) return
  if (update.manual) {
    const page = update.manual
    void bridge.openUrl(page).catch(() => {
      if (state.update) set({ update: { ...state.update, error: `Your browser didn’t open. The download is at ${page}` } })
    })
    return
  }
  set({ update: { ...update, installing: true, error: undefined } })
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
  void checkUpdate()
  setInterval(() => void checkUpdate(), UPDATE_EVERY)
  try {
    const { signedIn } = await bridge.authStatus()
    set({ auth: signedIn ? "signed-in" : "signed-out" })
    // Names the app's folder "No project" in the sidebar.
    void bridge
      .scratchFolder()
      .then((scratch) => set({ scratch }))
      .catch(() => {})
    if (signedIn) {
      void loadAccount()
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

async function loadAccount() {
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
    set({ account })
  } catch {
    // The plan and windows stay as last read.
  }
}

export async function signOut() {
  try {
    const { signedIn } = await bridge.logout()
    if (signedIn) {
      set({ notice: "ANYSOLA_TOKEN is set, so this computer stays signed in." })
      return
    }
    set({ auth: "signed-out", account: null, sessions: [], open: null, permissions: [], questions: [] })
  } catch (e) {
    set({ notice: message(e) })
  }
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

async function loadSessions(openFirst = false) {
  const sessions = await bridge.sessions()
  set({ sessions })
  if (openFirst && !state.open && sessions[0]) await openSession(sessions[0])
}

export async function openSession(row: SessionRow) {
  void refreshGit(row.directory)
  set({
    comments: [],
    open: {
      key: `v${++views}`,
      directory: row.directory,
      sessionId: row.id,
      title: row.title,
      items: [],
      live: [],
      info: null,
      status: { kind: "idle" },
      loading: true,
      revert: null,
      todos: [],
      lastAgent: null,
      fill: null,
    },
  })
  const n = ++reads
  try {
    const opened = await bridge.openSession(row.directory, row.id)
    // The composer's mode is the window's, the floor the session's: a turn still running here
    // would otherwise go on under the mode it started with while the composer shows another.
    void bridge.setMode(row.directory, row.id, state.mode).catch(() => {})
    set((s) => ({ busy: toggle(s.busy, row.id, opened.busy) }))
    setOpen(
      (o) => ({
        ...(latest(n) ? viewPatch(opened.view, o) : {}),
        info: opened.info,
        title: opened.session.title,
        loading: false,
        status: o.status.kind === "idle" && opened.busy ? { kind: "working" } : o.status,
      }),
      row.id
    )
  } catch (e) {
    setOpen({ loading: false, status: { kind: "failed", error: message(e) } }, row.id)
    // A session deleted elsewhere ("This session no longer exists."): its row goes too.
    void loadSessions().catch(() => {})
  }
}

/** A new session: in `directory`, else the default folder, else one the user picks. */
export async function newSession(directory?: string) {
  if (!directory) directory = state.defaultFolder ?? (await pickFolder()) ?? undefined
  if (!directory) return
  void refreshGit(directory)
  const view = { key: `v${++views}` }
  set({
    comments: [],
    open: {
      ...view,
      directory,
      sessionId: null,
      title: "New session",
      items: [],
      live: [],
      info: null,
      status: { kind: "idle" },
      loading: false,
      revert: null,
      todos: [],
      lastAgent: null,
      fill: null,
    },
  })
  try {
    const info = await bridge.folderInfo(directory)
    setOpen({ info }, view)
  } catch (e) {
    setOpen({ status: { kind: "failed", error: message(e) } }, view)
  }
}

/** Opens the folder picker where the default folder is. */
export async function pickFolder() {
  return bridge.pickFolder(state.defaultFolder)
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
  try {
    if (directory) localStorage.setItem("anysola.defaultFolder", directory)
    else localStorage.removeItem("anysola.defaultFolder")
  } catch {
    // Kept for this window only.
  }
  set({ defaultFolder: directory })
}

/** A view read from the database; text it now holds stops streaming separately. */
function viewPatch(view: View, o: Open): Partial<Open> {
  const items = view.items
  return {
    items,
    revert: view.revert,
    todos: view.todos,
    lastAgent: view.agent,
    live: o.live.filter((l) => !items.some((i) => "text" in i && i.text === l.text)),
  }
}

/** Each read of the open view is numbered; an answer older than one already shown is dropped. */
let reads = 0
let shown = 0

function latest(n: number) {
  if (n < shown) return false
  shown = n
  return true
}

async function refresh(sessionId: string) {
  const open = state.open
  if (!open || open.sessionId !== sessionId) return
  const n = ++reads
  try {
    const view = await bridge.transcript(open.directory, sessionId)
    if (latest(n)) setOpen((o) => viewPatch(view, o), sessionId)
  } catch {
    // The next event reads again.
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

let fills = 0

/** The last message sent, until its turn starts: a send that fails or is refused before then
 * puts it back in the composer instead of losing what was typed. */
let unsent: { target: string | { key: string }; text: string; files: Attachment[]; comments: Comment[] } | null =
  null

function putBack(target: string | { key: string }) {
  const u = unsent
  if (!u || !same(u.target, target)) return
  unsent = null
  if (!showing(target)) return
  // Only into an empty composer: what the user typed since stays.
  setOpen({ fill: { text: u.text, files: u.files, n: ++fills, replacing: "" } }, target)
  if (state.comments.length === 0) set({ comments: u.comments })
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

/** A new session's view takes the id its first prompt created, and any end that came first. */
function adopt(target: string | { key: string }, sessionId: string) {
  setOpen({ sessionId }, target)
  if (unsent && same(unsent.target, target)) unsent = { ...unsent, target: sessionId }
  void loadSessions()
  const e = early.get(sessionId)
  early.delete(sessionId)
  if (e && e.type !== "started") onEvent(e)
  return sessionId
}

/** Busy and status read back from the core, for when the events that would say so were lost
 * or never sent. A start or end that came meanwhile says more, so then only the view is taken. */
async function reconcile(directory: string, sessionId: string) {
  const since = activity.get(sessionId)
  const n = ++reads
  try {
    const opened = await bridge.openSession(directory, sessionId)
    const current = activity.get(sessionId) === since
    if (current) set((s) => ({ busy: toggle(s.busy, sessionId, opened.busy) }))
    setOpen((o) => {
      const shown = o.status.kind === "working" || o.status.kind === "retrying"
      return {
        ...(latest(n) ? viewPatch(opened.view, o) : {}),
        ...(current && shown && !opened.busy ? { live: [], status: { kind: "idle" } as Status } : {}),
        ...(current && !shown && opened.busy ? { status: { kind: "working" } as Status } : {}),
      }
    }, sessionId)
  } catch {
    // The next event says more.
  }
}

export async function send(text: string, files: Attachment[] = [], skills: string[] = []) {
  const open = state.open
  if (!open || (!text.trim() && files.length === 0 && state.comments.length === 0)) return
  let target = whose(open)
  unsent = { target, text, files, comments: state.comments }
  text = withComments(text)
  set({ comments: [] })
  // The prompt makes a staged rewind final.
  setOpen({ status: { kind: "working" }, revert: null })
  const draft = !open.sessionId
  if (open.sessionId) touch(open.sessionId)
  else creating++
  try {
    const { sessionId, queued } = await bridge.prompt(open.directory, {
      sessionId: open.sessionId,
      text,
      mode: state.mode,
      // A new session takes the model picked before it existed.
      model: draft ? (open.info?.modelId ?? null) : null,
      variant: draft ? (open.info?.variant ?? null) : null,
      agent: draft ? (open.info?.agent ?? null) : null,
      files,
      skills,
    })
    if (draft) target = adopt(target, sessionId)
    // A running turn took it: it is delivered, whatever that turn does next.
    if (queued && unsent && same(unsent.target, target)) unsent = null
    // Busy comes from `started`: the turn is spawned before this answer, and its end can
    // arrive first.
    await refresh(sessionId)
  } catch (e) {
    failed(target, e)
    putBack(target)
  } finally {
    if (draft && --creating === 0) early.clear()
  }
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
    setOpen((o) => ({ ...viewPatch(view, o), live: [], status: { kind: "idle" }, fill: { text, n: ++fills } }), sessionId)
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
    // The rewound message leaves the composer unless the user changed it.
    const replacing = open.fill?.text
    setOpen((o) => ({ ...viewPatch(view, o), fill: { text: "", n: ++fills, replacing } }), sessionId)
  } catch (e) {
    set({ notice: message(e) })
  }
}

/** Sends a message again from where it was, with new text, or as it was when `text` is null. */
export async function resend(messageId: string, text: string | null) {
  const open = state.open
  if (!open?.sessionId) return
  const sessionId = open.sessionId
  setOpen({ status: { kind: "working" }, live: [], revert: null }, sessionId)
  touch(sessionId)
  try {
    await bridge.resend(open.directory, sessionId, messageId, text, state.mode)
    await refresh(sessionId)
  } catch (e) {
    setOpen({ status: { kind: "failed", error: message(e) } }, sessionId)
  }
}

/** Sends the last message again. */
export async function retry() {
  const open = state.open
  const last = open && lastUser(open)
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
  unsent = { target, text: args ? `/${name} ${args}` : `/${name}`, files: [], comments: [] }
  setOpen({ status: { kind: "working" }, revert: null })
  const draft = !open.sessionId
  if (open.sessionId) touch(open.sessionId)
  else creating++
  try {
    const { sessionId } = await bridge.runCommand(open.directory, {
      sessionId: open.sessionId,
      name,
      arguments: args,
      mode: state.mode,
      model: draft ? (open.info?.modelId ?? null) : null,
      variant: draft ? (open.info?.variant ?? null) : null,
      agent: draft ? (open.info?.agent ?? null) : null,
    })
    if (draft) target = adopt(target, sessionId)
    // Busy comes from `started`: the turn is spawned before this answer, and its end can
    // arrive first.
    await refresh(sessionId)
  } catch (e) {
    failed(target, e)
    putBack(target)
  } finally {
    if (draft && --creating === 0) early.clear()
  }
}

export async function compact() {
  const open = state.open
  if (!open?.sessionId) {
    set({ notice: "There’s nothing to compact yet." })
    return
  }
  const sessionId = open.sessionId
  setOpen({ status: { kind: "working" } }, sessionId)
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
  await chooseAgent("build")
  await send("Go ahead with the plan.")
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
    set((s) => (s.peek?.sessionId === peek.sessionId ? { peek: { ...s.peek, items, loading: false } } : {}))
  } catch (e) {
    set({ peek: null, notice: message(e) })
  }
}

export async function answer(q: Asked, answers: string[][] | null) {
  set((s) => ({ questions: s.questions.filter((x) => x.requestId !== q.requestId) }))
  try {
    await bridge.replyQuestion(q.directory, q.requestId, answers)
  } catch (e) {
    set({ notice: message(e) })
  }
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
  for (const row of rows) {
    try {
      await bridge.deleteSession(row.directory, row.id)
      gone.add(row.id)
    } catch {
      kept.push(row.title)
    }
  }
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

/** The session's net file changes (null outside git: the window sums the edits instead). */
export async function readChanges(): Promise<FileDiff[] | null> {
  const open = state.open
  if (!open?.sessionId) return null
  try {
    return await bridge.changes(open.directory, open.sessionId)
  } catch {
    return null
  }
}

/** Puts one file back as it was before the session changed it. */
export async function restoreFile(file: string) {
  const open = state.open
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
  set((s) => ({ comments: [...s.comments, { ...c, id: ++comments }] }))
}

export function removeComment(id: number) {
  set((s) => ({ comments: s.comments.filter((c) => c.id !== id) }))
}

/** The comments as text the model reads after the message. */
function withComments(text: string) {
  if (state.comments.length === 0) return text
  const list = state.comments
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
    set({ git: { ...status, directory } })
  } catch {
    set({ git: null })
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

export const gitCommit = (text: string) => gitStep((d) => bridge.gitCommit(d, text))
export const gitPush = () => gitStep((d) => bridge.gitPush(d))
export const gitBranch = (name: string) => gitStep((d) => bridge.gitBranch(d, name))

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

export function stop() {
  const open = state.open
  if (!open?.sessionId) return
  const { directory, sessionId } = open
  const since = activity.get(sessionId)
  bridge.interrupt(directory, sessionId).then(
    // Stopping a session with nothing running sends no event; "Working" would stay.
    () =>
      setTimeout(() => {
        if (activity.get(sessionId) === since) void reconcile(directory, sessionId)
      }, STOP_GRACE),
    (e) => set({ notice: message(e) })
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
  if (open?.sessionId)
    void bridge.setMode(open.directory, open.sessionId, mode).catch((e) => set({ notice: message(e) }))
}

export async function reply(p: Permission, answer: PermissionReply) {
  set((s) => ({ permissions: s.permissions.filter((x) => x.requestId !== p.requestId) }))
  try {
    await bridge.replyPermission(p.directory, p.requestId, answer)
  } catch (e) {
    set({ notice: message(e) })
  }
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

function directoryOf(sessionId: string) {
  if (state.open?.sessionId === sessionId) return state.open.directory
  return state.sessions.find((s) => s.id === sessionId)?.directory ?? ""
}

function onEvent(e: CoreEvent) {
  if (e.type === "notice") {
    set({ notice: e.message })
    return
  }
  if (e.type === "lagged") {
    // Lost events may have started or ended turns: read busy and status back, not only the text.
    const open = state.open
    if (open?.sessionId && sameFolder(open.directory, e.directory)) void reconcile(open.directory, open.sessionId)
    void loadSessions().catch(() => {})
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
    case "started":
      if (unsent?.target === id) unsent = null
      set((s) => ({ busy: toggle(s.busy, id, true) }))
      if (here) setOpen({ status: { kind: "working" } }, id)
      break
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
      if (here) void refresh(id)
      break
    case "retrying":
      if (here)
        setOpen(
          { status: { kind: "retrying", attempt: e.attempt, until: Date.now() + e.delayMs, error: e.error } },
          id
        )
      break
    case "permission": {
      // The mode is the session's floor in the core; an ask that reaches here is one a rule
      // (`read *.env`, the user's own) wants asked, whatever the mode.
      const p: Permission = { ...e, rootId: e.rootId ?? id, directory: e.directory || directoryOf(id) }
      set((s) => ({ permissions: [...s.permissions.filter((x) => x.requestId !== e.requestId), p] }))
      break
    }
    case "question":
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
      set((s) => ({ permissions: s.permissions.filter((x) => x.requestId !== e.requestId) }))
      break
    case "title":
      set((s) => ({ sessions: s.sessions.map((r) => (r.id === id ? { ...r, title: e.title } : r)) }))
      if (here) setOpen({ title: e.title }, id)
      break
    case "done":
    case "failed":
    case "interrupted":
      if (unsent?.target === id) unsent = null
      set((s) => ({
        busy: toggle(s.busy, id, false),
        permissions: s.permissions.filter((p) => p.sessionId !== id && p.rootId !== id),
        questions: s.questions.filter((q) => q.sessionId !== id && q.rootId !== id),
      }))
      void loadAccount()
      if (here) {
        setOpen(
          {
            live: [],
            status:
              e.type === "done"
                ? { kind: "idle" }
                : e.type === "failed"
                  ? { kind: "failed", error: e.error, retry: true }
                  : { kind: "stopped", reason: e.reason },
          },
          id
        )
        void refresh(id)
        // The turn may have changed files: the header's git count follows.
        void refreshGit()
      }
      void loadSessions()
      break
    case "refused":
      set((s) => ({ busy: toggle(s.busy, id, false) }))
      if (here) {
        setOpen({ live: [], status: { kind: "failed", error: e.error } }, id)
        putBack(id)
        void refresh(id)
      }
      break
    case "settled":
      if (unsent?.target === id) unsent = null
      set((s) => ({ busy: toggle(s.busy, id, false) }))
      if (here) {
        setOpen((o) => (o.status.kind === "working" ? { live: [], status: { kind: "idle" } } : {}), id)
        void refresh(id)
        void refreshGit()
      }
      void loadSessions()
      break
    case "usage":
      if (here && e.fiveHour !== null) setOpen((o) => ({ info: o.info && { ...o.info, fiveHour: e.fiveHour } }), id)
      break
    case "created":
      void loadSessions()
      break
  }
}

export function dismissNotice() {
  set({ notice: null })
}
