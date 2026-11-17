/**
 * The pieces every Settings section is made of: a group of rows on one surface, a row with its
 * label on the left and its control on the right, the switch, the segmented choice, the inline
 * confirm, and the loading and error states of a section that reads from the agent.
 */
import { ArrowUpRight, Check, ChevronDown, FolderClosed } from "lucide-react"
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react"

import { openExternal } from "../store"
import { Popover, Spinner, btn, cn, folderName } from "../ui"

export function message(e: unknown): string {
  return typeof e === "string" ? e : e instanceof Error ? e.message : "That didn’t work."
}

/** One section: the nav names it, so its heading is for screen readers only. */
export function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="mx-auto flex w-full max-w-[42rem] flex-col gap-8 px-8 pt-4 pb-16">
      <h1 className="sr-only">{title}</h1>
      {children}
    </div>
  )
}

/** Rows on one surface, under a small label; `aside` sits right of the label (a count, an action). */
export function Group({
  label,
  aside,
  children,
  note,
  className,
}: {
  label?: ReactNode
  aside?: ReactNode
  children: ReactNode
  /** A line under the group that says something no row can. */
  note?: ReactNode
  className?: string
}) {
  return (
    <section className="flex flex-col gap-2">
      {(label || aside) && (
        <div className="flex min-h-6 items-center gap-3 px-1">
          {label && <h2 className="text-2xs min-w-0 truncate font-medium tracking-wide text-muted">{label}</h2>}
          {aside && <div className="text-2xs ml-auto flex items-center gap-2 text-muted">{aside}</div>}
        </div>
      )}
      <div className={cn("flex flex-col divide-y divide-separator rounded-xl bg-surface", className)}>{children}</div>
      {note && <p className="text-2xs px-1 leading-relaxed text-muted">{note}</p>}
    </section>
  )
}

/** A label (and a line under it only when it says more) with its control on the right. */
export function Row({
  label,
  hint,
  children,
  id,
  className,
}: {
  label: ReactNode
  hint?: ReactNode
  children?: ReactNode
  /** Ties the label to its control for screen readers. */
  id?: string
  className?: string
}) {
  return (
    // Narrow, or with a confirm open, the controls wrap under the label instead of squeezing it.
    <div className={cn("flex min-h-12 flex-wrap items-center gap-x-4 gap-y-2 px-4 py-2.5", className)}>
      <div className="flex min-w-[min(100%,12rem)] flex-1 flex-col gap-0.5">
        <span id={id} className="text-sm">
          {label}
        </span>
        {hint && <span className="text-dense text-muted">{hint}</span>}
      </div>
      {children !== undefined && <div className="ml-auto flex max-w-full min-w-0 items-center gap-2">{children}</div>}
    </div>
  )
}

/** A value shown as is, right of its label. */
export function Value({ children, mono = false, title }: { children: ReactNode; mono?: boolean; title?: string }) {
  return (
    <span title={title} className={cn("text-dense max-w-72 truncate text-muted", mono && "font-mono text-xs")}>
      {children}
    </span>
  )
}

export function Toggle({
  checked,
  onChange,
  label,
  labelledBy,
  disabled = false,
}: {
  checked: boolean
  onChange: (on: boolean) => void
  label?: string
  labelledBy?: string
  disabled?: boolean
}) {
  return (
    <button type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      aria-labelledby={labelledBy}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        "relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors focus-visible:status-focused-field disabled:opacity-50",
        checked ? "bg-accent hover:bg-accent-hover" : "bg-default-hover hover:bg-segment"
      )}
    >
      <span
        className={cn(
          "absolute top-0.5 left-0.5 size-4 rounded-full transition-transform",
          checked ? "translate-x-4 bg-accent-foreground" : "bg-foreground/70"
        )}
      />
    </button>
  )
}

