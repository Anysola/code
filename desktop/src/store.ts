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
  /** Text for the composer to take (a rewound message); `n` tells refills apart. With
   * `replacing`, only while the composer still holds that text. */
  fill: { text: string; n: number; replacing?: string } | null
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
  /** The app's folder for sessions without a project, once known. */
  scratch: string | null
}

/** Edits the "Edits" mode lets through without asking. */
const EDIT_ACTIONS = new Set(["edit", "write", "patch"])

function readMode(): Mode {
  try {
    const m = localStorage.getItem("anysola.mode")
    if (m === "ask" || m === "edits" || m === "auto") return m
  } catch {
    // Storage can be unavailable; the default stands.
  }
  return "edits"
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
}

const listeners = new Set<() => void>()

function set(patch: Partial<State> | ((s: State) => Partial<State>)) {
  state = { ...state, ...(typeof patch === "function" ? patch(state) : patch) }
  listeners.forEach((l) => l())
}

function setOpen(patch: Partial<Open> | ((o: Open) => Partial<Open>), sessionId?: string | null) {
  set((s) => {
    if (!s.open) return {}
    if (sessionId !== undefined && s.open.sessionId !== sessionId) return {}
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
    void bridge.openUrl(update.manual)
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
    void bridge.openUrl(login.url)
    await bridge.loginWait()
    set({ auth: "signed-in", login: null })
    void loadAccount()
    await loadSessions(true)
  } catch (e) {
    set({ login: null, loginError: message(e) })
  }
}

export function openLoginPage() {
  if (state.login) void bridge.openUrl(state.login.url)
}

async function loadAccount() {
  try {
    const account = await bridge.account()
    if (account.expired) {
      set({ auth: "signed-out", account: null, loginError: "Your sign-in expired. Sign in again." })
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
  set({
    open: {
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
  try {
    const opened = await bridge.openSession(row.directory, row.id)
    set((s) => ({ busy: toggle(s.busy, row.id, opened.busy) }))
    setOpen(
      (o) => ({
        ...viewPatch(opened.view, o),
        info: opened.info,
        title: opened.session.title,
        loading: false,
        status: o.status.kind === "idle" && opened.busy ? { kind: "working" } : o.status,
      }),
      row.id
    )
  } catch (e) {
    setOpen({ loading: false, status: { kind: "failed", error: message(e) } }, row.id)
  }
}

/** A new session: in `directory`, else the default folder, else one the user picks. */
export async function newSession(directory?: string) {
  if (!directory) directory = state.defaultFolder ?? (await pickFolder()) ?? undefined
  if (!directory) return
  set({
    open: {
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
    setOpen({ info }, null)
  } catch (e) {
    setOpen({ status: { kind: "failed", error: message(e) } }, null)
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

async function refresh(sessionId: string) {
  const open = state.open
  if (!open || open.sessionId !== sessionId) return
  try {
    const view = await bridge.transcript(open.directory, sessionId)
    setOpen((o) => viewPatch(view, o), sessionId)
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

export async function send(text: string, files: Attachment[] = [], skills: string[] = []) {
  const open = state.open
  if (!open || (!text.trim() && files.length === 0)) return
  // The prompt makes a staged rewind final.
  setOpen({ status: { kind: "working" }, revert: null })
  try {
    const draft = !open.sessionId
    const { sessionId } = await bridge.prompt(open.directory, {
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
    if (!open.sessionId) {
      setOpen({ sessionId }, null)
      void loadSessions()
    }
    set((s) => ({ busy: toggle(s.busy, sessionId, true) }))
    await refresh(sessionId)
  } catch (e) {
    setOpen({ status: { kind: "failed", error: message(e) } })
  }
}

export async function chooseModel(modelId: string, variant: string | null) {
  const open = state.open
  if (!open?.info) return
  if (!open.sessionId) {
    const model = open.info.models.find((m) => m.id === modelId)
    setOpen({ info: { ...open.info, modelId, model: model?.name ?? modelId, variant } }, null)
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

let fills = 0

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
  try {
    const { text, view } = await bridge.revert(open.directory, sessionId, messageId)
    setOpen((o) => ({ ...viewPatch(view, o), live: [], status: { kind: "idle" }, fill: { text, n: ++fills } }), sessionId)
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
  try {
    const view = await bridge.unrevert(open.directory, sessionId)
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
  try {
    await bridge.resend(open.directory, sessionId, messageId, text, state.mode)
    set((s) => ({ busy: toggle(s.busy, sessionId, true) }))
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
  setOpen({ status: { kind: "working" }, revert: null })
  try {
    const draft = !open.sessionId
    const { sessionId } = await bridge.runCommand(open.directory, {
      sessionId: open.sessionId,
      name,
      arguments: args,
      mode: state.mode,
      model: draft ? (open.info?.modelId ?? null) : null,
      variant: draft ? (open.info?.variant ?? null) : null,
      agent: draft ? (open.info?.agent ?? null) : null,
    })
    if (draft) {
      setOpen({ sessionId }, null)
      void loadSessions()
    }
    set((s) => ({ busy: toggle(s.busy, sessionId, true) }))
    await refresh(sessionId)
  } catch (e) {
    setOpen({ status: { kind: "failed", error: message(e) } })
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
    setOpen({ info: { ...open.info, agent } }, null)
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
  try {
    await bridge.deleteSession(row.directory, row.id)
    if (state.open?.sessionId === row.id) set({ open: null })
    set((s) => ({ sessions: s.sessions.filter((r) => r.id !== row.id) }))
  } catch (e) {
    set({ notice: message(e) })
  }
}

export function stop() {
  const open = state.open
  if (open?.sessionId) void bridge.interrupt(open.directory, open.sessionId)
}

export function setMode(mode: Mode) {
  try {
    localStorage.setItem("anysola.mode", mode)
  } catch {
    // Kept for this window only.
  }
  set({ mode })
  // The open session's floor follows at once; a running turn picks it up at its next step.
  const open = state.open
  if (open?.sessionId)
    void bridge.setMode(open.directory, open.sessionId, mode).catch((e) => set({ notice: message(e) }))
  // Asks already waiting that the new mode lets through.
  for (const p of state.permissions) if (allowedByMode(p)) void reply(p, "once")
}

function allowedByMode(p: Permission) {
  return state.mode === "auto" || (state.mode === "edits" && EDIT_ACTIONS.has(p.action))
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
    if (state.open?.sessionId && state.open.directory === e.directory) void refresh(state.open.sessionId)
    return
  }
  const id = e.sessionId
  const here = state.open?.sessionId === id
  if (state.peek?.sessionId === id && (e.type === "changed" || e.type === "done" || e.type === "failed"))
    void refreshPeek()
  switch (e.type) {
    case "started":
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
      const p: Permission = { ...e, rootId: e.rootId ?? id, directory: e.directory || directoryOf(id) }
      if (allowedByMode(p)) {
        void reply(p, "once")
        break
      }
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
      }
      void loadSessions()
      break
    case "refused":
      set((s) => ({ busy: toggle(s.busy, id, false) }))
      if (here) {
        setOpen({ live: [], status: { kind: "failed", error: e.error } }, id)
        void refresh(id)
      }
      break
    case "settled":
      set((s) => ({ busy: toggle(s.busy, id, false) }))
      if (here) {
        setOpen((o) => (o.status.kind === "working" ? { live: [], status: { kind: "idle" } } : {}), id)
        void refresh(id)
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
