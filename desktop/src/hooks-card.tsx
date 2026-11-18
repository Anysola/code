/**
 * A project's hooks that wait for the user's answer, in the conversation of its folder: they
 * run nothing until allowed. The card asks without taking focus or keys from the message box.
 */
import { useState } from "react"

import type { HookFile, Hooks } from "./bridge"
import { defaultTimeout, shortPath, useHooks, when } from "./settings/hooks"
import { currentBridge } from "./store"
import { btn, cn, Spinner } from "./ui"

/** One card per project file whose hooks wait for an answer; nothing while none do. */
export function HookTrustCards({ directory, className }: { directory: string; className?: string }) {
  const load = useHooks(directory)
  const waiting = load.data?.files.filter((f) => f.decision === null) ?? []
  if (waiting.length === 0) return null
  return (
    <div className={className}>
      {waiting.map((f) => (
        <HookTrustCard key={`${f.path}:${f.digest}`} file={f} directory={directory} onList={load.set} />
      ))}
    </div>
  )
}

function HookTrustCard({
  file: f,
  directory,
  onList,
}: {
  file: HookFile
  directory: string
  onList: (h: Hooks) => void
}) {
  const [busy, setBusy] = useState<boolean | null>(null)
  const [error, setError] = useState<string | null>(null)
  const answer = (allow: boolean) => {
    setBusy(allow)
    setError(null)
    currentBridge()
      .hooksTrust(directory, f.path, f.digest, allow)
      .then(onList, (e) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => setBusy(null))
  }
  const n = f.hooks.length
  // Hooks allowed earlier, now gone from the file: they keep running until the user accepts that,
  // so an edit can't switch a guard off by itself.
  const emptied = n === 0
  return (
    <section className="rounded-xl bg-surface p-4" aria-label="This project’s hooks">
      <div className="flex items-center gap-2 text-sm">
        <span className="size-1.5 rounded-full bg-warning" />
        {emptied ? "This project’s hooks were removed" : f.standing ? "This project’s hooks changed" : "Run this project’s hooks?"}
      </div>
      <p className="text-dense mt-1 pl-3.5 text-muted">
        {emptied
          ? "The file below no longer defines hooks. The ones you allowed before keep running until you accept that."
          : f.standing
            ? `The ones you allowed before keep running until you allow ${n === 1 ? "this one" : "these"}. Allow only if you trust the change.`
            : `${n === 1 ? "It runs a command" : `They run ${n} commands`} on this computer while the agent works. Allow only if you trust this project.`}
      </p>
      <div className="mt-3 max-h-[min(18rem,32vh)] overflow-auto rounded-lg bg-background/60 px-3 py-2.5">
        <div className={cn("text-2xs truncate font-mono text-muted", !emptied && "mb-2")} title={f.path}>
          {shortPath(f.path, directory)}
        </div>
        {!emptied && (
          <ul className="grid grid-cols-[auto_auto_minmax(0,1fr)] items-baseline gap-x-2.5 gap-y-1.5">
            {f.hooks.map((h, i) => (
              <li key={i} className="contents">
                <span className="text-dense flex flex-col whitespace-nowrap text-foreground/85">
                  {when(h.event, h.matcher)}
                  {/* A shorter limit can weaken a guard: said where it is decided. */}
                  {h.timeout !== defaultTimeout(h.event) && (
                    <span className="text-2xs text-muted">Time limit {h.timeout} s</span>
                  )}
                </span>
                <span className="text-2xs text-muted" aria-hidden>
                  →
                </span>
                <span className="font-mono text-xs break-all whitespace-pre-wrap">{h.command}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
      <div className="mt-4 flex flex-wrap items-center gap-2">
        <button onClick={() => answer(true)} disabled={busy !== null} className={btn("primary", "sm")}>
          {emptied ? "Stop running them" : "Allow"}
        </button>
        {busy !== null && <Spinner className="ml-1" />}
        <button onClick={() => answer(false)} disabled={busy !== null} className={btn("ghost", "sm", "ml-auto")}>
          {emptied ? "Keep them running" : "Don’t allow"}
        </button>
      </div>
      {error && <p className="text-dense mt-2 text-danger select-text">{error}</p>}
    </section>
  )
}
