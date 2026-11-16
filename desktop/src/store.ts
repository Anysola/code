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
  type PermissionReply,
  type Question,
  type SessionRow,
  type UpdateCheck,
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
  | { kind: "failed"; error: string }
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
    },
  })
  try {
    const opened = await bridge.openSession(row.directory, row.id)
    set((s) => ({ busy: toggle(s.busy, row.id, opened.busy) }))
    setOpen(
      (o) => ({
        items: opened.items,
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

export async function newSession() {
  const directory = await bridge.pickFolder()
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
    },
  })
  try {
    const info = await bridge.folderInfo(directory)
    setOpen({ info }, null)
  } catch (e) {
    setOpen({ status: { kind: "failed", error: message(e) } }, null)
  }
}

async function refresh(sessionId: string) {
  const open = state.open
  if (!open || open.sessionId !== sessionId) return
  try {
    const items = await bridge.transcript(open.directory, sessionId)
    // Text the database now holds stops streaming separately.
    setOpen(
      (o) => ({ items, live: o.live.filter((l) => !items.some((i) => "text" in i && i.text === l.text)) }),
      sessionId
    )
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

export async function send(text: string, files: Attachment[] = []) {
  const open = state.open
  if (!open || (!text.trim() && files.length === 0)) return
  setOpen({ status: { kind: "working" } })
  try {
    const draft = !open.sessionId
    const { sessionId } = await bridge.prompt(open.directory, {
      sessionId: open.sessionId,
      text,
      mode: state.mode,
      // A new session takes the model picked before it existed.
      model: draft ? (open.info?.modelId ?? null) : null,
      variant: draft ? (open.info?.variant ?? null) : null,
      files,
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
    const items = await bridge.transcript(peek.directory, peek.sessionId)
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
                  ? { kind: "failed", error: e.error }
                  : { kind: "stopped", reason: e.reason },
          },
          id
        )
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
