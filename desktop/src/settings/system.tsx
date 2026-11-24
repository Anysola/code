/** Discord, Data and About. */
import { useEffect, useState } from "react"

import type { DataInfo, DiscordStatus, Prefs, WorktreeWithWork } from "../bridge"
import { setPref, usePrefs } from "../prefs"
import { currentBridge, reloadSessions, useStore } from "../store"
import { btn, cn, folderName, Mark, Spinner } from "../ui"
import { OpenPath } from "./agent"
import { VersionRow } from "./app"
import { bytes, Confirm, day, Group, LinkRow, Loaded, message, Row, Section, tilde, Toggle, useLoad, Value } from "./controls"

// ---------------------------------------------------------------------------
// Discord
// ---------------------------------------------------------------------------

const DISCORD: Record<DiscordStatus["state"], { word: string; mark: string }> = {
  connected: { word: "Connected", mark: "bg-success" },
  waiting: { word: "Waiting for Discord", mark: "bg-warning" },
  off: { word: "Off", mark: "bg-muted/40" },
}

export function DiscordSection() {
  const d = usePrefs((p) => p.discord)
  const set = (patch: Partial<Prefs["discord"]>) => void setPref("discord", patch)
  const [status, setStatus] = useState<DiscordStatus | null>(null)
  const [statusError, setStatusError] = useState<string | null>(null)
  // Read while the section is open: Discord starts and stops on its own.
  useEffect(() => {
    let live = true
    const read = () =>
      currentBridge()
        .discordStatus()
        .then(
          (s) => {
            if (!live) return
            setStatus(s)
            setStatusError(null)
          },
          (e) => live && setStatusError(message(e))
        )
    void read()
    const t = setInterval(read, 3000)
    return () => {
      live = false
      clearInterval(t)
    }
  }, [d.enabled, d.applicationId])
  const shows: { key: "project" | "session" | "model" | "elapsed"; label: string }[] = [
    { key: "project", label: "Project name" },
    { key: "session", label: "Session title" },
    { key: "model", label: "Model" },
    { key: "elapsed", label: "Time in the session" },
  ]
  const state = status ? DISCORD[status.state] : null
  return (
    <Section title="Discord">
      <Group>
        <Row label="Show what I’m working on in Discord" id="discord-on">
          <Toggle labelledBy="discord-on" checked={d.enabled} onChange={(enabled) => set({ enabled })} />
        </Row>
        <Row
          label="Status"
          hint={statusError ? <span className="text-danger">{statusError}</span> : (status?.detail ?? undefined)}
        >
          {state ? (
            <span className="text-dense flex items-center gap-2 text-muted">
              <span className={cn("size-1.5 rounded-full", state.mark)} aria-hidden />
              {state.word}
            </span>
          ) : (
            !statusError && <Spinner />
          )}
        </Row>
      </Group>
      <Group label="It may show">
        {shows.map((s) => (
          <Row key={s.key} label={s.label} id={`discord-${s.key}`} className={cn("min-h-11 py-2", !d.enabled && "opacity-60")}>
            <Toggle labelledBy={`discord-${s.key}`} checked={d[s.key]} disabled={!d.enabled} onChange={(on) => set({ [s.key]: on })} />
          </Row>
        ))}
      </Group>
      <Preview prefs={d} />
      <ApplicationId value={d.applicationId} onChange={(applicationId) => set({ applicationId })} />
    </Section>
  )
}

/** The status card as Discord would draw it, from what is open now. */
function Preview({ prefs: d }: { prefs: Prefs["discord"] }) {
  const open = useStore((s) => s.open)
  const scratch = useStore((s) => s.scratch)
  const busy = useStore((s) => (s.open?.sessionId ? s.busy.has(s.open.sessionId) : false))
  const project = open ? (scratch && open.directory === scratch ? "No project" : folderName(open.directory)) : null
  const lines = open
    ? [
        d.session ? open.title : busy ? "Working" : "In a session",
        [d.project && project, d.model && open.info?.model].filter(Boolean).join(" · "),
        d.elapsed ? "12:04 elapsed" : null,
      ].filter(Boolean)
    : ["Not in a session"]
  return (
    <Group label="Preview" note={d.enabled ? "Built from the session open now." : "Turned off: Discord shows nothing."}>
      <div className={cn("flex items-center gap-3.5 p-4", !d.enabled && "opacity-50")}>
        <div className="flex size-14 shrink-0 items-center justify-center rounded-xl bg-default">
          <Mark className="h-5 text-foreground" />
        </div>
        <div className="flex min-w-0 flex-col gap-0.5">
          <span className="text-dense font-medium">Anysola Code</span>
          {lines.map((l, i) => (
            <span key={i} className={cn("text-2xs truncate", i === 0 ? "text-foreground/85" : "text-muted")}>
              {l}
            </span>
          ))}
        </div>
      </div>
    </Group>
  )
}

