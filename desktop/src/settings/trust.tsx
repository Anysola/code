/**
 * The user's answer on what a project file may change, in the Settings section it belongs to: the
 * state (waiting, allowed, not allowed), what that means, and the buttons that change it, as the
 * project groups of Hooks do. The conversation's card asks about the same items.
 */
import { useState, type ReactNode } from "react"

import type { TrustItem } from "../bridge"
import { answerTrust, remoteLabel, remoteText, WHEN_DOWNLOADED } from "../hooks-card"
import { btn, cn, Spinner } from "../ui"
import { OpenPath } from "./agent"
import { Group, message, Row } from "./controls"
import { shortPath } from "./hooks"

export const TRUST_STATE = {
  waiting: { word: "Waiting for you", mark: "bg-warning" },
  allowed: { word: "Allowed", mark: "bg-success" },
  denied: { word: "Not allowed", mark: "bg-muted/60" },
} as const

export const stateOf = (item: TrustItem) => TRUST_STATE[item.decision ?? "waiting"]

/** A state's dot and word, as a row's label. */
export function StateLabel({ item, children }: { item: TrustItem; children?: ReactNode }) {
  const state = stateOf(item)
  return (
    <span className="flex min-w-0 items-center gap-2">
      <span className={cn("size-1.5 shrink-0 rounded-full", state.mark)} aria-hidden />
      {children ?? state.word}
    </span>
  )
}

/**
 * One project file's kind: its state, `hint` for what that means now, and Allow / Don't allow
 * while it waits, Allow again once refused, Stop allowing once allowed. The answer reaches every
 * view that shows the item before the buttons come back.
 */
export function TrustRow({
  item,
  directory,
  hint,
  label,
}: {
  item: TrustItem
  directory: string
  hint: ReactNode
  /** In place of the state's word (the state's dot stays). */
  label?: ReactNode
}) {
  const [busy, setBusy] = useState<boolean | null>(null)
  const [error, setError] = useState<string | null>(null)
  const answer = (allow: boolean) => {
    setBusy(allow)
    setError(null)
    answerTrust(directory, [item], allow)
      .catch((e) => setError(message(e)))
      .finally(() => setBusy(null))
  }
  return (
    <>
      <Row label={<StateLabel item={item}>{label}</StateLabel>} hint={hint}>
        {busy !== null && <Spinner />}
        {item.decision !== "allowed" && (
          <button
            disabled={busy !== null}
            onClick={() => answer(true)}
            className={btn(item.decision === null ? "primary" : "secondary", "sm")}
          >
            Allow
          </button>
        )}
        {item.decision !== "denied" && (
          <button disabled={busy !== null} onClick={() => answer(false)} className={btn("ghost", "sm", "text-muted")}>
            {item.decision === null ? "Don’t allow" : "Stop allowing"}
          </button>
        )}
      </Row>
      {error && <p className="text-dense px-4 py-2.5 text-danger select-text">{error}</p>}
    </>
  )
}

// Built when shown: this module and the card's import each other, so the card's sentence isn't
// there yet when this one loads.
const remoteHint = (decision: TrustItem["decision"]) =>
  decision === "denied"
    ? "Not downloaded. Allow if you trust this project."
    : decision === "allowed"
      ? WHEN_DOWNLOADED
      : `Nothing is downloaded until you allow it. ${WHEN_DOWNLOADED}`

/** One group per project file that makes the app download something, in every state. */
export function ProjectDownloads({ directory, items }: { directory: string; items: TrustItem[] }) {
  return (
    <>
      {items.map((item) => (
        <Group
          key={item.path}
          label={
            <>
              This project <span className="ml-2 font-mono font-normal">{shortPath(item.path, directory)}</span>
            </>
          }
          aside={<OpenPath path={item.path} small />}
        >
          <TrustRow item={item} directory={directory} hint={remoteHint(item.decision)} />
          {item.entries.map((e, i) => (
            <EntryRow key={i} scope={remoteLabel(e.label)} text={remoteText(e)} />
          ))}
        </Group>
      ))}
    </>
  )
}

export function EntryRow({ scope, text }: { scope: ReactNode; text: ReactNode }) {
  return (
    <div className="text-dense flex min-h-11 flex-wrap items-baseline gap-x-4 gap-y-0.5 px-4 py-2.5">
      <span className="w-32 shrink-0 text-muted">{scope}</span>
      <span className="min-w-[min(100%,16rem)] flex-1 break-words">{text}</span>
    </div>
  )
}
