// Plain formatting shared by the TUI and the `anysola usage` / `anysola devices` commands. No UI dependencies.

const record = (value: unknown) => (value && typeof value === "object" ? (value as Record<string, unknown>) : {})
const num = (value: unknown) => (typeof value === "number" && Number.isFinite(value) ? value : 0)
const str = (value: unknown) => (typeof value === "string" ? value : "")

export function formatCredits(value: number) {
  return Math.round(value).toLocaleString("en-US")
}

export function formatDate(value: string | null | undefined) {
  if (!value) return undefined
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  return date.toLocaleString("en-US", { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" })
}

/** "team" → "Team". */
export function planName(plan: string) {
  return plan.charAt(0).toUpperCase() + plan.slice(1)
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"]

/** Calendar date as on the web: "18 Nov", with the year only when it is not this year ("5 Jan 2027"). */
export function formatShortDate(value: string | number | null | undefined, now = Date.now()) {
  if (value === null || value === undefined || value === "") return undefined
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return undefined
  const day = `${date.getDate()} ${MONTHS[date.getMonth()]}`
  return date.getFullYear() === new Date(now).getFullYear() ? day : `${day} ${date.getFullYear()}`
}

/** List time as on the web: relative within 7 days ("now", "40m ago", "5h ago", "2d ago"), then the date. */
export function formatAgo(value: number, now = Date.now()) {
  const minutes = Math.floor((now - value) / 60_000)
  if (minutes < 1) return "now"
  if (minutes < 60) return `${minutes}m ago`
  if (minutes < 24 * 60) return `${Math.floor(minutes / 60)}h ago`
  if (minutes < 7 * 24 * 60) return `${Math.floor(minutes / (24 * 60))}d ago`
  return formatShortDate(value, now)!
}

/** Left-aligned first column, right-aligned numbers. */
export function table(header: string[], rows: string[][]) {
  const all = [header, ...rows]
  const widths = header.map((_, index) => Math.max(...all.map((row) => (row[index] ?? "").length)))
  const line = (row: string[]) =>
    row.map((cell, index) => (index === 0 ? cell.padEnd(widths[index]!) : cell.padStart(widths[index]!))).join("   ")
  return [line(header), ...rows.map(line)]
}

/** Totals, then by day and by model, as aligned text tables (shared with `anysola usage`). */
export function usageLines(body: unknown) {
  const data = record(body)
  const totals = record(data.totals)
  const byDay = Array.isArray(data.byDay) ? data.byDay.map(record) : []
  const byModel = Array.isArray(data.byModel) ? data.byModel.map(record) : []
  const lines = [
    "Totals",
    ...table(
      ["", "requests", "credits"],
      [["all", formatCredits(num(totals.requests)), formatCredits(num(totals.credits))]],
    ).map((line) => `  ${line}`),
    ...(typeof totals.cacheHitRate === "number" ? [`  Cache hit rate ${Math.round(totals.cacheHitRate * 100)}%`] : []),
    "",
    "By day",
    ...table(
      ["date", "requests", "credits"],
      byDay.map((day) => [str(day.date), formatCredits(num(day.requests)), formatCredits(num(day.credits))]),
    ).map((line) => `  ${line}`),
    "",
    "By model",
    ...table(
      ["model", "requests", "credits"],
      byModel.map((model) => [str(model.model), formatCredits(num(model.requests)), formatCredits(num(model.credits))]),
    ).map((line) => `  ${line}`),
  ]
  if (typeof data.creditsAvailable === "number")
    lines.push("", `Balance ${formatCredits(data.creditsAvailable)} credits${data.renewsAt ? ` · renews ${formatDate(str(data.renewsAt))}` : ""}`)
  return lines
}
