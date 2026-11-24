/**
 * Embedded interactive terminal drawer at the bottom of a session: opens in the session's
 * working directory (its worktree if isolated, otherwise the main folder) and stays alive
 * while switching sessions so running dev servers or watchers are not lost.
 */
import "@xterm/xterm/css/xterm.css"

import { FitAddon } from "@xterm/addon-fit"
import { Terminal, type ITheme } from "@xterm/xterm"
import { RotateCw, Terminal as TerminalIcon, Trash2, X } from "lucide-react"
import { useEffect, useRef, useState } from "react"

import { setTerminalOpen, useChrome } from "./chrome"
import { KEYS } from "./shortcuts"
import { currentBridge, useWorkDir, useWorktree, type Open } from "./store"
import { btn, cn, folderName, Tooltip } from "./ui"

const HEIGHT_KEY = "anysola.terminalHeight"
const DEFAULT_HEIGHT = 240
const MIN_HEIGHT = 140

function readHeight(): number {
  try {
    const n = Number(localStorage.getItem(HEIGHT_KEY))
    return Number.isFinite(n) && n >= MIN_HEIGHT ? n : DEFAULT_HEIGHT
  } catch {
    return DEFAULT_HEIGHT
  }
}

function saveHeight(px: number) {
  try {
    localStorage.setItem(HEIGHT_KEY, String(Math.round(px)))
  } catch {
    // Kept for this window only.
  }
}

function isDark(): boolean {
  return document.documentElement.classList.contains("dark")
}

function xtermTheme(dark: boolean): ITheme {
  if (dark) {
    return {
      background: "#181613",
      foreground: "#f5f4f2",
      cursor: "#e7e4de",
      cursorAccent: "#181613",
      selectionBackground: "rgba(231, 228, 222, 0.22)",
      black: "#26231f",
      red: "#e06c75",
      green: "#8cc265",
      yellow: "#e5c07b",
      blue: "#61afef",
      magenta: "#c678dd",
      cyan: "#56b6c2",
      white: "#d7d3cc",
      brightBlack: "#6d6860",
      brightRed: "#ea868f",
      brightGreen: "#a5d484",
      brightYellow: "#efd299",
      brightBlue: "#7ec0f7",
      brightMagenta: "#d596e8",
      brightCyan: "#73c9d4",
      brightWhite: "#f8f7f5",
    }
  }
  return {
    background: "#f7f6f4",
    foreground: "#1c1a17",
    cursor: "#2b2824",
    cursorAccent: "#f7f6f4",
    selectionBackground: "rgba(28, 26, 23, 0.16)",
    black: "#2b2824",
    red: "#c23b45",
    green: "#2f7d32",
    yellow: "#9a6700",
    blue: "#1d63b8",
    magenta: "#8a3fa0",
    cyan: "#147a87",
    white: "#6c6760",
    brightBlack: "#7a756d",
    brightRed: "#d64d57",
    brightGreen: "#3c9440",
    brightYellow: "#b57b06",
    brightBlue: "#2b74cc",
    brightMagenta: "#a052b8",
    brightCyan: "#1d91a0",
    brightWhite: "#1c1a17",
  }
}

function decodeBase64(b64: string): Uint8Array {
  const bin = atob(b64)
  const out = new Uint8Array(bin.length)
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i)
  return out
}

// ---------------------------------------------------------------------------
// Live PTY instances keyed by working folder (kept across session switches)
// ---------------------------------------------------------------------------

type TermInstance = {
  key: string
  directory: string
  ptyId: string | null
  shell: string | null
  exited: { code: number | null } | null
  error: string | null
  term: Terminal
  fit: FitAddon
  host: HTMLDivElement | null
  starting: boolean
  listeners: Set<() => void>
}

const instances = new Map<string, TermInstance>()
const byPtyId = new Map<string, TermInstance>()

function notifyInstance(inst: TermInstance) {
  inst.listeners.forEach((fn) => fn())
}

/** Routed from `store.ts`'s single `core` event listener. */
export function handleTerminalEvent(event: { type: "terminalData"; id: string; data: string } | { type: "terminalExit"; id: string; code: number | null }) {
  const inst = byPtyId.get(event.id)
  if (!inst) return
  if (event.type === "terminalData") {
    inst.term.write(decodeBase64(event.data))
  } else {
    byPtyId.delete(event.id)
    inst.ptyId = null
    inst.exited = { code: event.code }
    const suffix = event.code === null ? "" : ` (exit ${event.code})`
    inst.term.writeln(`\r\n\x1b[2m[Process exited${suffix}]\x1b[0m`)
    notifyInstance(inst)
  }
}

