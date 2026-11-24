/**
 * The window's own state around the conversation: the sidebar folded away, the shortcuts sheet,
 * the palette, Review, the budget editor, fork notes put away, the editors found on this
 * computer, the "updated" notice, and deletes that wait behind an Undo.
 */
import { useSyncExternalStore } from "react"

import type { Editor, SessionRow } from "./bridge"
import { getPrefs } from "./prefs"
import { onBeforeClose } from "./ui"
import { closeSession, currentBridge, deleteSessions, openSession } from "./store"

type Chrome = {
  /** The sidebar folded away (remembered). */
  sidebarHidden: boolean
  shortcuts: boolean
  /** The palette (Ctrl K): sessions and commands. */
  palette: boolean
  reviewing: boolean
  /** The open session's budget editor, under the header's budget. */
  budgeting: boolean
  /** Forks whose "Branched from" note was put away (remembered). */
  forkNotes: Set<string>
  /** Sessions deleted from the window but not yet from the computer. */
  deleting: { ids: Set<string>; rows: SessionRow[]; wasOpen: string | null } | null
  /** The editors found, the preferred first; null until read. */
  editors: Editor[] | null
  /** Set once after the app restarted on a newer version. */
  updated: string | null
  /** A short confirmation ("Path copied") that goes by itself. */
  flash: { text: string; n: number } | null
}

const HIDDEN_KEY = "anysola.sidebarHidden"
/** Deletes behind an Undo, written down the moment they start: a window that goes before their
 * time (Alt F4, a crash, the updater's restart) carries them out at the next start. */
const DELETES_KEY = "anysola.pendingDeletes"
const VERSION_KEY = "anysola.version"
const FORK_NOTES_KEY = "anysola.forkNotesDismissed"
/** Fork notes remembered as put away; the oldest go first. */
const FORK_NOTES_KEPT = 300

function readHidden() {
  try {
    return localStorage.getItem(HIDDEN_KEY) === "1"
  } catch {
    return false
  }
}

function readForkNotes(): Set<string> {
  try {
    const raw = JSON.parse(localStorage.getItem(FORK_NOTES_KEY) ?? "[]")
    return new Set(Array.isArray(raw) ? raw.filter((x) => typeof x === "string") : [])
  } catch {
    return new Set()
  }
}

let chrome: Chrome = {
  sidebarHidden: readHidden(),
  shortcuts: false,
  palette: false,
  reviewing: false,
  budgeting: false,
  forkNotes: readForkNotes(),
  deleting: null,
  editors: null,
  updated: null,
  flash: null,
}

const listeners = new Set<() => void>()

function set(patch: Partial<Chrome>) {
  chrome = { ...chrome, ...patch }
  listeners.forEach((l) => l())
}

export function useChrome<T>(select: (c: Chrome) => T): T {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l)
      return () => listeners.delete(l)
    },
    () => select(chrome)
  )
}

export function toggleSidebar(show?: boolean) {
  const hidden = show === undefined ? !chrome.sidebarHidden : !show
  try {
    localStorage.setItem(HIDDEN_KEY, hidden ? "1" : "0")
  } catch {
    // Kept for this window only.
  }
  set({ sidebarHidden: hidden })
}

export function showShortcuts(on = !chrome.shortcuts) {
  set({ shortcuts: on })
}

let flashes = 0

export function flash(text: string) {
  const n = ++flashes
  set({ flash: { text, n } })
  setTimeout(() => chrome.flash?.n === n && set({ flash: null }), 2500)
}

export function setReviewing(on: boolean) {
  if (chrome.reviewing !== on) set({ reviewing: on })
}

export function showPalette(on: boolean) {
  if (chrome.palette !== on) set({ palette: on })
}

export function editBudget(on: boolean) {
  if (chrome.budgeting !== on) set({ budgeting: on })
}

/** Puts a fork's "Branched from" note away for good. */
export function dismissForkNote(sessionId: string) {
  const ids = [...chrome.forkNotes, sessionId].slice(-FORK_NOTES_KEPT)
  try {
    localStorage.setItem(FORK_NOTES_KEY, JSON.stringify(ids))
  } catch {
    // Kept for this window only.
  }
  set({ forkNotes: new Set(ids) })
}

// ---------------------------------------------------------------------------
// Delete with Undo
// ---------------------------------------------------------------------------

/** How long a delete waits for Undo. */
const UNDO_FOR = 8000

let timer: ReturnType<typeof setTimeout> | undefined

function storedDeletes(): SessionRow[] {
  try {
    const raw = JSON.parse(localStorage.getItem(DELETES_KEY) ?? "[]")
    return Array.isArray(raw) ? raw.filter((r) => r && typeof r.id === "string" && typeof r.directory === "string") : []
  } catch {
    return []
  }
}

