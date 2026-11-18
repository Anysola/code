/**
 * An MCP server's form (elicitation), in the conversation like a question: one control per
 * field, required marks, plain-word checks as the user goes, and Submit / Decline / dismiss.
 */
import { Check, ExternalLink, X } from "lucide-react"
import { useId, useState } from "react"

import type { FormField, FormRequest } from "./bridge"
import { Choice, Toggle } from "./settings/controls"
import { openExternal, replyForm, useStore } from "./store"
import { btn, cn, Spinner, Tooltip } from "./ui"

const FIELD =
  "h-9 w-full rounded-lg bg-background/60 px-3 text-sm outline-none placeholder:text-field-placeholder focus-visible:status-focused-field"

/** What a field starts with: its default, in the shape its control holds. */
function initial(f: FormField): unknown {
  const d = f.default
  if (f.type === "boolean") return typeof d === "boolean" ? d : false
  if (f.type === "multiselect") return Array.isArray(d) ? d.filter((x) => typeof x === "string") : []
  if (d === undefined || d === null) return ""
  if (f.format === "date-time" && typeof d === "string") return toLocal(d)
  return String(d)
}

/** An ISO time as `datetime-local` shows it, in this computer's time zone. */
function toLocal(iso: string) {
  const t = new Date(iso)
  if (Number.isNaN(t.getTime())) return ""
  const pad = (n: number) => String(n).padStart(2, "0")
  return `${t.getFullYear()}-${pad(t.getMonth() + 1)}-${pad(t.getDate())}T${pad(t.getHours())}:${pad(t.getMinutes())}`
}

const EMAIL = /^[^\s@]+@[^\s@]+\.[^\s@]+$/

/** What's wrong with a field's value, in plain words; null when it's fine (or still empty and optional). */
function problem(f: FormField, v: unknown): string | null {
  if (f.type === "external" || f.type === "boolean") return null
  if (f.type === "multiselect") {
    const n = (v as string[]).length
    if (n === 0 && !f.required && !f.minItems) return null
    if (f.required && n === 0) return "Pick at least one."
    if (f.minItems && n < f.minItems) return `Pick at least ${f.minItems}.`
    if (f.maxItems && n > f.maxItems) return `Pick at most ${f.maxItems}.`
    return null
  }
  const s = String(v ?? "").trim()
  if (!s) return f.required ? "Required." : null
  if (f.type === "number" || f.type === "integer") {
    const n = Number(s)
    if (!Number.isFinite(n)) return "Enter a number."
    if (f.type === "integer" && !Number.isInteger(n)) return "Enter a whole number."
    if (f.minimum !== undefined && n < f.minimum) return `The lowest is ${f.minimum}.`
    if (f.maximum !== undefined && n > f.maximum) return `The highest is ${f.maximum}.`
    return null
  }
  if (f.options?.length && f.custom === false && !f.options.some((o) => o.value === s)) return "Pick one of the choices."
  if (f.format === "email" && !EMAIL.test(s)) return "Enter an email address, like name@example.com."
  if (f.format === "uri") {
    try {
      new URL(s)
    } catch {
      return "Enter a full address, starting with https://."
    }
  }
  if (f.minLength && s.length < f.minLength) return `Use at least ${f.minLength} characters.`
  if (f.maxLength && s.length > f.maxLength) return `Use at most ${f.maxLength} characters.`
  return null
}

/** The values to send, typed: numbers as numbers, an empty optional field left out. */
function typed(fields: FormField[], values: Record<string, unknown>) {
  const out: Record<string, unknown> = {}
  for (const f of fields) {
    const v = values[f.key]
    if (f.type === "external") continue
    if (f.type === "boolean") out[f.key] = !!v
    else if (f.type === "multiselect") {
      if ((v as string[]).length || f.required) out[f.key] = v
    } else {
      const s = String(v ?? "").trim()
      if (!s) continue
      if (f.type === "number" || f.type === "integer") out[f.key] = Number(s)
      else if (f.format === "date-time") out[f.key] = new Date(s).toISOString()
      else out[f.key] = s
    }
  }
  return out
}