function ApplicationId({ value, onChange }: { value: string | null; onChange: (v: string | null) => void }) {
  const [text, setText] = useState(value ?? "")
  useEffect(() => setText(value ?? ""), [value])
  const bad = text.trim() !== "" && !/^\d+$/.test(text.trim())
  const commit = () => {
    const v = text.trim()
    if (bad) return
    if ((v || null) !== value) onChange(v || null)
  }
  return (
    <Group note="Optional. Leave it empty to show Anysola Code’s own card. An application of yours from the Discord Developer Portal shows its name instead.">
      <Row
        label="Your own application ID"
        hint={bad ? <span className="text-danger">Only digits: it’s the number the Developer Portal shows.</span> : undefined}
      >
        <input
          value={text}
          onChange={(e) => setText(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => {
            if (e.key === "Enter") commit()
            if (e.key === "Escape") setText(value ?? "")
          }}
          inputMode="numeric"
          placeholder="Anysola Code’s own"
          aria-label="Discord application ID"
          aria-invalid={bad}
          spellCheck={false}
          className="text-dense h-8 w-52 rounded-lg bg-default px-2.5 font-mono tabular-nums outline-none placeholder:font-sans placeholder:text-field-placeholder focus-visible:status-focused-field aria-invalid:bg-danger/10"
        />
      </Row>
    </Group>
  )
}

// ---------------------------------------------------------------------------
// Data
// ---------------------------------------------------------------------------

const CACHES: Record<DataInfo["caches"][number]["id"], string> = {
  "tool-output": "Saved tool output",
  cache: "Downloads",
  shell: "Saved command output",
}

/** The import's time: seconds or milliseconds, whichever the number is. */
const when = (at: number) => (at < 1e12 ? at * 1000 : at)

export function DataSection() {
  const load = useLoad(() => currentBridge().dataInfo(), [])
  return (
    <Section title="Data">
      <Loaded load={load} rows={4}>
        {(data) => (
          <>
            <Group
              note={
                data.imported
                  ? `${data.imported.sessions} session${data.imported.sessions === 1 ? " was" : "s were"} imported from an OpenCode install on ${day(when(data.imported.at))}.`
                  : undefined
              }
            >
              <Row label="Data folder" hint={<span className="font-mono text-xs">{tilde(data.folder)}</span>}>
                <OpenPath path={data.folder} label="Open" />
              </Row>
              <Row
                label="Sessions"
                hint={<span className="font-mono text-xs">{tilde(data.database.path)}</span>}
              >
                <Value>
                  {data.sessions.toLocaleString("en-GB")} · {bytes(data.database.bytes)}
                </Value>
              </Row>
              <Row label="Log file" hint={<span className="font-mono text-xs">{tilde(data.log)}</span>}>
                <OpenPath path={data.log} label="Open" />
              </Row>
            </Group>
            <Group label="Caches" note="Clearing a cache keeps your sessions and settings.">
              {data.caches.map((c) => (
                <Cache key={c.id} cache={c} onData={load.set} />
              ))}
            </Group>
            <Export />
            <Group label="Delete">
              <Row label="Delete all sessions" hint="Sessions with a turn running or being applied are kept">
                <DeleteAll count={data.sessions} onDone={() => load.reload()} />
              </Row>
            </Group>
          </>
        )}
      </Loaded>
    </Section>
  )
}

function Cache({ cache: c, onData }: { cache: DataInfo["caches"][number]; onData: (d: DataInfo) => void }) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  return (
    <Row
      label={CACHES[c.id] ?? c.id}
      hint={error ? <span className="text-danger">{error}</span> : <span className="font-mono text-xs">{tilde(c.path)}</span>}
    >
      <Value>{c.bytes === 0 ? "Empty" : bytes(c.bytes)}</Value>
      <button
        disabled={busy || c.bytes === 0}
        onClick={() => {
          setBusy(true)
          setError(null)
          currentBridge()
            .dataClear(c.id)
            .then(onData, (e) => setError(message(e)))
            .finally(() => setBusy(false))
        }}
        className={btn("secondary", "sm", "gap-1.5")}
      >
        {busy && <Spinner className="size-3.5" />}
        Clear
      </button>
    </Row>
  )
}

