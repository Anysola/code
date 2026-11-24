import { Spinner as HeroSpinner } from "@heroui/react/spinner"
import { buttonVariants } from "@heroui/styles"
import { clsx, type ClassValue } from "clsx"
import { Minus, Plus, Square, X } from "lucide-react"
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode, type RefObject } from "react"
import { extendTailwindMerge } from "tailwind-merge"

import { getBridge } from "./bridge"
import { parsePatch, type DiffLine } from "./patch"

// Grafit's added sizes are sizes, not colours: without this, a colour class after one drops it.
const twMerge = extendTailwindMerge({ extend: { theme: { text: ["3xs", "2xs", "dense", "md"] } } })

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

export function btn(
  variant: "primary" | "secondary" | "tertiary" | "ghost" | "danger-soft",
  size: "sm" | "md" = "md",
  className?: string,
  iconOnly = false
) {
  return cn(buttonVariants({ variant, size, isIconOnly: iconOnly }), className)
}

export function Spinner({ className }: { className?: string }) {
  return <HeroSpinner size="sm" className={cn("size-3.5 text-muted", className)} />
}

export function Kbd({ children, onPrimary = false }: { children: ReactNode; onPrimary?: boolean }) {
  return (
    <kbd
      className={cn(
        "font-sans text-2xs inline-flex h-5 min-w-5 items-center justify-center rounded-md px-1.5",
        onPrimary ? "bg-accent-foreground/10 text-accent-foreground/70" : "bg-default/60 text-muted"
      )}
    >
      {children}
    </kbd>
  )
}

/** The Anysola A. */
export function Mark({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 365.18 280.9" className={cn("h-3 w-auto", className)} fill="currentColor" aria-hidden>
      <path d="M145.43 0 L208.55 0 L222.7 22.31 L73.74 280.9 L17.31 280.9 L0 250.08 Z" />
      <path d="M235.09 40.45 L365.18 249.81 L347.6 280.9 L288.09 280.9 L189.33 119.48 Z" />
    </svg>
  )
}

/** The window has no native frame: this bar drags it; the controls sit on the rightmost one. */
export function TitleBar({
  children,
  className,
  controls = false,
}: {
  children?: ReactNode
  className?: string
  controls?: boolean
}) {
  return (
    // "deep": the title, folder and logo inside drag too; buttons and popovers don't.
    <div data-tauri-drag-region="deep" className={cn("flex h-9 shrink-0 items-center select-none", className)}>
      <div className="flex h-full min-w-0 flex-1 items-center">
        {children}
      </div>
      {controls && <WindowControls />}
    </div>
  )
}

/** Work that must finish before the window closes (a delete waiting behind Undo). */
const beforeClose: (() => Promise<void> | void)[] = []

export function onBeforeClose(run: () => Promise<void> | void) {
  beforeClose.push(run)
}

/** What must finish before the window goes (drafts, waiting deletes); never rejects. */
export function runBeforeClose() {
  return Promise.all(beforeClose.map((run) => Promise.resolve().then(run).catch(() => {}))).then(() => {})
}

function WindowControls() {
  const act = (fn: "minimize" | "toggleMaximize" | "close") => () =>
    void (fn === "close" ? runBeforeClose() : Promise.resolve())
      .then(() => getBridge())
      .then((b) => b.window[fn]())
  const buttons = [
    { icon: Minus, label: "Minimize", run: act("minimize") },
    { icon: Square, label: "Maximize", run: act("toggleMaximize") },
    { icon: X, label: "Close", run: act("close") },
  ]
  // Each control takes the bar's full height to be easy to hit, but paints like the header's icon
  // buttons: a full-bleed block would be cut by the panel's rounded corner. The inset keeps
  // Close's circle clear of that corner's curve.
  return (
    <div className="flex h-full pr-2">
      {buttons.map(({ icon: Icon, label, run }, i) => (
        <Tooltip key={label} label={label}>
          <button onClick={run} aria-label={label} className="group/wc flex h-full w-11 items-center justify-center outline-none">
            <span
              className={btn(
                "ghost",
                "sm",
                cn(
                  "pointer-events-none text-muted group-hover/wc:bg-(--button-bg-hover) group-hover/wc:text-foreground group-active/wc:scale-[0.98] group-active/wc:bg-(--button-bg-pressed) group-focus-visible/wc:status-focused",
                  i === 2 && "group-hover/wc:bg-danger group-hover/wc:text-danger-foreground group-active/wc:bg-danger-hover"
                ),
                true
              )}
            >
              <Icon className={i === 1 ? "size-3" : "size-3.5"} strokeWidth={1.75} />
            </span>
          </button>
        </Tooltip>
      ))}
    </div>
  )
}