export function FormCard({ form }: { form: FormRequest }) {
  const [values, setValues] = useState<Record<string, unknown>>(() =>
    Object.fromEntries(form.fields.map((f) => [f.key, initial(f)]))
  )
  /** Fields the user has left (or tried to submit past): their problems show from then on. */
  const [touched, setTouched] = useState<Set<string>>(new Set())
  const [tried, setTried] = useState(false)
  const external = form.fields.find((f) => f.type === "external")
  const problems = Object.fromEntries(form.fields.map((f) => [f.key, problem(f, values[f.key])]))
  const ok = form.fields.every((f) => !problems[f.key])
  const set = (key: string, v: unknown) => setValues((all) => ({ ...all, [key]: v }))
  const leave = (key: string) => setTouched((t) => (t.has(key) ? t : new Set(t).add(key)))
  // The answer is on its way: the card waits for it, with what was typed still in it.
  const sending = useStore((s) => s.sending.has(form.formId))
  const submit = () => {
    setTried(true)
    if (ok) void replyForm(form, { action: "accept", values: typed(form.fields, values) })
  }
  return (
    <div className="flex flex-col gap-4 rounded-xl bg-surface p-4" aria-label={`${form.server} is requesting input`} role="group">
      <div className="flex items-start gap-2">
        <span className="mt-1.5 size-1.5 shrink-0 rounded-full bg-warning" />
        <div className="min-w-0 flex-1">
          <div className="text-sm">
            {form.server} is requesting input
            {form.title && form.title !== `${form.server} is requesting input` && (
              <span className="text-muted"> · {form.title}</span>
            )}
          </div>
          {form.message && <p className="text-dense mt-1 whitespace-pre-wrap text-muted">{form.message}</p>}
        </div>
        <Tooltip label="Dismiss" side="top">
          <button
            onClick={() => void replyForm(form, { action: "cancel" })}
            disabled={sending}
            aria-label="Dismiss"
            className={btn("ghost", "sm", "-mt-1 -mr-1 size-7 shrink-0 text-muted", true)}
          >
            <X className="size-3.5" />
          </button>
        </Tooltip>
      </div>
      {external ? (
        <ExternalStep field={external} />
      ) : (
        <>
          {/* Capped, so Submit stays on screen however many fields come. */}
          <div className="-m-1 flex max-h-[min(26rem,50vh)] flex-col gap-4 overflow-y-auto p-1">
            {form.fields.map((f) => (
              <Field
                key={f.key}
                field={f}
                value={values[f.key]}
                onChange={(v) => set(f.key, v)}
                onLeave={() => leave(f.key)}
                problem={tried || touched.has(f.key) ? problems[f.key] : null}
              />
            ))}
          </div>
          <div className="flex items-center gap-2">
            <button onClick={submit} disabled={(tried && !ok) || sending} className={btn("primary", "sm")}>
              Submit
            </button>
            {sending && <Spinner />}
            <button onClick={() => void replyForm(form, { action: "decline" })} disabled={sending} className={btn("ghost", "sm", "ml-auto text-muted")}>
              Decline
            </button>
          </div>
        </>
      )}
    </div>
  )
}

/** The URL kind: the page opens in the browser; the server ends the request itself. */
function ExternalStep({ field }: { field: FormField }) {
  const [opened, setOpened] = useState(false)
  const url = field.url ?? ""
  let host = url
  try {
    host = new URL(url).host
  } catch {
    // Shown as given.
  }
  return (
    <div className="flex flex-col gap-3">
      {field.description && <p className="text-dense text-muted">{field.description}</p>}
      <div className="flex flex-wrap items-center gap-3">
        <button
          onClick={() => {
            openExternal(url)
            setOpened(true)
          }}
          disabled={!url}
          className={btn("primary", "sm", "gap-2")}
        >
          <ExternalLink className="size-3.5" />
          {field.title || "Open"}
        </button>
        <span className="text-2xs min-w-0 truncate font-mono text-muted" title={url}>
          {host}
        </span>
      </div>
      {opened && <p className="text-2xs text-muted">Waiting for you to finish in the browser. This closes on its own when you’re done.</p>}
    </div>
  )
}

