/**
 * The app's preferences: read from the Rust side at start, written back whole on every change,
 * and applied to the document (theme, text size, column width, code wrapping, motion). The
 * appearance part is also kept in localStorage and applied before React mounts, so a window
 * opens in its theme without a flash.
 */
import { useSyncExternalStore } from "react"

import type { Bridge, Prefs } from "./bridge"

export const DEFAULT_PREFS: Prefs = {
  general: { defaultFolder: null, startMode: "last", checkUpdates: true, editor: null },
  appearance: { theme: "dark", textSize: "default", width: "default", wrapCode: false, reduceMotion: false },
  conversation: { changes: "closed", commands: "closed", lookups: "grouped", shorten: true, send: "enter" },
  notifications: { enabled: true, approval: true, question: true, finished: true, failed: true },
  discord: { enabled: false, project: true, session: false, model: true, elapsed: true, applicationId: null },
  sources: { claudeSkills: true },
}

/** Whether Light and System are offered; a stored one shows as dark while this is off. */
export const LIGHT_THEME = true

const APPEARANCE_KEY = "anysola.appearance"

let prefs: Prefs = withDefaults(readCached())
/** False until the Rust side's copy is read: nothing is written over a file not yet read. */
let loaded = false
const listeners = new Set<() => void>()

function emit() {
  listeners.forEach((l) => l())
}

/** Fills in what an older file (or the cache) lacks, so a missing group never breaks a read. */
function withDefaults(p: Partial<Prefs> | null | undefined): Prefs {
  const out = structuredClone(DEFAULT_PREFS)
  if (!p || typeof p !== "object") return out
  for (const group of Object.keys(out) as (keyof Prefs)[]) {
    const from = (p as Record<string, unknown>)[group]
    if (from && typeof from === "object") Object.assign(out[group], from)
  }
  return out
}

function readCached(): Partial<Prefs> | null {
  try {
    const raw = localStorage.getItem(APPEARANCE_KEY)
    return raw ? { appearance: JSON.parse(raw) } : null
  } catch {
    return null
  }
}

export function getPrefs(): Prefs {
  return prefs
}

export function usePrefs<T>(select: (p: Prefs) => T): T {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l)
      return () => listeners.delete(l)
    },
    () => select(prefs)
  )
}

// ---------------------------------------------------------------------------
// The document
// ---------------------------------------------------------------------------

const systemDark = typeof matchMedia === "function" ? matchMedia("(prefers-color-scheme: dark)") : null
const systemCalm = typeof matchMedia === "function" ? matchMedia("(prefers-reduced-motion: reduce)") : null

export function resolvedTheme(theme: Prefs["appearance"]["theme"]): "dark" | "light" {
  if (!LIGHT_THEME || theme === "dark") return "dark"
  if (theme === "light") return "light"
  return systemDark && !systemDark.matches ? "light" : "dark"
}

const ROOT_SIZE = { small: "15px", default: "16px", large: "17px" } as const
const WIDTH = { narrow: "44rem", default: "50rem", wide: "62rem" } as const

/** Puts the appearance on `<html>`: theme class, root size, column width, wrapping, motion. */
export function applyAppearance(a: Prefs["appearance"] = prefs.appearance) {
  const root = document.documentElement
  const theme = resolvedTheme(a.theme)
  root.classList.toggle("dark", theme === "dark")
  root.classList.toggle("light", theme === "light")
  root.dataset.theme = theme
  root.style.colorScheme = theme
  root.style.fontSize = ROOT_SIZE[a.textSize] ?? ROOT_SIZE.default
  root.style.setProperty("--conversation-width", WIDTH[a.width] ?? WIDTH.default)
  root.toggleAttribute("data-wrap-code", a.wrapCode)
  root.toggleAttribute("data-reduce-motion", a.reduceMotion || !!systemCalm?.matches)
}

/** Before React mounts: the appearance last applied, and live updates from the system. */
export function applyEarly() {
  const preview = new URLSearchParams(location.search).get("theme")
  if (preview === "light" || preview === "dark" || preview === "system")
    prefs = { ...prefs, appearance: { ...prefs.appearance, theme: preview } }
  applyAppearance()
  systemDark?.addEventListener("change", () => applyAppearance())
  systemCalm?.addEventListener("change", () => applyAppearance())
}

function cacheAppearance() {
  try {
    localStorage.setItem(APPEARANCE_KEY, JSON.stringify(prefs.appearance))
  } catch {
    // The next start applies the default first, then the file's.
  }
}