export function UsageMeter({ percent, className }: { percent: number | null; className?: string }) {
  if (percent === null) return null
  return (
    <div className={cn("flex items-center gap-2", className)} title="Share of your 5-hour limit used">
      <div className="h-1 flex-1 overflow-hidden rounded-full bg-default">
        <div
          className={cn("h-full rounded-full", percent >= 90 ? "bg-warning" : "bg-foreground/70")}
          style={{ width: `${Math.min(100, percent)}%` }}
        />
      </div>
      <span className="text-2xs text-muted tabular-nums">{percent}% of 5h</span>
    </div>
  )
}

export function Counts({ add, del }: { add: number; del: number }) {
  return (
    <span className="font-mono text-2xs tabular-nums">
      <span className="text-success">+{add}</span>
      {del > 0 && <span className="ml-1.5 text-danger">−{del}</span>}
    </span>
  )
}

/** A diff; with `comments`, a line takes a remark (the "+" in its gutter) and shows its own. */
export function DiffView({
  patch,
  wrap = false,
  className,
  comments,
}: {
  patch: string
  wrap?: boolean
  className?: string
  comments?: {
    list: { id: number; line: number; removed: boolean; text: string }[]
    add: (line: DiffLine, text: string) => void
    remove: (id: number) => void
  }
}) {
  const lines = parsePatch(patch)
  const [writing, setWriting] = useState<number | null>(null)
  return (
    <div className={cn("overflow-x-auto font-mono text-xs leading-[1.6rem]", className)}>
      {lines.map((l, i) => {
        const number = l.kind === "del" ? l.old : l.new
        const own =
          comments && l.kind !== "hunk"
            ? comments.list.filter((c) => c.line === number && c.removed === (l.kind === "del"))
            : []
        return (
          <div key={i}>
            <DiffRow l={l} wrap={wrap} onComment={comments && l.kind !== "hunk" ? () => setWriting(i) : undefined} />
            {own.map((c) => (
              <div key={c.id} className="text-dense flex items-start gap-2 bg-default/40 py-2 pr-3 pl-18 font-sans">
                <span className="min-w-0 flex-1 whitespace-pre-wrap">{c.text}</span>
                <Tooltip label="Remove comment">
                  <button
                    onClick={() => comments?.remove(c.id)}
                    aria-label="Remove comment"
                    className="mt-0.5 rounded text-muted hover:text-foreground focus-visible:status-focused-field"
                  >
                    <X className="size-3.5" />
                  </button>
                </Tooltip>
              </div>
            ))}
            {writing === i && comments && (
              <CommentBox
                onCancel={() => setWriting(null)}
                onAdd={(text) => {
                  comments.add(l, text)
                  setWriting(null)
                }}
              />
            )}
          </div>
        )
      })}
    </div>
  )
}