/** A choice of a few: the composer's mode pill. */
export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
  disabled = false,
}: {
  value: T
  options: { id: T; label: string; title?: string }[]
  onChange: (v: T) => void
  label: string
  disabled?: boolean
}) {
  return (
    <div className="flex h-8 shrink-0 items-center rounded-full bg-default/60 p-0.5" role="radiogroup" aria-label={label}>
      {options.map((o) => (
        <button type="button"
          key={o.id}
          role="radio"
          aria-checked={value === o.id}
          title={o.title}
          disabled={disabled}
          onClick={() => onChange(o.id)}
          className={cn(
            "text-dense h-7 rounded-full px-3 whitespace-nowrap transition-colors focus-visible:status-focused-field disabled:opacity-50",
            value === o.id ? "bg-default-hover text-foreground" : "text-muted enabled:hover:text-foreground"
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  )
}

/** A choice of many: a button that opens a list, like the model picker. */
export function Choice<T extends string | null>({
  value,
  options,
  onChange,
  label,
  disabled = false,
}: {
  value: T
  options: { id: T; label: string; note?: string | null }[]
  onChange: (v: T) => void
  label: string
  disabled?: boolean
}) {
  const [show, setShow] = useState(false)
  const close = useCallback(() => setShow(false), [])
  const current = options.find((o) => o.id === value)
  return (
    <div className="relative">
      <button type="button"
        onClick={() => setShow(!show)}
        aria-expanded={show}
        aria-label={`${label}: ${current?.label ?? "None"}`}
        disabled={disabled}
        className={btn("secondary", "sm", "max-w-64 gap-1.5")}
      >
        <span className="min-w-0 truncate">{current?.label ?? "None"}</span>
        <ChevronDown className="size-3.5 shrink-0 text-muted" />
      </button>
      <Popover open={show} onClose={close} side="bottom" align="end" className="w-64">
        <div role="listbox" aria-label={label} className="flex max-h-[min(20rem,50vh)] flex-col overflow-y-auto">
          {options.map((o) => (
            <button type="button"
              key={o.id ?? "none"}
              role="option"
              aria-selected={o.id === value}
              onClick={() => {
                close()
                onChange(o.id)
              }}
              className="flex items-start gap-2.5 rounded-lg px-2.5 py-2 text-left hover:bg-default-hover focus-visible:status-focused-field"
            >
              <span className="min-w-0 flex-1">
                <span className="block text-sm">{o.label}</span>
                {o.note && <span className="text-2xs block text-muted">{o.note}</span>}
              </span>
              {o.id === value && <Check className="mt-0.5 size-3.5 shrink-0" />}
            </button>
          ))}
        </div>
      </Popover>
    </div>
  )
}

/**
 * An action that asks first, in place: the button turns into the question with the action and
 * Cancel. Escape cancels. `run` returns an error to show, or nothing.
 */
export function Confirm({
  label,
  question,
  action,
  run,
  danger = true,
  variant = "ghost",
  className,
  icon,
}: {
  label: ReactNode
  question: ReactNode
  /** The confirming button's words, like "Remove" or "Delete 214". */
  action: string
  run: () => Promise<string | void>
  danger?: boolean
  variant?: "ghost" | "secondary" | "danger-soft"
  className?: string
  icon?: ReactNode
}) {
  const [asking, setAsking] = useState(false)
  const [working, setWorking] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const confirm = useRef<HTMLButtonElement>(null)
  useEffect(() => {
    if (asking) confirm.current?.focus()
  }, [asking])
  if (!asking)
    return (
      <button type="button"
        onClick={() => {
          setError(null)
          setAsking(true)
        }}
        className={btn(variant, "sm", cn("shrink-0 gap-1.5", variant === "ghost" && "text-muted", className))}
      >
        {icon}
        {label}
      </button>
    )
  return (
    <div
      className="flex min-w-0 flex-wrap items-center justify-end gap-x-3 gap-y-2"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.preventDefault()
          e.stopPropagation()
          setAsking(false)
        }
      }}
    >
      <span className="text-dense min-w-0">{question}</span>
      <span className="flex shrink-0 items-center gap-1">
        <button type="button"
          ref={confirm}
          disabled={working}
          onClick={() => {
            setWorking(true)
            setError(null)
            void run()
              .then((e) => {
                if (e) setError(e)
                else setAsking(false)
              })
              .catch((e) => setError(message(e)))
              .finally(() => setWorking(false))
          }}
          className={btn(danger ? "danger-soft" : "primary", "sm", "gap-1.5")}
        >
          {working && <Spinner className="size-3.5" />}
          {action}
        </button>
        <button type="button" onClick={() => setAsking(false)} className={btn("ghost", "sm")}>
          Cancel
        </button>
      </span>
      {error && <span className="text-dense w-full text-right text-danger">{error}</span>}
    </div>
  )
}

/** A call's answer for a section: loading, failed with Retry, or there. */
export type Load<T> = {
  data: T | undefined
  error: string | null
  loading: boolean
  reload: () => void
  set: (data: T) => void
}

export function useLoad<T>(read: () => Promise<T>, deps: unknown[]): Load<T> {
  const [data, setData] = useState<T | undefined>(undefined)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(true)
  const n = useRef(0)
  const run = useCallback(() => {
    const at = ++n.current
    setLoading(true)
    setError(null)
    read().then(
      (d) => {
        if (at !== n.current) return
        setData(d)
        setLoading(false)
      },
      (e) => {
        if (at !== n.current) return
        setError(message(e))
        setLoading(false)
      }
    )
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps)
  useEffect(run, [run])
  return { data, error, loading, reload: run, set: setData }
}

/**
 * Renders a load: its rows' outline while the first read runs (so nothing jumps when it lands),
 * the error with Retry, or the content. A read again keeps the content shown.
 */
export function Loaded<T>({
  load,
  rows = 3,
  bare = false,
  children,
}: {
  load: Load<T>
  rows?: number
  /** Under a heading of its own: no room kept for a group label. */
  bare?: boolean
  children: (data: T) => ReactNode
}) {
  if (load.data !== undefined) return <>{children(load.data)}</>
  if (load.error) return <Failed error={load.error} retry={load.reload} bare={bare} />
  return <Pending rows={rows} bare={bare} />
}

export function Pending({ rows = 3, bare = false }: { rows?: number; bare?: boolean }) {
  return (
    <div className="flex flex-col gap-2" aria-busy="true" aria-label="Loading">
      {!bare && <div className="h-6" />}
      <div className="flex flex-col divide-y divide-separator rounded-xl bg-surface">
        {Array.from({ length: rows }, (_, i) => (
          <div key={i} className="flex h-12 items-center px-4">
            {i === 0 && <Spinner />}
          </div>
        ))}
      </div>
    </div>
  )
}

export function Failed({ error, retry, bare = false }: { error: string; retry: () => void; bare?: boolean }) {
  return (
    <div className="flex flex-col gap-2">
      {!bare && <div className="h-6" />}
      <div className="flex items-center gap-4 rounded-xl bg-surface px-4 py-3">
        <p className="text-dense min-w-0 flex-1 text-danger select-text">{error}</p>
        <button type="button" onClick={retry} className={btn("secondary", "sm", "shrink-0")}>
          Retry
        </button>
      </div>
    </div>
  )
}

/** The folder an agent section reads, small, above it. */
export function FolderLine({ directory, own }: { directory: string; own: boolean }) {
  return (
    <p className="text-2xs -mb-4 flex min-w-0 items-center gap-1.5 px-1 text-muted" title={directory}>
      <FolderClosed className="size-3 shrink-0" strokeWidth={1.75} />
      <span className="truncate">{own ? "No project · the app’s own folder" : folderName(directory)}</span>
    </p>
  )
}

/** Bytes as the file manager would say them. */
export function bytes(n: number) {
  if (n < 1024) return `${n} B`
  const units = ["KB", "MB", "GB", "TB"]
  let v = n / 1024
  let i = 0
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024
    i++
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`
}

/** "17 Nov", "17 Nov 2025" outside this year. */
export function day(ms: number) {
  const d = new Date(ms)
  return d.toLocaleDateString("en-GB", {
    day: "numeric",
    month: "short",
    year: d.getFullYear() === new Date().getFullYear() ? undefined : "numeric",
  })
}

/** A home folder as `~`. */
export function tilde(path: string) {
  return path.replace(/^(\/home\/[^/]+|\/Users\/[^/]+|[A-Z]:\\Users\\[^\\]+)/i, "~")
}

/** A row that opens a page in the browser; the address says where it goes. */
export function LinkRow({ label, url }: { label: string; url: string }) {
  return (
    <button type="button"
      onClick={() => openExternal(url)}
      className="group flex min-h-12 w-full items-center gap-4 px-4 py-2.5 text-left transition-colors first:rounded-t-xl last:rounded-b-xl hover:bg-default/40 focus-visible:status-focused-field"
    >
      <span className="min-w-0 flex-1 text-sm">{label}</span>
      <span className="text-2xs min-w-0 truncate text-muted">{url.replace(/^https:\/\//, "").replace(/\/$/, "")}</span>
      <ArrowUpRight className="size-3.5 shrink-0 text-muted group-hover:text-foreground" />
    </button>
  )
}
