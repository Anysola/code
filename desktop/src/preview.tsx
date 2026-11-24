/**
 * Live local Preview panel beside the conversation: embeds a localhost / 127.0.0.1 dev server
 * inside the app with reload, responsive viewport presets (Desktop / Tablet / Mobile), external
 * browser handoff, and automatic detection of localhost URLs printed in the embedded terminal.
 */
import { ExternalLink, Globe, Monitor, RotateCw, Smartphone, Tablet, X } from "lucide-react"
import { useEffect, useRef, useState, type FormEvent } from "react"

import { setPreviewOpen, setPreviewUrl, useChrome } from "./chrome"
import { KEYS } from "./shortcuts"
import { openExternal } from "./store"
import { btn, cn, Tooltip } from "./ui"

const WIDTH_KEY = "anysola.previewWidth"
const DEFAULT_WIDTH = 520
const MIN_WIDTH = 320

type Viewport = "desktop" | "tablet" | "mobile"

const VIEWPORT_WIDTH: Record<Viewport, number | null> = {
  desktop: null,
  tablet: 768,
  mobile: 390,
}

function readWidth(): number {
  try {
    const n = Number(localStorage.getItem(WIDTH_KEY))
    return Number.isFinite(n) && n >= MIN_WIDTH ? n : DEFAULT_WIDTH
  } catch {
    return DEFAULT_WIDTH
  }
}

function saveWidth(px: number) {
  try {
    localStorage.setItem(WIDTH_KEY, String(Math.round(px)))
  } catch {
    // Kept for this window only.
  }
}

/** Normalizes user input (`3000`, `localhost:5173`, `http://127.0.0.1:3000/app`) into a local URL. */
export function normalizeLocalUrl(raw: string): string | null {
  const s = raw.trim()
  if (!s) return null
  if (/^\d{2,5}(\/.*)?$/.test(s)) return `http://localhost:${s}`
  const withProto = /^https?:\/\//i.test(s) ? s : `http://${s}`
  try {
    const u = new URL(withProto)
    if (u.protocol !== "http:" && u.protocol !== "https:") return null
    const host = u.hostname.toLowerCase()
    if (host === "localhost" || host === "127.0.0.1" || host === "[::1]" || host === "::1") {
      return u.toString()
    }
    return null
  } catch {
    return null
  }
}