function storeDeletes(rows: SessionRow[]) {
  try {
    if (rows.length) localStorage.setItem(DELETES_KEY, JSON.stringify(rows))
    else localStorage.removeItem(DELETES_KEY)
  } catch {
    // Then only this window's run deletes them.
  }
}

function remember(rows: SessionRow[]) {
  const ids = new Set(rows.map((r) => r.id))
  storeDeletes([...storedDeletes().filter((r) => !ids.has(r.id)), ...rows])
}

function forget(rows: SessionRow[]) {
  const ids = new Set(rows.map((r) => r.id))
  storeDeletes(storedDeletes().filter((r) => !ids.has(r.id)))
}

let resumed = false

/** At start (once per window: a sign-in later mustn't cut an Undo short): deletes a window that
 * went too soon left undone. */
export async function resumeDeletes() {
  if (resumed) return
  resumed = true
  const rows = storedDeletes()
  if (rows.length === 0) return
  await deleteSessions(rows)
  forget(rows)
}

/**
 * Hides the sessions at once and deletes them for real after a few seconds, unless undone.
 * `openId`: the session open now, which closes if it is one of them (Undo opens it again).
 */
export function deleteLater(rows: SessionRow[], openId: string | null) {
  if (rows.length === 0) return
  // A delete still waiting goes through first: only the latest can be undone.
  void flushDeletes()
  const open = openId && rows.some((r) => r.id === openId) ? openId : null
  if (open) closeSession()
  remember(rows)
  set({ deleting: { ids: new Set(rows.map((r) => r.id)), rows, wasOpen: open } })
  timer = setTimeout(() => void flushDeletes(), UNDO_FOR)
}

export function undoDelete() {
  const d = chrome.deleting
  if (!d) return
  clearTimeout(timer)
  forget(d.rows)
  set({ deleting: null })
  const back = d.rows.find((r) => r.id === d.wasOpen)
  if (back) void openSession(back)
}

/** Deletes what waits now: when its time is up, before the app closes, or before another delete. */
export async function flushDeletes() {
  const d = chrome.deleting
  if (!d) return
  clearTimeout(timer)
  set({ deleting: null })
  // A running session is kept, and the store's notice says why; its row comes back with the list.
  await deleteSessions(d.rows)
  forget(d.rows)
}

if (typeof window !== "undefined") {
  // A closing window deletes what waited when it has the time; what it doesn't finish is
  // stored (see `remember`) and done at the next start.
  window.addEventListener("pagehide", () => void flushDeletes())
  onBeforeClose(flushDeletes)
}

// ---------------------------------------------------------------------------
// Editors
// ---------------------------------------------------------------------------

let reading: Promise<void> | null = null

/** Reads the editors once (and again when asked, after the preference changes). */
export function loadEditors(again = false) {
  if (reading && !again) return reading
  reading = currentBridge()
    .editors()
    .then(
      (editors) => set({ editors }),
      () => set({ editors: [] })
    )
  return reading
}

/** The editor files open in: the chosen one when found, else the first found; null when none. */
export function preferredEditor(editors: Editor[] | null): Editor | null {
  if (!editors || editors.length === 0) return null
  const chosen = getPrefs().general.editor
  return editors.find((e) => e.id === chosen) ?? editors[0]
}

// ---------------------------------------------------------------------------
// Updated to a new version
// ---------------------------------------------------------------------------

/** After a restart on a newer version, once: "Updated to X". The first run only remembers it. */
export async function checkUpdated() {
  let version: string
  try {
    version = (await currentBridge().about()).version
  } catch {
    return
  }
  let before: string | null = null
  try {
    before = localStorage.getItem(VERSION_KEY)
    localStorage.setItem(VERSION_KEY, version)
  } catch {
    return
  }
  if (before && before !== version && newer(version, before)) set({ updated: version })
}

export function dismissUpdated() {
  set({ updated: null })
}

/** "1.0.10" after "1.0.9"; anything unparsable counts as newer. */
export function newer(a: string, b: string) {
  const pa = a.split(/[.-]/).map(Number)
  const pb = b.split(/[.-]/).map(Number)
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const x = pa[i] ?? 0
    const y = pb[i] ?? 0
    if (Number.isNaN(x) || Number.isNaN(y)) return true
    if (x !== y) return x > y
  }
  return false
}

export const RELEASES = "https://github.com/Anysola/code/releases"
export const HELP = "https://www.anysola.com/help"
export const FEEDBACK = "https://account.anysola.com/support"
export const SIGN_UP = "https://www.anysola.com"
