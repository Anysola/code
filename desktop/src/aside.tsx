/**
 * A side question (`/btw`) over the composer: the question, then its answer, which is never kept
 * in the session. Escape closes it (cancelling one still loading) and gives the message box focus:
 * the composer takes that press (`useDoubleEscape`), before it would count toward a stop.
 */
import { Check, Copy, MessageCircleQuestion, RotateCcw, Wallet, X } from "lucide-react"
import { useState } from "react"

import type { AsideErrorKind } from "./bridge"
import { editBudget } from "./chrome"
import { askAside, closeAside, notify, useStore, type Aside, type Open } from "./store"
import { Prose } from "./transcript"
import { btn, Spinner, Tooltip } from "./ui"

/** One line for each reason there is no answer; null keeps the core's own words. */
const SAYS: Record<Exclude<AsideErrorKind, "cancelled">, string | null> = {
  budget: "This session’s budget is spent.",
  rate_limit: "Too many requests at once. Try again in a moment.",
  usage_limit: null,
  too_long: "The conversation is too long for a side question.",
  empty: "No answer came back.",
  other: null,
}

/** Asking again can help: a busy moment, an empty answer, anything unexpected. */
const AGAIN = new Set<AsideErrorKind>(["rate_limit", "empty", "other"])

function focusComposer() {
  requestAnimationFrame(() => document.querySelector<HTMLTextAreaElement>('textarea[aria-label="Message"]')?.focus())
}

export function AsideCard({ open }: { open: Open }) {
  const aside = useStore((s) => (open.sessionId ? (s.asides[open.sessionId] ?? null) : null))
  if (!aside || !open.sessionId) return null
  return <Card key={aside.n} aside={aside} sessionId={open.sessionId} />
}

function Card({ aside, sessionId }: { aside: Aside; sessionId: string }) {
  const [copied, setCopied] = useState(false)
  const close = () => {
    closeAside(sessionId)
    focusComposer()
  }
  const copy = () => {
    if (!aside.text) return
    void navigator.clipboard.writeText(aside.text).then(
      () => {
        setCopied(true)
        setTimeout(() => setCopied(false), 1500)
      },
      () => notify("The answer couldn’t be copied. Select its text and copy it instead.")
    )
  }
  const error = aside.error
  return (
    <section aria-label="Side question" className="mb-2 rounded-xl bg-surface/70">
      <div className="flex items-start gap-2.5 py-1 pr-1 pl-3.5">
        <MessageCircleQuestion className="mt-[0.55rem] size-3.5 shrink-0 text-muted" strokeWidth={1.75} />
        <p className="text-dense line-clamp-2 min-w-0 flex-1 py-1.5 break-words text-muted select-text" title={aside.question}>
          {aside.question}
        </p>
        <span className="flex shrink-0 items-center">
          {aside.state === "done" && (
            <Tooltip label={copied ? "Copied" : "Copy answer"} side="top">
              <button onClick={copy} aria-label={copied ? "Copied" : "Copy answer"} className={btn("ghost", "sm", "size-7 text-muted", true)}>
                {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
              </button>
            </Tooltip>
          )}
          <Tooltip label="Close" keys="Esc" side="top">
            <button onClick={close} aria-label="Close" className={btn("ghost", "sm", "size-7 text-muted", true)}>
              <X className="size-3.5" />
            </button>
          </Tooltip>
        </span>
      </div>
      {aside.state === "loading" && (
        <div className="text-dense flex items-center gap-2 px-3.5 pb-3 text-muted" role="status">
          <Spinner className="size-3" />
          Answering
        </div>
      )}
      {aside.state === "done" && aside.text && (
        // A short window keeps the composer usable: the answer scrolls inside the card.
        <div className="max-h-[min(22rem,max(4rem,calc(100vh_-_27rem)))] overflow-y-auto px-3.5 pb-3 select-text">
          <Prose text={aside.text} dense />
        </div>
      )}
      {aside.state === "error" && error && error.kind !== "cancelled" && (
        <div className="text-dense flex min-h-9 items-center gap-2 pr-1 pb-1 pl-3.5">
          <span className="min-w-0 flex-1 text-warning select-text">{SAYS[error.kind] ?? error.message}</span>
          {error.kind === "budget" && (
            <button onClick={() => editBudget(true)} className={btn("ghost", "sm", "h-7 shrink-0 gap-1.5 text-muted")}>
              <Wallet className="size-3.5" />
              Change budget
            </button>
          )}
          {AGAIN.has(error.kind) && (
            <button onClick={() => void askAside(aside.question)} className={btn("ghost", "sm", "h-7 shrink-0 gap-1.5 text-muted")}>
              <RotateCcw className="size-3.5" />
              Ask again
            </button>
          )}
        </div>
      )}
    </section>
  )
}