function Field({
  field: f,
  value,
  onChange,
  onLeave,
  problem,
}: {
  field: FormField
  value: unknown
  onChange: (v: unknown) => void
  onLeave: () => void
  problem: string | null
}) {
  const id = useId()
  const hint = f.description ? <span className="text-2xs block text-muted">{f.description}</span> : null
  const label = (
    <span className="text-sm">
      {f.title || f.key}
      {f.required && (
        <span className="text-muted" aria-label="required">
          {" "}
          *
        </span>
      )}
    </span>
  )
  const error = problem && (
    <span id={`${id}-p`} className="text-2xs text-warning">
      {problem}
    </span>
  )
  if (f.type === "boolean")
    return (
      <div className="flex items-start gap-3">
        <span className="min-w-0 flex-1" id={`${id}-l`}>
          {label}
          {hint}
        </span>
        <Toggle checked={!!value} onChange={onChange} labelledBy={`${id}-l`} />
      </div>
    )
  const options = f.options ?? []
  if (f.type === "multiselect" || (options.length > 0 && f.custom === false && options.length <= 5)) {
    const multiple = f.type === "multiselect"
    const picked = multiple ? (value as string[]) : [String(value ?? "")]
    return (
      <fieldset className="flex flex-col gap-1.5" onBlur={onLeave}>
        <legend className="contents">
          {label}
          {hint}
        </legend>
        <div className="mt-1 flex flex-col gap-1" role={multiple ? "group" : "radiogroup"}>
          {options.map((o) => {
            const on = picked.includes(o.value)
            return (
              <button
                key={o.value}
                type="button"
                role={multiple ? "checkbox" : "radio"}
                aria-checked={on}
                onClick={() => {
                  if (!multiple) return onChange(o.value)
                  onChange(on ? picked.filter((v) => v !== o.value) : [...picked, o.value])
                  onLeave()
                }}
                className={cn(
                  "flex items-center gap-3 rounded-lg px-3 py-2 text-left transition-colors focus-visible:status-focused-field",
                  on ? "bg-default" : "hover:bg-default/60"
                )}
              >
                <span
                  className={cn(
                    "flex size-4 shrink-0 items-center justify-center",
                    multiple ? "rounded" : "rounded-full",
                    on ? "bg-accent text-accent-foreground" : "border-[1.5px] border-muted/70"
                  )}
                >
                  {on && <Check className="size-3" strokeWidth={2.5} />}
                </span>
                <span className="text-sm">{o.label}</span>
              </button>
            )
          })}
        </div>
        {error}
      </fieldset>
    )
  }
  if (options.length > 0 && f.custom === false)
    return (
      <div className="flex items-center gap-3">
        <span className="min-w-0 flex-1">
          {label}
          {hint}
          {error}
        </span>
        <Choice
          value={String(value ?? "") || null}
          options={[...(f.required ? [] : [{ id: null, label: "None" }]), ...options.map((o) => ({ id: o.value as string | null, label: o.label }))]}
          onChange={(v) => {
            onChange(v ?? "")
            onLeave()
          }}
          label={f.title || f.key}
        />
      </div>
    )
  const number = f.type === "number" || f.type === "integer"
  const type = number
    ? "number"
    : f.format === "email"
      ? "email"
      : f.format === "uri"
        ? "url"
        : f.format === "date"
          ? "date"
          : f.format === "date-time"
            ? "datetime-local"
            : "text"
  const range =
    number && (f.minimum !== undefined || f.maximum !== undefined)
      ? f.minimum !== undefined && f.maximum !== undefined
        ? `${f.minimum} to ${f.maximum}`
        : f.minimum !== undefined
          ? `${f.minimum} or more`
          : `Up to ${f.maximum}`
      : null
  return (
    <label className="flex flex-col gap-1.5" htmlFor={id}>
      <span>
        {label}
        {hint}
      </span>
      <input
        id={id}
        type={type}
        value={String(value ?? "")}
        onChange={(e) => onChange(e.target.value)}
        onBlur={onLeave}
        min={f.minimum}
        max={f.maximum}
        step={f.type === "integer" ? 1 : number ? "any" : undefined}
        list={options.length ? `${id}-o` : undefined}
        placeholder={range ?? (f.format === "email" ? "name@example.com" : f.format === "uri" ? "https://" : undefined)}
        aria-invalid={!!problem}
        aria-describedby={problem ? `${id}-p` : undefined}
        className={cn(FIELD, "[color-scheme:inherit]")}
      />
      {options.length > 0 && (
        <datalist id={`${id}-o`}>
          {options.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </datalist>
      )}
      {error}
    </label>
  )
}