function CommentBox({ onAdd, onCancel }: { onAdd: (text: string) => void; onCancel: () => void }) {
  const [text, setText] = useState("")
  return (
    <div className="bg-default/40 py-2 pr-3 pl-18 font-sans">
      <textarea
        autoFocus
        value={text}
        rows={2}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey && text.trim()) {
            e.preventDefault()
            onAdd(text.trim())
          } else if (e.key === "Escape") {
            e.stopPropagation()
            onCancel()
          }
        }}
        placeholder="What should change here?"
        aria-label="Comment"
        className="text-dense block w-full resize-none rounded-lg bg-field px-2.5 py-1.5 outline-none placeholder:text-field-placeholder focus-visible:status-focused-field"
      />
      <div className="flex items-center justify-end gap-2 pt-1.5">
        <span className="text-2xs mr-auto text-muted">Goes with your next message</span>
        <button onClick={onCancel} className={btn("ghost", "sm")}>
          Cancel
        </button>
        <button onClick={() => text.trim() && onAdd(text.trim())} disabled={!text.trim()} className={btn("primary", "sm")}>
          Comment
        </button>
      </div>
    </div>
  )
}

function DiffRow({ l, wrap, onComment }: { l: DiffLine; wrap: boolean; onComment?: () => void }) {
  if (l.kind === "hunk") return <div className="bg-default/30 px-3 text-muted">{l.text}</div>
  return (
    <div
      className={cn(
        "code-line group/line relative flex",
        wrap ? "whitespace-pre-wrap" : "whitespace-pre",
        l.kind === "add" && "bg-success/10",
        l.kind === "del" && "bg-danger/10"
      )}
    >
      {onComment && (
        <Tooltip label="Comment on this line">
          <button
            onClick={onComment}
            aria-label="Comment on this line"
            className="absolute top-1 left-1 flex size-4 items-center justify-center rounded bg-accent text-accent-foreground opacity-0 group-hover/line:opacity-100 focus-visible:opacity-100"
          >
            <Plus className="size-3" strokeWidth={2.5} />
          </button>
        </Tooltip>
      )}
      {/* With comments, the gutter leaves room for the "+" before the number. */}
      <span className={cn("shrink-0 pr-2 text-right text-muted/90 tabular-nums select-none", onComment ? "w-14" : "w-10")}>
        {l.kind === "del" ? l.old : l.new}
      </span>
      <span
        className={cn(
          "w-4 shrink-0 text-center select-none",
          l.kind === "add" ? "text-success" : l.kind === "del" ? "text-danger" : "text-transparent"
        )}
      >
        {l.kind === "add" ? "+" : l.kind === "del" ? "−" : " "}
      </span>
      <span className={cn("min-w-0 pr-4", l.kind === "ctx" && "text-foreground/70")}>{l.text}</span>
    </div>
  )
}

