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

import { setResizing, setTerminalOpen, useChrome } from "./chrome"
import { noteTerminalUrls } from "./preview"
import { KEYS } from "./shortcuts"
import { currentBridge, useWorkDir, useWorktree, type Open } from "./store"
import { btn, cn, folderKey, folderName, Tooltip } from "./ui"

const HEIGHT_KEY = "anysola.terminalHeight"
const DEFAULT_HEIGHT = 240
const MIN_HEIGHT = 140

function maxHeight(): number {
  return Math.max(MIN_HEIGHT, Math.floor(window.innerHeight * 0.55))
}

function clampHeight(px: number): number {
  return Math.min(maxHeight(), Math.max(MIN_HEIGHT, px))
}

function readHeight(): number {
  try {
    const n = Number(localStorage.getItem(HEIGHT_KEY))
    return Number.isFinite(n) && n >= MIN_HEIGHT ? clampHeight(n) : clampHeight(DEFAULT_HEIGHT)
  } catch {
    return clampHeight(DEFAULT_HEIGHT)
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
  // Exact match with the drawer surface (`bg-[#f6f5f2] dark:bg-[#161411]`) so there is no seam.
  if (dark) {
    return {
      background: "#161411",
      foreground: "#f5f4f2",
      cursor: "#e7e4de",
      cursorAccent: "#161411",
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
    background: "#f6f5f2",
    foreground: "#1c1a17",
    cursor: "#2b2824",
    cursorAccent: "#f6f5f2",
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

type TerminalEvent =
  | { type: "terminalData"; id: string; data: string }
  | { type: "terminalExit"; id: string; code: number | null }

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
  decoder: TextDecoder
  lineBuf: string
  listeners: Set<() => void>
}

const instances = new Map<string, TermInstance>()
const byPtyId = new Map<string, TermInstance>()
/** Buffer early events that arrive before `terminalOpen` resolves and registers `byPtyId`. */
const pendingEvents = new Map<string, TerminalEvent[]>()

function notifyInstance(inst: TermInstance) {
  inst.listeners.forEach((fn) => fn())
}

function applyEvent(inst: TermInstance, event: TerminalEvent) {
  if (event.type === "terminalData") {
    const bytes = decodeBase64(event.data)
    inst.term.write(bytes)
    try {
      const chunk = inst.decoder.decode(bytes, { stream: true })
      inst.lineBuf = (inst.lineBuf + chunk).slice(-1024)
      const lines = inst.lineBuf.split(/\r?\n/)
      // Scan completed lines plus a trailing slice once whitespace terminates a URL.
      if (lines.length > 1) {
        noteTerminalUrls(lines.slice(0, -1).join("\n"))
        inst.lineBuf = lines[lines.length - 1]
      }
      if (/\s$/.test(inst.lineBuf)) {
        noteTerminalUrls(inst.lineBuf)
      }
    } catch {
      // Non-UTF-8 chunk: skip URL scan.
    }
  } else {
    byPtyId.delete(event.id)
    inst.ptyId = null
    inst.exited = { code: event.code }
    const suffix = event.code === null ? "" : ` (exit ${event.code})`
    inst.term.writeln(`\r\n\x1b[2m[Process exited${suffix}]\x1b[0m`)
    notifyInstance(inst)
  }
}

/** Routed from `store.ts`'s single `core` event listener. */
export function handleTerminalEvent(event: TerminalEvent) {
  const inst = byPtyId.get(event.id)
  if (!inst) {
    const list = pendingEvents.get(event.id) ?? []
    if (list.length < 200) {
      list.push(event)
      pendingEvents.set(event.id, list)
    }
    return
  }
  applyEvent(inst, event)
}

function getOrCreate(directory: string): TermInstance {
  const key = folderKey(directory)
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
    decoder: new TextDecoder(),
    lineBuf: "",
    listeners: new Set(),
  }

  term.attachCustomKeyEventHandler((e) => {
    if (e.type !== "keydown") return true
    const ctrl = (e.ctrlKey || e.metaKey) && !e.altKey
    // Let Ctrl+` and Ctrl+Shift+O bubble to window shortcuts.
    if (ctrl && (e.key === "`" || e.code === "Backquote")) return false
    if (ctrl && e.shiftKey && e.key.toLowerCase() === "o") return false
    // Copy when text is selected (or Ctrl+Shift+C); otherwise send ^C to the PTY.
    if (ctrl && e.key.toLowerCase() === "c" && (e.shiftKey || term.hasSelection())) {
      e.preventDefault()
      e.stopPropagation()
      const sel = term.getSelection()
      if (sel) void navigator.clipboard.writeText(sel)
      return false
    }
    if (ctrl && e.key.toLowerCase() === "v") {
      e.preventDefault()
      e.stopPropagation()
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
  inst.lineBuf = ""
  notifyInstance(inst)
  try {
    const cols = Math.max(20, inst.term.cols || 80)
    const rows = Math.max(4, inst.term.rows || 14)
    const { id, shell } = await currentBridge().terminalOpen(inst.directory, cols, rows)
    inst.ptyId = id
    inst.shell = shell
    byPtyId.set(id, inst)
    // Flush any early output/exit events that arrived before `terminalOpen` returned.
    const buffered = pendingEvents.get(id)
    if (buffered) {
      pendingEvents.delete(id)
      for (const ev of buffered) applyEvent(inst, ev)
    }
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
    pendingEvents.delete(old)
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

  // Keep height clamped when the window shrinks.
  useEffect(() => {
    const onResize = () => setHeight((h) => clampHeight(h))
    window.addEventListener("resize", onResize)
    return () => window.removeEventListener("resize", onResize)
  }, [])

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
    setResizing(true)
    const startY = e.clientY
    const startH = height
    const onMove = (ev: MouseEvent) => {
      if (!dragging.current) return
      setHeight(clampHeight(startH + (startY - ev.clientY)))
    }
    const onUp = () => {
      dragging.current = false
      setResizing(false)
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
      className="flex shrink-0 flex-col bg-[#f6f5f2] dark:bg-[#161411]"
    >
      {/* Tone-stepped drag handle (no hard hairline border) */}
      <div
        onMouseDown={onDragStart}
        role="separator"
        aria-orientation="horizontal"
        aria-label="Resize terminal"
        className="group flex h-2 shrink-0 cursor-row-resize items-center justify-center"
      >
        <span className="h-0.5 w-8 rounded-full bg-transparent transition-colors group-hover:bg-muted/40" />
      </div>
      {/* Slim header (matches Preview height) */}
      <div className="flex h-9 shrink-0 items-center gap-2 px-3">
        <TerminalIcon className="size-3.5 shrink-0 text-muted" strokeWidth={1.75} />
        <span className="text-2xs min-w-0 truncate font-mono text-muted" title={directory}>
          {folderName(directory)}
          {worktree ? ` · ${worktree.branch}` : ""}
        </span>
        {inst.shell && <span className="text-3xs shrink-0 rounded bg-default/60 px-1.5 py-0.5 font-mono text-muted">{inst.shell}</span>}
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
          className={cn("min-h-0 flex-1 overflow-hidden px-3 pb-2")}
        />
      )}
    </section>
  )
}
