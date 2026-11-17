import { Spinner as HeroSpinner } from "@heroui/react/spinner"
import { buttonVariants } from "@heroui/styles"
import { clsx, type ClassValue } from "clsx"
import { Minus, Plus, Square, X } from "lucide-react"
import { useEffect, useRef, useState, type ReactNode, type RefObject } from "react"
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

function WindowControls() {
  const act = (fn: "minimize" | "toggleMaximize" | "close") => () => void getBridge().then((b) => b.window[fn]())
  const buttons = [
    { icon: Minus, label: "Minimize", run: act("minimize") },
    { icon: Square, label: "Maximize", run: act("toggleMaximize") },
    { icon: X, label: "Close", run: act("close") },
  ]
  return (
    <div className="flex h-full">
      {buttons.map(({ icon: Icon, label, run }, i) => (
        <button
          key={label}
          onClick={run}
          aria-label={label}
          className={cn(
            "flex h-full w-11 items-center justify-center text-muted transition-colors hover:bg-default-hover hover:text-foreground focus-visible:status-focused-field focus-visible:ring-inset",
            i === 2 && "hover:bg-danger hover:text-danger-foreground"
          )}
        >
          <Icon className={i === 1 ? "size-3" : "size-3.5"} strokeWidth={1.75} />
        </button>
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
                <button
                  onClick={() => comments?.remove(c.id)}
                  aria-label="Remove comment"
                  className="mt-0.5 text-muted hover:text-foreground"
                >
                  <X className="size-3.5" />
                </button>
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
  if (l.kind === "hunk") return <div className="bg-default/30 px-3 text-muted/70">{l.text}</div>
  return (
    <div
      className={cn(
        "group/line relative flex",
        wrap ? "whitespace-pre-wrap" : "whitespace-pre",
        l.kind === "add" && "bg-success/10",
        l.kind === "del" && "bg-danger/10"
      )}
    >
      {onComment && (
        <button
          onClick={onComment}
          aria-label="Comment on this line"
          className="absolute top-1 left-1 flex size-4 items-center justify-center rounded bg-accent text-accent-foreground opacity-0 group-hover/line:opacity-100 focus-visible:opacity-100"
        >
          <Plus className="size-3" strokeWidth={2.5} />
        </button>
      )}
      {/* With comments, the gutter leaves room for the "+" before the number. */}
      <span className={cn("shrink-0 pr-2 text-right text-muted/50 tabular-nums select-none", onComment ? "w-14" : "w-10")}>
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
 * Escape closes a panel laid over the session. Pressed in the composer behind it, the key goes
 * to the panel only: the composer would stop the turn or deny the waiting ask. Fields inside the
 * panel handle their own Escape first (a comment box cancels the comment).
 */
export function useEscape(panel: RefObject<HTMLElement | null>, onClose: () => void) {
  useEffect(() => {
    const outside = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || panel.current?.contains(e.target as Node)) return
      e.stopPropagation()
      onClose()
    }
    const inside = (e: KeyboardEvent) => e.key === "Escape" && onClose()
    document.addEventListener("keydown", outside, true)
    window.addEventListener("keydown", inside)
    return () => {
      document.removeEventListener("keydown", outside, true)
      window.removeEventListener("keydown", inside)
    }
  }, [panel, onClose])
}

/**
 * A panel that opens from a control: closes on Escape and on a click outside. `side` puts it
 * above (composer) or below (sidebar menus) its anchor.
 */
export function Popover({
  open,
  onClose,
  side = "top",
  align = "start",
  className,
  children,
}: {
  open: boolean
  onClose: () => void
  side?: "top" | "bottom"
  align?: "start" | "end"
  className?: string
  children: ReactNode
}) {
  const ref = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose()
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation()
        onClose()
      }
    }
    // After the click that opened it.
    const t = setTimeout(() => document.addEventListener("mousedown", onDown))
    document.addEventListener("keydown", onKey, true)
    return () => {
      clearTimeout(t)
      document.removeEventListener("mousedown", onDown)
      document.removeEventListener("keydown", onKey, true)
    }
  }, [open, onClose])
  if (!open) return null
  return (
    <div
      ref={ref}
      role="dialog"
      // Inside a title bar, its text and padding would drag the window.
      data-tauri-drag-region="false"
      className={cn(
        "absolute z-30 rounded-xl bg-popover p-1.5",
        side === "top" ? "bottom-full mb-2" : "top-full mt-1.5",
        align === "start" ? "left-0" : "right-0",
        className
      )}
    >
      {children}
    </div>
  )
}

export function MenuItem({
  onSelect,
  danger = false,
  children,
}: {
  onSelect: () => void
  danger?: boolean
  children: ReactNode
}) {
  return (
    <button
      onClick={onSelect}
      className={cn(
        "text-dense flex h-8 w-full items-center gap-2 rounded-lg px-2.5 text-left hover:bg-default-hover focus-visible:status-focused-field",
        danger && "text-danger"
      )}
    >
      {children}
    </button>
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
