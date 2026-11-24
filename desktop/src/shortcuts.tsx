/**
 * The keyboard shortcuts: one list that the shortcuts sheet (Ctrl /) and Settings › Keyboard
 * both show, and the window-wide keys themselves.
 */
import { useEffect, useRef } from "react"

import type { SessionRow } from "./bridge"
import { setReviewing, showPalette, showShortcuts, toggleSidebar, useChrome } from "./chrome"
import { usePrefs } from "./prefs"
import { closeSettings, isRunning, newSession, openSession, openSettings, stop, useStore } from "./store"
import { btn, folderKey, Kbd, useEscape } from "./ui"

export type Shortcut = { label: string; keys: string[] }

/** Every shortcut, grouped as the sheet and Settings show them. */
export function useShortcutList(): { label: string; rows: Shortcut[] }[] {
  const send = usePrefs((p) => p.conversation.send)
  const sendKey = send === "enter" ? "Enter" : "Ctrl Enter"
  return [
    {
      label: "Anywhere",
      rows: [
        { label: "New session", keys: ["Ctrl N"] },
        { label: "New session in this folder", keys: ["Ctrl Shift N"] },
        { label: "Search sessions and commands", keys: ["Ctrl K"] },
        { label: "Previous or next session", keys: ["Alt ↑", "Alt ↓"] },
        { label: "Show or hide the sidebar", keys: ["Ctrl B"] },
        { label: "Go to the message box", keys: ["Ctrl L"] },
        { label: "Stop the running turn", keys: ["Ctrl ."] },
        { label: "Review changes", keys: ["Ctrl Shift D"] },
        { label: "Settings", keys: ["Ctrl ,"] },
        { label: "Keyboard shortcuts", keys: ["Ctrl /"] },
        { label: "Zoom in, out, or back", keys: ["Ctrl +", "Ctrl −", "Ctrl 0"] },
        { label: "Close a panel, menu or Settings", keys: ["Esc"] },
      ],
    },
    {
      label: "Message box",
      rows: [
        { label: "Send", keys: [sendKey] },
        { label: "New line", keys: send === "enter" ? ["Shift Enter"] : ["Enter", "Shift Enter"] },
        { label: "Allow the waiting request, with the box empty", keys: [sendKey] },
        { label: "Deny the waiting request", keys: ["Esc"] },
        { label: "Stop the turn", keys: ["Esc Esc"] },
        { label: "Commands and skills", keys: ["/"] },
        { label: "Files and skills", keys: ["@"] },
        { label: "Move in a list", keys: ["↑", "↓"] },
        { label: "Take the highlighted item", keys: ["Enter", "Tab"] },
      ],
    },
    {
      label: "Menus",
      rows: [
        { label: "Move between items", keys: ["↑", "↓", "Home", "End"] },
        { label: "Choose", keys: ["Enter"] },
        { label: "Close and go back", keys: ["Esc"] },
      ],
    },
    {
      label: "Sessions",
      rows: [
        { label: "Select one more", keys: ["Ctrl click"] },
        { label: "Select a range", keys: ["Shift click"] },
        { label: "Stop selecting", keys: ["Esc"] },
        { label: "Save a new name", keys: ["Enter"] },
      ],
    },
    {
      label: "Changes",
      rows: [
        { label: "Add a comment on a line", keys: ["Enter"] },
        { label: "Cancel a comment", keys: ["Esc"] },
      ],
    },
  ]
}

/** The key a control's tooltip names, from the same list. */
export const KEYS = {
  newSession: "Ctrl N",
  palette: "Ctrl K",
  sidebar: "Ctrl B",
  stop: "Ctrl .",
  review: "Ctrl Shift D",
  settings: "Ctrl ,",
  shortcuts: "Ctrl /",
} as const

/** Sessions in the sidebar's order: pinned first, then by project. */
export function sessionOrder(sessions: SessionRow[], pinned: Set<string>) {
  const pins = sessions.filter((s) => pinned.has(s.id))
  const by = new Map<string, SessionRow[]>()
  for (const s of sessions) {
    if (pinned.has(s.id)) continue
    const key = folderKey(s.directory)
    by.set(key, [...(by.get(key) ?? []), s])
  }
  return [...pins, ...[...by.values()].flat()]
}

/**
 * The key of a shortcut press. A layout that types an ASCII character there matches by that
 * character only (Turkish Q's Ctrl . is the key typing "."); a letter key typing a non-Latin
 * letter (Cyrillic, Greek…) falls back to its US letter. Punctuation never falls back: on
 * another layout that position types something else, and the shortcut would fire twice.
 */
export function shortcutKey(e: Pick<KeyboardEvent, "key" | "code">) {
  const key = e.key.toLowerCase()
  if (key.length === 1 && !/[\x00-\x7f]/.test(key) && /^Key[A-Z]$/.test(e.code)) return e.code.slice(3).toLowerCase()
  return key
}