/** "now", "40m ago", "5h ago", "2d ago", then the day and month (DESIGN.md short form). */
export function ago(ms: number) {
  const s = Math.max(0, (Date.now() - ms) / 1000)
  if (s < 60) return "now"
  if (s < 3600) return `${Math.floor(s / 60)}m ago`
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`
  if (s < 7 * 86400) return `${Math.floor(s / 86400)}d ago`
  return new Date(ms).toLocaleDateString("en-GB", { day: "numeric", month: "short", timeZone: "UTC" })
}

export function folderName(directory: string) {
  return directory.split(/[\\/]/).filter(Boolean).pop() ?? directory
}

/**
 * One key per folder however it is spelled (the picker, a stored session and a terminal differ on
 * Windows): Windows paths (drive letter or UNC) ignore case and treat `/` as `\`; any path
 * ignores a trailing separator. Shown and passed on, a folder keeps its first spelling.
 */
export function folderKey(directory: string) {
  if (/^[a-z]:([\\/]|$)|^[\\/]{2}[^\\/]/i.test(directory))
    return directory.replace(/\//g, "\\").replace(/\\+$/, "").toLowerCase()
  return directory.length > 1 ? directory.replace(/\/+$/, "") : directory
}

export function sameFolder(a: string | null | undefined, b: string | null | undefined) {
  return a != null && b != null && folderKey(a) === folderKey(b)
}

/**
 * Everything laid over the window that Escape closes (a panel, a sheet, a menu, Settings), in
 * the order it opened. One Escape closes the top one only; nothing under it hears the key.
 */
type Layer = {
  panel: RefObject<HTMLElement | null>
  onEscape: RefObject<(e: KeyboardEvent) => void>
  /** A menu: it takes Escape even from inside, before a form around it would cancel itself. */
  first: boolean
}
const layers: Layer[] = []

/** True while something Escape would close is open: the composer leaves the key alone. */
export function anyLayer() {
  return layers.length > 0
}

if (typeof document !== "undefined") {
  // Pressed outside the top layer (in the composer behind it, say): taken before anyone sees it.
  document.addEventListener(
    "keydown",
    (e) => {
      const top = layers[layers.length - 1]
      if (e.key !== "Escape" || !top || (!top.first && top.panel.current?.contains(e.target as Node))) return
      e.preventDefault()
      e.stopPropagation()
      top.onEscape.current(e)
    },
    true
  )
  // Pressed inside it: its fields go first (a comment box cancels the comment and stops the key).
  window.addEventListener("keydown", (e) => {
    const top = layers[layers.length - 1]
    if (e.key !== "Escape" || e.defaultPrevented || !top || !top.panel.current?.contains(e.target as Node)) return
    e.preventDefault()
    top.onEscape.current(e)
  })
}

/**
 * Makes `panel` a layer while `active`: Escape closes it when it is the top one. Pressed in the
 * composer behind it, the key goes to the panel only: the composer would stop the turn or deny
 * the waiting ask.
 */
export function useEscape(
  panel: RefObject<HTMLElement | null>,
  onEscape: (e: KeyboardEvent) => void,
  active = true,
  first = false
) {
  const latest = useRef(onEscape)
  latest.current = onEscape
  useEffect(() => {
    if (!active) return
    const layer: Layer = { panel, onEscape: latest, first }
    layers.push(layer)
    return () => {
      const at = layers.indexOf(layer)
      if (at >= 0) layers.splice(at, 1)
    }
  }, [panel, active, first])
}

const ITEMS = '[role="menuitem"]:not(:disabled), [role="option"]:not(:disabled), [role="radio"]:not(:disabled)'

/** Room kept between a panel and the window's edge. */
const EDGE = 8

/**
 * A panel that opens from a control: closes on Escape (focus goes back to the control) and on a
 * click outside. `side` puts it above or below the element it sits in, flipped when there's no
 * room; it is fixed to the window and clamped inside it, so no frame's overflow can clip it.
 * `menu`: a keyboard menu (role=menu, its first item focused, ↑/↓/Home/End, Tab closes); a list
 * of options moves the same way. `matchWidth`: at least as wide as that element.
 */
export function Popover({
  open,
  onClose,
  side = "top",
  align = "start",
  menu = false,
  label,
  matchWidth = false,
  className,
  children,
}: {
  open: boolean
  onClose: () => void
  side?: "top" | "bottom"
  align?: "start" | "end"
  menu?: boolean
  label?: string
  matchWidth?: boolean
  className?: string
  children: ReactNode
}) {
  const ref = useRef<HTMLDivElement>(null)
  const [place, setPlace] = useState<CSSProperties>({ top: 0, left: 0, opacity: 0 })
  // Placed against the element it sits in, measured before it shows and again as things move.
  useLayoutEffect(() => {
    if (!open) return
    const panel = ref.current
    const anchor = panel?.parentElement
    if (!panel || !anchor) return
    const measure = () => {
      const a = anchor.getBoundingClientRect()
      const vw = window.innerWidth
      const vh = window.innerHeight
      const gap = side === "top" ? 8 : 6
      const w = Math.min(Math.max(panel.offsetWidth, matchWidth ? a.width : 0), vw - 2 * EDGE)
      // The whole content's height, whatever height it was last capped at.
      const h = panel.scrollHeight
      const below = vh - a.bottom - gap - EDGE
      const above = a.top - gap - EDGE
      const down = side === "bottom" ? h <= below || below >= above : !(h <= above || above >= below)
      const room = Math.max(120, down ? below : above)
      const height = Math.min(h, room)
      const top = down ? a.bottom + gap : a.top - gap - height
      const left = Math.min(Math.max(EDGE, align === "start" ? a.left : a.right - w), vw - w - EDGE)
      setPlace({ top, left, maxHeight: room, minWidth: matchWidth ? a.width : undefined })
    }
    measure()
    window.addEventListener("resize", measure)
    // A scrolling list under an open row menu: the menu follows its row.
    document.addEventListener("scroll", measure, true)
    return () => {
      window.removeEventListener("resize", measure)
      document.removeEventListener("scroll", measure, true)
    }
  }, [open, side, align, matchWidth])
  // The latest onClose, so a caller's new function each render doesn't reopen (and refocus) it.
  const closeRef = useRef(onClose)
  closeRef.current = onClose
  useEscape(ref, () => closeRef.current(), open, true)
  useEffect(() => {
    if (!open) return
    const onClose = () => closeRef.current()
    const trigger = document.activeElement instanceof HTMLElement ? document.activeElement : null
    const panel = ref.current
    // Into the panel: a menu's first item, a list's chosen option, else the panel itself.
    const first =
      panel?.querySelector<HTMLElement>('[role="option"][aria-selected="true"]') ??
      panel?.querySelector<HTMLElement>(ITEMS) ??
      null
    ;(first ?? panel)?.focus({ preventScroll: true })
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node
      // The control it opened from toggles it itself.
      if (ref.current && !ref.current.contains(t) && !ref.current.parentElement?.contains(t)) onClose()
    }
    // After the click that opened it.
    const t = setTimeout(() => document.addEventListener("mousedown", onDown))
    return () => {
      clearTimeout(t)
      document.removeEventListener("mousedown", onDown)
      // Closed from inside (Escape, an item): focus goes back to the control, not to nowhere.
      const now = document.activeElement
      if (trigger?.isConnected && (!now || now === document.body || panel?.contains(now))) trigger.focus({ preventScroll: true })
    }
  }, [open])
  if (!open) return null
  return (
    <div
      ref={ref}
      role={menu ? "menu" : "dialog"}
      aria-label={label}
      tabIndex={-1}
      // Inside a title bar, its text and padding would drag the window.
      data-tauri-drag-region="false"
      onKeyDown={(e) => {
        if (menu && e.key === "Tab") {
          e.preventDefault()
          onClose()
          return
        }
        const target = e.target as HTMLElement
        if (!["ArrowDown", "ArrowUp", "Home", "End"].includes(e.key) || !target.matches(ITEMS)) return
        const items = [...e.currentTarget.querySelectorAll<HTMLElement>(ITEMS)]
        const at = items.indexOf(target)
        const next =
          e.key === "Home" ? 0 : e.key === "End" ? items.length - 1 : (at + (e.key === "ArrowDown" ? 1 : -1) + items.length) % items.length
        e.preventDefault()
        items[next]?.focus()
      }}
      style={place}
      className={cn("fixed z-50 overflow-y-auto rounded-xl bg-popover p-1.5 outline-none", className)}
    >
      {children}
    </div>
  )
}

export function MenuItem({
  onSelect,
  danger = false,
  disabled = false,
  children,
}: {
  onSelect: () => void
  danger?: boolean
  disabled?: boolean
  children: ReactNode
}) {
  return (
    <button
      type="button"
      role="menuitem"
      tabIndex={-1}
      disabled={disabled}
      onClick={onSelect}
      className={cn(
        "text-dense flex h-8 w-full shrink-0 items-center gap-2 rounded-lg px-2.5 text-left outline-none hover:bg-default-hover focus-visible:bg-default-hover disabled:opacity-50",
        danger && "text-danger"
      )}
    >
      {children}
    </button>
  )
}

export function MenuSeparator() {
  return <div role="separator" className="my-1 border-t border-separator" />
}

// ---------------------------------------------------------------------------
// Tooltip
// ---------------------------------------------------------------------------

/** When the last tooltip hid: one shown a moment ago makes the next show at once. */
let warmUntil = 0

/**
 * A small label (and its shortcut) for an icon-only control: after a short hover, or at once on
 * keyboard focus. The control keeps its own aria-label; this is for the eye only.
 */
export function Tooltip({
  label,
  keys,
  note,
  side = "bottom",
  children,
}: {
  label: string
  keys?: string | null
  /** A second, quieter line: what the control does that its name doesn't say. */
  note?: string
  side?: "top" | "bottom"
  children: ReactNode
}) {
  const wrap = useRef<HTMLSpanElement>(null)
  const tip = useRef<HTMLSpanElement>(null)
  const [show, setShow] = useState(false)
  const [place, setPlace] = useState<CSSProperties>({ top: 0, left: 0, opacity: 0 })
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const open = (now: boolean) => {
    clearTimeout(timer.current)
    if (now || Date.now() < warmUntil) setShow(true)
    else timer.current = setTimeout(() => setShow(true), 450)
  }
  const close = () => {
    clearTimeout(timer.current)
    setShow((s) => {
      if (s) warmUntil = Date.now() + 500
      return false
    })
  }
  useEffect(() => () => clearTimeout(timer.current), [])
  useEffect(() => {
    if (!show) return
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && close()
    document.addEventListener("keydown", onKey, true)
    document.addEventListener("scroll", close, true)
    return () => {
      document.removeEventListener("keydown", onKey, true)
      document.removeEventListener("scroll", close, true)
    }
  }, [show])
  useLayoutEffect(() => {
    if (!show) return
    const el = wrap.current?.firstElementChild
    const t = tip.current
    if (!el || !t) return
    const a = el.getBoundingClientRect()
    const w = t.offsetWidth
    const h = t.offsetHeight
    const down = side === "bottom" ? a.bottom + 6 + h <= window.innerHeight - EDGE : a.top - 6 - h < EDGE
    const left = Math.min(Math.max(EDGE, a.left + a.width / 2 - w / 2), window.innerWidth - w - EDGE)
    setPlace({ top: down ? a.bottom + 6 : a.top - 6 - h, left })
  }, [show, side])
  return (
    <span
      ref={wrap}
      className="contents"
      onPointerEnter={(e) => e.pointerType === "mouse" && open(false)}
      onPointerLeave={close}
      onPointerDown={close}
      onFocus={(e) => e.target.matches(":focus-visible") && open(true)}
      onBlur={close}
    >
      {children}
      {show && (
        <span
          ref={tip}
          aria-hidden
          style={place}
          className={cn(
            "text-2xs pointer-events-none fixed z-[60] rounded-md bg-popover px-2 py-1 text-foreground",
            note ? "flex w-max max-w-64 flex-col gap-0.5" : "flex items-center gap-1.5 whitespace-nowrap"
          )}
        >
          {label}
          {keys && <span className="text-muted">{keys}</span>}
          {note && <span className="text-muted">{note}</span>}
        </span>
      )}
    </span>
  )
}

/** "in 2h 10m", "in 3d": when a usage window starts over. */
export function resetsIn(iso: string | null) {
  if (!iso) return null
  const ms = Date.parse(iso) - Date.now()
  if (!Number.isFinite(ms) || ms <= 0) return null
  const m = Math.round(ms / 60_000)
  if (m < 60) return `in ${m}m`
  const h = Math.floor(m / 60)
  if (h < 24) return `in ${h}h ${m % 60}m`
  return `in ${Math.round(h / 24)}d`
}