function getOrCreate(directory: string): TermInstance {
  const key = directory.toLowerCase()
  const existing = instances.get(key)
  if (existing) return existing

  const term = new Terminal({
    cursorBlink: true,
    fontFamily: '"JetBrains Mono Variable", ui-monospace, monospace',
    fontSize: 12.5,
    lineHeight: 1.35,
    scrollback: 5000,
    theme: xtermTheme(isDark()),
    allowProposedApi: false,
  })
  const fit = new FitAddon()
  term.loadAddon(fit)

  const inst: TermInstance = {
    key,
    directory,
    ptyId: null,
    shell: null,
    exited: null,
    error: null,
    term,
    fit,
    host: null,
    starting: false,
    listeners: new Set(),
  }

  term.attachCustomKeyEventHandler((e) => {
    if (e.type !== "keydown") return true
    const ctrl = (e.ctrlKey || e.metaKey) && !e.altKey
    // Let Ctrl+` bubble to the window shortcut to toggle the drawer.
    if (ctrl && (e.key === "`" || e.code === "Backquote")) return false
    // Copy when text is selected (or Ctrl+Shift+C); otherwise send ^C to the PTY.
    if (ctrl && e.key.toLowerCase() === "c" && (e.shiftKey || term.hasSelection())) {
      const sel = term.getSelection()
      if (sel) void navigator.clipboard.writeText(sel)
      return false
    }
    if (ctrl && e.key.toLowerCase() === "v") {
      void navigator.clipboard.readText().then((text) => {
        if (text && inst.ptyId) void currentBridge().terminalWrite(inst.ptyId, text)
      })
      return false
    }
    return true
  })

  term.onData((data) => {
    if (inst.ptyId) void currentBridge().terminalWrite(inst.ptyId, data)
  })

  instances.set(key, inst)
  return inst
}

async function startPty(inst: TermInstance) {
  if (inst.starting || inst.ptyId) return
  inst.starting = true
  inst.error = null
  inst.exited = null
  notifyInstance(inst)
  try {
    const cols = Math.max(20, inst.term.cols || 80)
    const rows = Math.max(4, inst.term.rows || 14)
    const { id, shell } = await currentBridge().terminalOpen(inst.directory, cols, rows)
    inst.ptyId = id
    inst.shell = shell
    byPtyId.set(id, inst)
  } catch (e) {
    inst.error = e instanceof Error ? e.message : String(e)
  } finally {
    inst.starting = false
    notifyInstance(inst)
  }
}

async function restartPty(inst: TermInstance) {
  if (inst.ptyId) {
    const old = inst.ptyId
    byPtyId.delete(old)
    inst.ptyId = null
    await currentBridge().terminalClose(old).catch(() => {})
  }
  inst.term.reset()
  await startPty(inst)
  inst.term.focus()
}

// ---------------------------------------------------------------------------
// Drawer component
// ---------------------------------------------------------------------------