const LOCAL_URL_RE = /\bhttps?:\/\/(?:localhost|127\.0\.0\.1|\[::1\]):\d{2,5}(?:\/[^\s"'<>)\x1b]*)?/gi

// Detected localhost URLs from terminal output (most recent first, max 6).
const detectedUrls: string[] = []
const detectedListeners = new Set<() => void>()

export function noteTerminalUrls(text: string) {
  const matches = text.match(LOCAL_URL_RE)
  if (!matches) return
  let changed = false
  for (const raw of matches) {
    const cleaned = raw.replace(/[.,;:]+$/, "")
    const norm = normalizeLocalUrl(cleaned)
    if (!norm) continue
    const idx = detectedUrls.indexOf(norm)
    if (idx === 0) continue
    if (idx > 0) detectedUrls.splice(idx, 1)
    detectedUrls.unshift(norm)
    if (detectedUrls.length > 6) detectedUrls.length = 6
    changed = true
  }
  if (changed) detectedListeners.forEach((l) => l())
}

function useDetectedUrls(): string[] {
  const [, bump] = useState(0)
  useEffect(() => {
    const fn = () => bump((n) => n + 1)
    detectedListeners.add(fn)
    return () => {
      detectedListeners.delete(fn)
    }
  }, [])
  return detectedUrls
}

export function PreviewPanel() {
  const shown = useChrome((c) => c.previewOpen)
  const url = useChrome((c) => c.previewUrl)
  const detected = useDetectedUrls()
  const [width, setWidth] = useState(readWidth)
  const [input, setInput] = useState(url ?? "")
  const [error, setError] = useState<string | null>(null)
  const [nonce, setNonce] = useState(0)
  const [viewport, setViewport] = useState<Viewport>("desktop")
  const dragging = useRef(false)

  useEffect(() => {
    setInput(url ?? "")
    setError(null)
  }, [url])

  // Auto-fill the first detected localhost URL when the panel opens empty.
  useEffect(() => {
    if (shown && !url && detected.length > 0) {
      setPreviewUrl(detected[0])
    }
  }, [shown, url, detected])

  if (!shown) return null

  const submit = (e: FormEvent) => {
    e.preventDefault()
    const trimmed = input.trim()
    if (!trimmed) {
      setPreviewUrl(null)
      setError(null)
      return
    }
    const norm = normalizeLocalUrl(trimmed)
    if (!norm) {
      setError("Preview only embeds localhost or 127.0.0.1 URLs (or a port like 3000).")
      return
    }
    setError(null)
    setPreviewUrl(norm)
    setNonce((n) => n + 1)
  }

  const onDragStart = (e: React.MouseEvent) => {
    e.preventDefault()
    dragging.current = true
    const startX = e.clientX
    const startW = width
    const onMove = (ev: MouseEvent) => {
      if (!dragging.current) return
      const max = Math.max(MIN_WIDTH, Math.floor(window.innerWidth * 0.65))
      const next = Math.min(max, Math.max(MIN_WIDTH, startW + (startX - ev.clientX)))
      setWidth(next)
    }
    const onUp = () => {
      dragging.current = false
      window.removeEventListener("mousemove", onMove)
      window.removeEventListener("mouseup", onUp)
      setWidth((w) => {
        saveWidth(w)
        return w
      })
    }
    window.addEventListener("mousemove", onMove)
    window.addEventListener("mouseup", onUp)
  }

  const vpWidth = VIEWPORT_WIDTH[viewport]

  return (
    <aside
      aria-label="Preview"
      style={{ width }}
      className="relative flex h-full shrink-0 border-l border-separator bg-background/85"
    >
      {/* Left drag handle */}
      <div
        onMouseDown={onDragStart}
        role="separator"
        aria-orientation="vertical"
        aria-label="Resize preview"
        className="w-1.5 shrink-0 cursor-col-resize transition-colors hover:bg-default-hover/70"
      />
      <div className="flex min-w-0 flex-1 flex-col">
        {/* Toolbar */}
        <div className="flex h-10 shrink-0 items-center gap-1.5 border-b border-separator px-2.5">
          <Tooltip label="Reload" side="bottom">
            <button
              type="button"
              disabled={!url}
              onClick={() => setNonce((n) => n + 1)}
              aria-label="Reload preview"
              className={btn("ghost", "sm", "size-7 text-muted", true)}
            >
              <RotateCw className="size-3.5" />
            </button>
          </Tooltip>
          <form onSubmit={submit} className="min-w-0 flex-1">
            <input
              value={input}
              onChange={(e) => {
                setInput(e.target.value)
                if (error) setError(null)
              }}
              placeholder="http://localhost:3000"
              aria-label="Preview URL"
              className="text-2xs h-7 w-full rounded-md bg-default/60 px-2.5 font-mono text-foreground outline-none placeholder:text-muted focus-visible:status-focused-field"
            />
          </form>
          <div className="flex shrink-0 items-center gap-0.5 rounded-md bg-default/40 p-0.5">
            {(
              [
                ["desktop", Monitor, "Desktop"],
                ["tablet", Tablet, "Tablet (768px)"],
                ["mobile", Smartphone, "Mobile (390px)"],
              ] as const
            ).map(([id, Icon, label]) => (
              <Tooltip key={id} label={label} side="bottom">
                <button
                  type="button"
                  onClick={() => setViewport(id)}
                  aria-label={label}
                  aria-pressed={viewport === id}
                  className={cn(
                    "flex size-6 items-center justify-center rounded text-muted transition-colors hover:text-foreground",
                    viewport === id && "bg-default-hover text-foreground"
                  )}
                >
                  <Icon className="size-3.5" />
                </button>
              </Tooltip>
            ))}
          </div>
          {url && (
            <Tooltip label="Open in browser" side="bottom">
              <button
                type="button"
                onClick={() => void openExternal(url)}
                aria-label="Open in browser"
                className={btn("ghost", "sm", "size-7 text-muted", true)}
              >
                <ExternalLink className="size-3.5" />
              </button>
            </Tooltip>
          )}
          <Tooltip label="Hide preview" keys={KEYS.preview} side="bottom">
            <button
              type="button"
              onClick={() => setPreviewOpen(false)}
              aria-label="Hide preview"
              className={btn("ghost", "sm", "size-7 text-muted", true)}
            >
              <X className="size-3.5" />
            </button>
          </Tooltip>
        </div>

        {error && <p className="text-2xs shrink-0 border-b border-separator px-3 py-1.5 text-danger">{error}</p>}

        {/* Body */}
        {url ? (
          <div className="flex min-h-0 flex-1 items-stretch justify-center overflow-auto bg-background p-0">
            <iframe
              key={`${url}#${nonce}`}
              src={url}
              title="Local preview"
              style={vpWidth ? { width: vpWidth } : undefined}
              className={cn("h-full border-0 bg-white", vpWidth ? "shrink-0 shadow-sm" : "w-full")}
            />
          </div>
        ) : (
          <div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center">
            <Globe className="size-7 text-muted/60" strokeWidth={1.5} />
            <div className="flex flex-col gap-1">
              <p className="text-dense font-medium">Local preview</p>
              <p className="text-2xs max-w-xs text-muted">
                Enter a local URL or port above (like <span className="font-mono">3000</span> or{" "}
                <span className="font-mono">localhost:5173</span>) to view a running dev server beside the chat.
              </p>
            </div>
            {detected.length > 0 && (
              <div className="mt-1 flex flex-wrap justify-center gap-1.5">
                {detected.map((u) => (
                  <button
                    key={u}
                    type="button"
                    onClick={() => {
                      setPreviewUrl(u)
                      setNonce((n) => n + 1)
                    }}
                    className={btn("secondary", "sm", "font-mono text-2xs")}
                  >
                    {u.replace(/^https?:\/\//, "")}
                  </button>
                ))}
              </div>
            )}
          </div>
        )}
      </div>
    </aside>
  )
}