/** The window-wide keys. Ctrl Alt is AltGr on Windows (typing @ or [), so it is never a shortcut. */
export function useShortcuts() {
  const sessions = useStore((s) => s.sessions)
  const pinned = useStore((s) => s.pinned)
  const open = useStore((s) => s.open)
  const busy = useStore((s) => s.busy)
  const deleting = useChrome((c) => c.deleting)
  // Signed out, at the fault screen or still starting, only the sheet opens: the rest would act
  // on sessions there are none of (and before the start, without the bridge).
  const ready = useStore((s) => s.auth === "signed-in" && !s.fault)
  // The sheet itself shows on the sign-in screen too, not while starting or at the fault screen.
  const sheet = useStore((s) => s.auth !== "checking" && !s.fault)
  const shown = useChrome((c) => c.shortcuts)
  const palette = useChrome((c) => c.palette)
  const now = useRef({ sessions, pinned, open, busy, deleting, ready, sheet, shown, palette })
  now.current = { sessions, pinned, open, busy, deleting, ready, sheet, shown, palette }
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented) return
      const ctrl = (e.ctrlKey || e.metaKey) && !e.altKey
      const key = shortcutKey(e)
      const inSettings = !!document.querySelector("[data-settings]")
      const { sessions, pinned, open, busy, deleting, ready, sheet } = now.current
      const run = (f: () => void) => {
        e.preventDefault()
        // The sheet is modal: another shortcut closes it rather than acting unseen behind it.
        if (key !== "/" && now.current.shown) showShortcuts(false)
        // So is the palette; Ctrl K itself toggles it.
        if (key !== "k" && now.current.palette) showPalette(false)
        f()
      }
      if (ctrl && key === "/") return sheet ? run(() => showShortcuts()) : undefined
      if (!ready) return
      if (ctrl && key === ",") return run(() => (inSettings ? closeSettings() : openSettings()))
      if (ctrl && key === "n")
        return run(() => {
          closeSettings()
          void newSession(e.shiftKey ? open?.directory : undefined)
        })
      if (ctrl && key === "k")
        return run(() => {
          if (now.current.palette) return showPalette(false)
          closeSettings()
          showPalette(true)
        })
      if (ctrl && key === "b") return run(() => toggleSidebar())
      if (ctrl && key === "l")
        return run(() => {
          closeSettings()
          requestAnimationFrame(() => document.querySelector<HTMLTextAreaElement>('textarea[aria-label="Message"]')?.focus())
        })
      if (inSettings) return
      // A new session's first turn has no id yet: Stop waits for it.
      if (ctrl && key === "." && open && (isRunning(open) || (!!open.sessionId && busy.has(open.sessionId)))) return run(stop)
      if (ctrl && e.shiftKey && key === "d" && open) return run(() => setReviewing(true))
      if (e.altKey && !e.ctrlKey && !e.metaKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
        const list = sessionOrder(
          sessions.filter((s) => !deleting?.ids.has(s.id)),
          pinned
        )
        if (list.length === 0) return
        const at = list.findIndex((s) => s.id === open?.sessionId)
        const next = e.key === "ArrowDown" ? (at < 0 ? 0 : at + 1) : at < 0 ? list.length - 1 : at - 1
        if (next < 0 || next >= list.length) return run(() => {})
        return run(() => void openSession(list[next]))
      }
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [])
}

// ---------------------------------------------------------------------------
// The sheet
// ---------------------------------------------------------------------------

export function ShortcutsSheet() {
  const shown = useChrome((c) => c.shortcuts)
  if (!shown) return null
  return <Sheet />
}

function Sheet() {
  const groups = useShortcutList()
  const panel = useRef<HTMLDivElement>(null)
  // Escape closes the sheet only, whatever is open under it.
  useEscape(panel, () => showShortcuts(false))
  useEffect(() => {
    const back = document.activeElement instanceof HTMLElement ? document.activeElement : null
    panel.current?.focus()
    return () => {
      if (back?.isConnected) back.focus({ preventScroll: true })
    }
  }, [])
  return (
    <div className="fixed inset-0 z-40 flex items-center justify-center bg-scrim p-6" onClick={() => showShortcuts(false)}>
      <div
        ref={panel}
        role="dialog"
        aria-modal="true"
        aria-labelledby="shortcuts-title"
        tabIndex={-1}
        onClick={(e) => e.stopPropagation()}
        onKeyDown={(e) => {
          // Tab stays in the sheet.
          if (e.key !== "Tab") return
          const items = [...e.currentTarget.querySelectorAll<HTMLElement>("button, [tabindex='0']")]
          const first = items[0]
          const last = items[items.length - 1]
          if (!first) return
          if (e.shiftKey && (document.activeElement === first || document.activeElement === e.currentTarget)) {
            e.preventDefault()
            last.focus()
          } else if (!e.shiftKey && document.activeElement === last) {
            e.preventDefault()
            first.focus()
          }
        }}
        className="flex max-h-full w-full max-w-[52rem] flex-col overflow-hidden rounded-2xl bg-overlay outline-none"
      >
        <div className="flex h-12 shrink-0 items-center gap-3 pr-3 pl-5">
          <h2 id="shortcuts-title" className="text-sm font-medium">
            Keyboard shortcuts
          </h2>
          <button
            type="button"
            onClick={() => showShortcuts(false)}
            className={btn("ghost", "sm", "ml-auto gap-1.5 text-muted")}
          >
            Close
            <Kbd>Esc</Kbd>
          </button>
        </div>
        <div tabIndex={0} className="min-h-0 overflow-y-auto px-5 pb-5 outline-none focus-visible:status-focused-field">
          {/* Two columns where there's room; a group never splits. */}
          <div className="columns-1 gap-8 [@media(min-width:960px)]:columns-2">
            {groups.map((g) => (
              <section key={g.label} className="mb-5 break-inside-avoid">
                <h3 className="text-2xs flex h-7 items-center font-medium tracking-wide text-muted">{g.label}</h3>
                <dl className="flex flex-col">
                  {g.rows.map((r) => (
                    <div key={r.label} className="flex min-h-8 items-center gap-4 py-1">
                      <dt className="text-dense min-w-0 flex-1">{r.label}</dt>
                      <dd className="flex shrink-0 flex-wrap justify-end gap-1">
                        {r.keys.map((k) => (
                          <Kbd key={k}>{k}</Kbd>
                        ))}
                      </dd>
                    </div>
                  ))}
                </dl>
              </section>
            ))}
          </div>
        </div>
      </div>
    </div>
  )
}