function Export() {
  const open = useStore((s) => s.open)
  const [result, setResult] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  useEffect(() => {
    setResult(null)
    setError(null)
  }, [open?.sessionId])
  if (!open?.sessionId) return null
  const sessionId = open.sessionId
  return (
    <Group>
      <Row
        label="Export this session"
        hint={
          error ? (
            <span className="text-danger">{error}</span>
          ) : result ? (
            <>
              Saved to <span className="font-mono text-xs">{tilde(result)}</span>
            </>
          ) : (
            <span className="truncate">{open.title}, as JSON</span>
          )
        }
      >
        <button
          disabled={busy}
          onClick={() => {
            setBusy(true)
            setError(null)
            currentBridge()
              .exportSession(open.directory, sessionId)
              .then((path) => path && setResult(path), (e) => setError(message(e)))
              .finally(() => setBusy(false))
          }}
          className={btn("secondary", "sm", "gap-1.5")}
        >
          {busy && <Spinner className="size-3.5" />}
          Export…
        </button>
      </Row>
    </Group>
  )
}

/** What Delete all left: the sessions kept because a turn runs, and those kept while being applied. */
function deletedText(deleted: number, kept: number, applying: number) {
  const running = kept - applying
  const parts = [`Deleted ${deleted.toLocaleString("en-GB")}.`]
  if (running > 0) parts.push(`${running} kept: ${running === 1 ? "a turn is" : "turns are"} running.`)
  if (applying > 0)
    parts.push(`${applying} kept: ${applying === 1 ? "its changes were" : "their changes were"} being applied.`)
  return parts.join(" ")
}

function DeleteAll({ count, onDone }: { count: number; onDone: () => void }) {
  const [result, setResult] = useState<string | null>(null)
  // The worktrees whose changes aren't applied, read when the question opens; undefined while reading, null when it couldn't be read.
  const [work, setWork] = useState<WorktreeWithWork[] | null | undefined>(undefined)
  if (result) return <span className="text-dense text-muted">{result}</span>
  if (count === 0) return <Value>No sessions</Value>
  const n = count.toLocaleString("en-GB")
  return (
    <Confirm
      label="Delete all…"
      variant="danger-soft"
      onAsk={() => {
        setWork(undefined)
        currentBridge()
          .worktreesWithWork()
          .then(setWork, () => setWork(null))
      }}
      waiting={work === undefined}
      question={
        work && work.length > 0 ? (
          <span className="flex flex-col gap-2 text-left">
            <span>
              Delete all {n}? This can’t be undone. {work.length === 1 ? "This worktree goes" : "These worktrees go"} with
              changes that aren’t applied:
            </span>
            <ul className="flex max-h-40 flex-col gap-0.5 overflow-y-auto">
              {work.map((w) => (
                <li key={w.sessionId} className="flex min-w-0 items-baseline gap-3 rounded-md bg-default/30 px-2.5 py-1.5">
                  <span className="min-w-0 flex-1 truncate" title={w.title}>
                    {w.title}
                  </span>
                  <span className="text-2xs shrink-0 text-muted tabular-nums">
                    {w.files} file{w.files === 1 ? "" : "s"} not applied
                  </span>
                </li>
              ))}
            </ul>
          </span>
        ) : (
          `Delete all ${n}? This can’t be undone.`
        )
      }
      action={`Delete ${n}`}
      run={async () => {
        try {
          const { deleted, kept, applying } = await currentBridge().deleteAllSessions()
          setResult(deletedText(deleted, kept, applying ?? 0))
          void reloadSessions()
          onDone()
        } catch (e) {
          return message(e)
        }
      }}
    />
  )
}

// ---------------------------------------------------------------------------
// About
// ---------------------------------------------------------------------------

const LINKS: [string, string][] = [
  ["Changelog", "https://www.anysola.com/changelog"],
  ["Help", "https://www.anysola.com/help"],
  ["Support", "https://account.anysola.com/support"],
  ["Source", "https://github.com/Anysola/code"],
  ["Status", "https://www.anysola.com/status"],
  ["Terms", "https://www.anysola.com/terms"],
  ["Privacy", "https://www.anysola.com/privacy"],
]

export function AboutSection() {
  const load = useLoad(() => currentBridge().about(), [])
  return (
    <Section title="About">
      <Loaded load={load} rows={2}>
        {(a) => (
          <Group>
            <VersionRow version={a.version} />
            <Row label="System">
              <Value>
                {a.os} · {a.arch}
              </Value>
            </Row>
          </Group>
        )}
      </Loaded>
      <Group>
        {LINKS.map(([label, url]) => (
          <LinkRow key={label} label={label} url={url} />
        ))}
      </Group>
    </Section>
  )
}