export function TerminalDrawer({ open }: { open: Open }) {
  const shown = useChrome((c) => c.terminalOpen)
  const directory = useWorkDir(open)
  const worktree = useWorktree(open.sessionId)
  const [height, setHeight] = useState(readHeight)
  const [, bump] = useState(0)
  const mountRef = useRef<HTMLDivElement>(null)
  const dragging = useRef(false)

  // Keep xterm theme synced with light/dark mode toggles.
  useEffect(() => {
    const apply = () => {
      const t = xtermTheme(isDark())
      instances.forEach((inst) => {
        inst.term.options.theme = t
      })
    }
    const obs = new MutationObserver(apply)
    obs.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] })
    return () => obs.disconnect()
  }, [])

  // Mount or re-attach the directory's terminal when the drawer is open.
  useEffect(() => {
    if (!shown) return
    const container = mountRef.current
    if (!container) return
    const inst = getOrCreate(directory)
    const sub = () => bump((n) => n + 1)
    inst.listeners.add(sub)

    container.innerHTML = ""
    if (!inst.host) {
      const host = document.createElement("div")
      host.className = "h-full w-full"
      inst.host = host
      container.appendChild(host)
      inst.term.open(host)
    } else {
      container.appendChild(inst.host)
    }

    const fitNow = () => {
      try {
        inst.fit.fit()
        if (inst.ptyId) void currentBridge().terminalResize(inst.ptyId, inst.term.cols, inst.term.rows)
      } catch {
        // Container may be hidden mid-transition.
      }
    }

    const ro = new ResizeObserver(fitNow)
    ro.observe(container)
    requestAnimationFrame(() => {
      fitNow()
      if (!inst.ptyId && !inst.starting && !inst.exited) {
        void startPty(inst).then(() => fitNow())
      }
      inst.term.focus()
    })

    return () => {
      ro.disconnect()
      inst.listeners.delete(sub)
    }
  }, [shown, directory])

  if (!shown) return null

  const inst = getOrCreate(directory)

  const onDragStart = (e: React.MouseEvent) => {
    e.preventDefault()
    dragging.current = true
    const startY = e.clientY
    const startH = height
    const onMove = (ev: MouseEvent) => {
      if (!dragging.current) return
      const max = Math.max(MIN_HEIGHT, Math.floor(window.innerHeight * 0.65))
      const next = Math.min(max, Math.max(MIN_HEIGHT, startH + (startY - ev.clientY)))
      setHeight(next)
    }
    const onUp = () => {
      dragging.current = false
      window.removeEventListener("mousemove", onMove)
      window.removeEventListener("mouseup", onUp)
      setHeight((h) => {
        saveHeight(h)
        return h
      })
    }
    window.addEventListener("mousemove", onMove)
    window.addEventListener("mouseup", onUp)
  }

  return (
    <section
      aria-label="Terminal"
      style={{ height }}
      className="flex shrink-0 flex-col border-t border-separator bg-background/85"
    >
      {/* Drag handle */}
      <div
        onMouseDown={onDragStart}
        role="separator"
        aria-orientation="horizontal"
        aria-label="Resize terminal"
        className="h-1.5 shrink-0 cursor-row-resize transition-colors hover:bg-default-hover/70"
      />
      {/* Slim header */}
      <div className="flex h-8 shrink-0 items-center gap-2 px-3">
        <TerminalIcon className="size-3.5 shrink-0 text-muted" strokeWidth={1.75} />
        <span className="text-2xs min-w-0 truncate font-mono text-muted" title={directory}>
          {folderName(directory)}
          {worktree ? ` · ${worktree.branch}` : ""}
        </span>
        {inst.shell && <span className="text-3xs shrink-0 rounded bg-default/70 px-1.5 py-0.5 font-mono text-muted">{inst.shell}</span>}
        {inst.exited && (
          <span className="text-2xs shrink-0 text-muted">
            Exited{inst.exited.code !== null ? ` (${inst.exited.code})` : ""}
          </span>
        )}
        <div className="ml-auto flex shrink-0 items-center gap-0.5">
          <Tooltip label="Restart shell" side="top">
            <button
              type="button"
              onClick={() => void restartPty(inst)}
              aria-label="Restart shell"
              className={btn("ghost", "sm", "size-7 text-muted", true)}
            >
              <RotateCw className="size-3.5" />
            </button>
          </Tooltip>
          <Tooltip label="Clear" side="top">
            <button
              type="button"
              onClick={() => {
                inst.term.clear()
                inst.term.focus()
              }}
              aria-label="Clear terminal"
              className={btn("ghost", "sm", "size-7 text-muted", true)}
            >
              <Trash2 className="size-3.5" />
            </button>
          </Tooltip>
          <Tooltip label="Hide terminal" keys={KEYS.terminal} side="top">
            <button
              type="button"
              onClick={() => setTerminalOpen(false)}
              aria-label="Hide terminal"
              className={btn("ghost", "sm", "size-7 text-muted", true)}
            >
              <X className="size-3.5" />
            </button>
          </Tooltip>
        </div>
      </div>
      {inst.error ? (
        <div className="flex flex-1 flex-col items-start justify-center gap-2 px-4 text-dense text-danger">
          <span>{inst.error}</span>
          <button type="button" onClick={() => void restartPty(inst)} className={btn("secondary", "sm")}>
            Try again
          </button>
        </div>
      ) : (
        <div
          ref={mountRef}
          onClick={() => inst.term.focus()}
          className={cn("min-h-0 flex-1 overflow-hidden px-2 pb-1.5")}
        />
      )}
    </section>
  )
}