// ---------------------------------------------------------------------------
// Reading and writing
// ---------------------------------------------------------------------------

let bridge: Bridge | null = null

/** Reads the preferences. A failed read keeps the defaults and writes nothing until a read works. */
export async function loadPrefs(b: Bridge): Promise<void> {
  bridge = b
  try {
    const read = withDefaults(await b.prefsGet())
    const keepPreview = new URLSearchParams(location.search).get("theme")
    prefs = keepPreview ? { ...read, appearance: { ...read.appearance, theme: prefs.appearance.theme } } : read
    loaded = true
    applyAppearance()
    cacheAppearance()
    emit()
    migrate()
  } catch {
    // Defaults stand; Settings tries again when it opens.
  }
}

/** Once: the default folder used to live in localStorage. */
function migrate() {
  let folder: string | null = null
  try {
    folder = localStorage.getItem("anysola.defaultFolder")
  } catch {
    return
  }
  if (folder === null) return
  const done = () => {
    try {
      localStorage.removeItem("anysola.defaultFolder")
    } catch {
      // Read again next start; the folder is set by then.
    }
  }
  if (prefs.general.defaultFolder || !folder) return done()
  // Removed only once the file holds it; a failed write tries again next start.
  void savePrefs({ ...prefs, general: { ...prefs.general, defaultFolder: folder } }).then((ok) => ok && done())
}

/** Writes in order, one at a time; only the newest answer is taken. */
let writing: Promise<unknown> = Promise.resolve()
let writes = 0

type Failure = (message: string) => void
let onFailure: Failure = () => {}

/** Where a failed write is reported (the store's notice). */
export function onPrefsFailure(f: Failure) {
  onFailure = f
}

async function savePrefs(next: Prefs): Promise<boolean> {
  const before = prefs
  prefs = next
  applyAppearance()
  cacheAppearance()
  emit()
  const b = bridge
  if (!b) return true
  if (!loaded) {
    // The file wasn't read: read it first, so a write never puts defaults over it.
    try {
      const read = withDefaults(await b.prefsGet())
      loaded = true
      // The change the user made stays on top of what the file holds.
      prefs = mergeChange(read, before, next)
    } catch (e) {
      revert(before, e)
      return false
    }
  }
  const n = ++writes
  const body = prefs
  const result = (writing = writing.then(
    () => b.prefsSet(body),
    () => b.prefsSet(body)
  ))
  try {
    const saved = await result
    if (n === writes) {
      prefs = withDefaults(saved)
      applyAppearance()
      cacheAppearance()
      emit()
    }
    return true
  } catch (e) {
    if (n === writes) revert(before, e)
    return false
  }
}

/** The groups the change touched, taken from `next`; the rest from what was read. */
function mergeChange(read: Prefs, before: Prefs, next: Prefs): Prefs {
  const out = structuredClone(read)
  for (const group of Object.keys(next) as (keyof Prefs)[])
    for (const [k, v] of Object.entries(next[group]))
      if ((before[group] as Record<string, unknown>)[k] !== v) (out[group] as Record<string, unknown>)[k] = v
  return out
}

function revert(before: Prefs, e: unknown) {
  prefs = before
  applyAppearance()
  cacheAppearance()
  emit()
  const why = typeof e === "string" ? e : e instanceof Error ? e.message : "The file couldn’t be written."
  onFailure(`That setting wasn’t saved. ${why}`)
}

/** Changes one group's fields; applied at once, put back if the write fails. */
export function setPref<G extends keyof Prefs>(group: G, patch: Partial<Prefs[G]>): Promise<boolean> {
  return savePrefs({ ...prefs, [group]: { ...prefs[group], ...patch } })
}

/** Reads the file again (Settings opening after a failed first read). */
export async function reloadPrefs(): Promise<void> {
  if (!bridge || loaded) return
  await loadPrefs(bridge)
}

// ---------------------------------------------------------------------------
// The send key
// ---------------------------------------------------------------------------

/** Whether this key sends (or allows) under the send preference; Shift Enter is always a new line. */
export function isSendKey(e: { key: string; shiftKey: boolean; ctrlKey: boolean; metaKey: boolean }, send = prefs.conversation.send) {
  if (e.key !== "Enter" || e.shiftKey) return false
  return send === "enter" || e.ctrlKey || e.metaKey
}

export function sendKeyLabel(send = prefs.conversation.send) {
  return send === "enter" ? "Enter" : "Ctrl Enter"
}
