/**
 * A project file's rules or downloads that aren't allowed, in the Settings section they belong to:
 * the file, what allowing would change, and the answer, as the project groups of Hooks and MCP
 * servers do. The conversation's card asks about the same items.
 */
import { useState, type ReactNode } from "react"

import type { TrustItem } from "../bridge"
import { answerTrust, remoteText, ruleText, scopeLabel, useTrust } from "../hooks-card"
import { btn, cn, Spinner } from "../ui"
import { OpenPath } from "./agent"
import { Group, message, Row } from "./controls"
import { shortPath } from "./hooks"

const HINT: Record<"permissions" | "remote", { waiting: string; denied: string }> = {
  permissions: {
    waiting: "These change what the agent may do without asking you. Until you allow them, your own rules decide.",
    denied: "Not applied: your own rules decide. Allow if you trust this project.",
  },
  remote: {
    waiting: "The app fetches these each time it opens this project, once you allow it.",
    denied: "Not fetched. Allow if you trust this project.",
  },
}

/** One group per project file whose `kind` waits or was refused; nothing when all are allowed. */
export function ProjectTrust({ directory, kind }: { directory: string; kind: "permissions" | "remote" }) {
  const trust = useTrust(directory)
  const items = (trust.data ?? []).filter((i) => i.kind === kind && i.decision !== "allowed")
  return (
    <>
      {items.map((i) => (
        <ProjectItem key={`${i.path}:${i.kind}`} item={i} directory={directory} />
      ))}
    </>
  )
}

function ProjectItem({ item, directory }: { item: TrustItem; directory: string }) {
  const [busy, setBusy] = useState<boolean | null>(null)
  const [error, setError] = useState<string | null>(null)
  const refused = item.decision === "denied"
  const kind = item.kind as "permissions" | "remote"
  const answer = (allow: boolean) => {
    setBusy(allow)
    setError(null)
    answerTrust(directory, [item], allow)
      .catch((e) => setError(message(e)))
      .finally(() => setBusy(null))
  }
  return (
    <Group
      label={
        <>
          This project <span className="ml-2 font-mono font-normal">{shortPath(item.path, directory)}</span>
        </>
      }
      aside={<OpenPath path={item.path} small />}
    >
      <Row
        label={
          <span className="flex items-center gap-2">
            <span className={cn("size-1.5 shrink-0 rounded-full", refused ? "bg-muted/60" : "bg-warning")} aria-hidden />
            {refused ? "Not allowed" : "Waiting for you"}
          </span>
        }
        hint={HINT[kind][refused ? "denied" : "waiting"]}
      >
        {busy !== null && <Spinner />}
        <button disabled={busy !== null} onClick={() => answer(true)} className={btn(refused ? "secondary" : "primary", "sm")}>
          Allow
        </button>
        {!refused && (
          <button disabled={busy !== null} onClick={() => answer(false)} className={btn("ghost", "sm", "text-muted")}>
            Don’t allow
          </button>
        )}
      </Row>
      {item.entries.map((e, i) => (
        <EntryRow
          key={i}
          scope={kind === "permissions" ? scopeLabel(e.label) : e.label === "instructions" ? "Instructions" : e.label}
          text={kind === "permissions" ? ruleText(e.command) : remoteText(e)}
        />
      ))}
      {error && <p className="text-dense px-4 py-2.5 text-danger select-text">{error}</p>}
    </Group>
  )
}

function EntryRow({ scope, text }: { scope: ReactNode; text: ReactNode }) {
  return (
    <div className="text-dense flex min-h-11 flex-wrap items-baseline gap-x-4 gap-y-0.5 px-4 py-2.5">
      <span className="w-32 shrink-0 text-muted">{scope}</span>
      <span className="min-w-[min(100%,16rem)] flex-1 break-words">{text}</span>
    </div>
  )
}

