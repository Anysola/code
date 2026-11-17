/** The app's own settings: General, Appearance, Conversation, Notifications, Keyboard. */
import { ArrowDownToLine, X } from "lucide-react"
import { useEffect, useState } from "react"

import type { Prefs, UpdateCheck } from "../bridge"
import { LIGHT_THEME, setPref, usePrefs } from "../prefs"
import { checkUpdateNow, currentBridge, installUpdate, pickFolder, settingsFolder, useStore } from "../store"
import { btn, Kbd, Spinner } from "../ui"
import { Choice, Group, Loaded, message, Row, Section, Segmented, tilde, Toggle, useLoad, Value } from "./controls"

// ---------------------------------------------------------------------------
// General
// ---------------------------------------------------------------------------

export function GeneralSection() {
  const general = usePrefs((p) => p.general)
  return (
    <Section title="General">
      <Group label="New sessions">
        <Row
          label="Default folder"
          hint={general.defaultFolder ? undefined : "Each new session asks for a folder"}
        >
          {general.defaultFolder && (
            <Value mono title={general.defaultFolder}>
              {tilde(general.defaultFolder)}
            </Value>
          )}
          <button
            onClick={() => void pickFolder().then((d) => void (d && setPref("general", { defaultFolder: d })))}
            className={btn("secondary", "sm")}
          >
            {general.defaultFolder ? "Change…" : "Choose…"}
          </button>
          {general.defaultFolder && (
            <button
              onClick={() => void setPref("general", { defaultFolder: null })}
              aria-label="Clear the default folder"
              title="Ask each time"
              className={btn("ghost", "sm", "text-muted", true)}
            >
              <X className="size-3.5" />
            </button>
          )}
        </Row>
        <Row label="Start in" id="start-mode">
          <Segmented
            label="New sessions start in"
            value={general.startMode}
            onChange={(startMode) => void setPref("general", { startMode })}
            options={[
              { id: "ask", label: "Ask" },
              { id: "edits", label: "Edits" },
              { id: "auto", label: "Auto" },
              { id: "last", label: "Last used" },
            ]}
          />
        </Row>
      </Group>
      <DefaultModel />
      <Updates />
    </Section>
  )
}

function DefaultModel() {
  const config = useLoad(async () => currentBridge().configGet(await settingsFolder()), [])
  const [error, setError] = useState<string | null>(null)
  return (
    <Loaded load={config} rows={1}>
      {(c) => (
        <Group
          label="Model"
          note={
            error ? (
              <span className="text-danger">{error}</span>
            ) : (
              <>
                Kept in <span className="font-mono">{tilde(c.file)}</span>, which the terminal app reads too.
              </>
            )
          }
        >
          <Row label="Default model">
            <Choice
              label="Default model"
              value={c.model}
              onChange={(model) => {
                setError(null)
                config.set({ ...c, model })
                void settingsFolder()
                  .then((d) => currentBridge().configSet(d, { model }))
                  .then(config.set, (e) => {
                    config.set(c)
                    setError(message(e))
                  })
              }}
              options={[
                { id: null, label: "Account default", note: "What your plan picks" },
                ...c.models.map((m) => ({ id: m.id, label: m.name, note: m.note })),
              ]}
            />
          </Row>
        </Group>
      )}
    </Loaded>
  )
}

function Updates() {
  const check = usePrefs((p) => p.general.checkUpdates)
  const about = useLoad(() => currentBridge().about(), [])
  return (
    <Group label="Updates">
      <Row label="Check for updates on start" id="check-updates">
        <Toggle
          labelledBy="check-updates"
          checked={check}
          onChange={(checkUpdates) => void setPref("general", { checkUpdates })}
        />
      </Row>
      <VersionRow version={about.data?.version ?? (about.error ? "Unknown" : "…")} />
    </Group>
  )
}

/** The version, with "Check now" (or the update it found) and the answer under it. */
export function VersionRow({ version }: { version: string }) {
  const update = useStore((s) => s.update)
  const [result, setResult] = useState<UpdateCheck | string | null>(null)
  const [checking, setChecking] = useState(false)
  const available = update?.available ? update : null
  return (
    <>
      <Row
        label="Version"
        hint={
          checking ? undefined : typeof result === "string" ? (
            <span className="text-danger">{result}</span>
          ) : available ? (
            `${available.version} is available`
          ) : result ? (
            "This is the latest version"
          ) : undefined
        }
      >
        <Value>{version}</Value>
        {available ? (
          <button
            onClick={() => void installUpdate()}
            disabled={available.installing}
            className={btn("primary", "sm", "gap-1.5")}
          >
            {available.installing ? <Spinner className="size-3.5 text-accent-foreground" /> : <ArrowDownToLine className="size-3.5" />}
            {available.installing ? "Installing…" : `Update to ${available.version}`}
          </button>
        ) : (
          <button
            disabled={checking}
            onClick={() => {
              setChecking(true)
              void checkUpdateNow()
                .then(setResult)
                .finally(() => setChecking(false))
            }}
            className={btn("secondary", "sm", "gap-1.5")}
          >
            {checking && <Spinner className="size-3.5" />}
            Check now
          </button>
        )}
      </Row>
      {available?.error && (
        <p className="text-dense px-4 py-2.5 text-danger select-text">{available.error}</p>
      )}
    </>
  )
}

// ---------------------------------------------------------------------------
// Appearance
// ---------------------------------------------------------------------------

export function AppearanceSection() {
  const a = usePrefs((p) => p.appearance)
  const set = (patch: Partial<Prefs["appearance"]>) => void setPref("appearance", patch)
  const systemCalm = useMedia("(prefers-reduced-motion: reduce)")
  return (
    <Section title="Appearance">
      <Group>
        {LIGHT_THEME && (
          <Row label="Theme">
            <Segmented
              label="Theme"
              value={a.theme}
              onChange={(theme) => set({ theme })}
              options={[
                { id: "dark", label: "Dark" },
                { id: "light", label: "Light" },
                { id: "system", label: "System", title: "Follows your system’s light or dark setting" },
              ]}
            />
          </Row>
        )}
        <Row label="Text size">
          <Segmented
            label="Text size"
            value={a.textSize}
            onChange={(textSize) => set({ textSize })}
            options={[
              { id: "small", label: "Small" },
              { id: "default", label: "Default" },
              { id: "large", label: "Large" },
            ]}
          />
        </Row>
        <Row label="Conversation width">
          <Segmented
            label="Conversation width"
            value={a.width}
            onChange={(width) => set({ width })}
            options={[
              { id: "narrow", label: "Narrow" },
              { id: "default", label: "Default" },
              { id: "wide", label: "Wide" },
            ]}
          />
        </Row>
      </Group>
      <Group>
        <Row label="Wrap long lines in code" hint="Diffs and code blocks; off, they scroll sideways" id="wrap-code">
          <Toggle labelledBy="wrap-code" checked={a.wrapCode} onChange={(wrapCode) => set({ wrapCode })} />
        </Row>
        <Row
          label="Reduce motion"
          hint={systemCalm ? "Your system already asks for less motion, so it’s reduced either way" : undefined}
          id="reduce-motion"
        >
          <Toggle labelledBy="reduce-motion" checked={a.reduceMotion} onChange={(reduceMotion) => set({ reduceMotion })} />
        </Row>
      </Group>
    </Section>
  )
}

function useMedia(query: string) {
  const [on, setOn] = useState(() => typeof matchMedia === "function" && matchMedia(query).matches)
  useEffect(() => {
    const m = matchMedia(query)
    const change = () => setOn(m.matches)
    m.addEventListener("change", change)
    return () => m.removeEventListener("change", change)
  }, [query])
  return on
}

// ---------------------------------------------------------------------------
// Conversation
// ---------------------------------------------------------------------------

export function ConversationSection() {
  const c = usePrefs((p) => p.conversation)
  const set = (patch: Partial<Prefs["conversation"]>) => void setPref("conversation", patch)
  return (
    <Section title="Conversation">
      <Group label="Shown at first" note="Each opens on click and stays as you leave it.">
        <Row label="File changes">
          <Segmented
            label="File changes"
            value={c.changes}
            onChange={(changes) => set({ changes })}
            options={[
              { id: "closed", label: "Closed" },
              { id: "open", label: "Open" },
            ]}
          />
        </Row>
        <Row label="Command output">
          <Segmented
            label="Command output"
            value={c.commands}
            onChange={(commands) => set({ commands })}
            options={[
              { id: "closed", label: "Closed" },
              { id: "open", label: "Open" },
            ]}
          />
        </Row>
        <Row label="Reads and searches">
          <Segmented
            label="Reads and searches"
            value={c.lookups}
            onChange={(lookups) => set({ lookups })}
            options={[
              { id: "grouped", label: "One line", title: "One line for a run of them, like “Read 3 files”" },
              { id: "listed", label: "A row each" },
            ]}
          />
        </Row>
        <Row
          label="Shorten long messages and code"
          hint="Cuts messages over 10 lines and code over 24 lines, with Show all"
          id="shorten"
        >
          <Toggle labelledBy="shorten" checked={c.shorten} onChange={(shorten) => set({ shorten })} />
        </Row>
      </Group>
      <Group label="Sending">
        <Row label="Send with" hint="Shift Enter always makes a new line">
          <Segmented
            label="Send with"
            value={c.send}
            onChange={(send) => set({ send })}
            options={[
              { id: "enter", label: "Enter" },
              { id: "ctrl-enter", label: "Ctrl Enter" },
            ]}
          />
        </Row>
      </Group>
    </Section>
  )
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

export function NotificationsSection() {
  const n = usePrefs((p) => p.notifications)
  const set = (patch: Partial<Prefs["notifications"]>) => void setPref("notifications", patch)
  const [test, setTest] = useState<"sending" | "sent" | string | null>(null)
  const rows: { key: keyof Omit<Prefs["notifications"], "enabled">; label: string }[] = [
    { key: "approval", label: "Needs your approval" },
    { key: "question", label: "Asks you a question" },
    { key: "finished", label: "Finishes" },
    { key: "failed", label: "Stops with an error" },
  ]
  return (
    <Section title="Notifications">
      <Group note="Notifications show only while the window is in the background.">
        <Row label="Notifications" id="notify-all">
          <Toggle labelledBy="notify-all" checked={n.enabled} onChange={(enabled) => set({ enabled })} />
        </Row>
      </Group>
      <Group label="When a session">
        {rows.map((r) => (
          <Row key={r.key} label={r.label} id={`notify-${r.key}`} className={n.enabled ? undefined : "opacity-60"}>
            <Toggle
              labelledBy={`notify-${r.key}`}
              checked={n[r.key]}
              disabled={!n.enabled}
              onChange={(on) => set({ [r.key]: on })}
            />
          </Row>
        ))}
      </Group>
      <Group>
        <Row
          label="Send a test"
          hint={
            test === "sent" ? (
              "Sent. If nothing showed, check the system’s notification settings for Anysola Code."
            ) : test && test !== "sending" ? (
              <span className="text-danger">{test}</span>
            ) : undefined
          }
        >
          <button
            disabled={test === "sending"}
            onClick={() => {
              setTest("sending")
              currentBridge()
                .notifyTest()
                .then(
                  () => setTest("sent"),
                  (e) => setTest(message(e))
                )
            }}
            className={btn("secondary", "sm", "gap-1.5")}
          >
            {test === "sending" && <Spinner className="size-3.5" />}
            Send
          </button>
        </Row>
      </Group>
    </Section>
  )
}

// ---------------------------------------------------------------------------
// Keyboard
// ---------------------------------------------------------------------------

export function KeyboardSection() {
  const send = usePrefs((p) => p.conversation.send)
  const sendKey = send === "enter" ? "Enter" : "Ctrl Enter"
  const groups: { label: string; rows: [string, string[]][] }[] = [
    {
      label: "Anywhere",
      rows: [
        ["New session", ["Ctrl N"]],
        ["Search sessions", ["Ctrl K"]],
        ["Settings", ["Ctrl ,"]],
        ["Close a panel, menu or Settings", ["Esc"]],
      ],
    },
    {
      label: "Message box",
      rows: [
        ["Send", [sendKey]],
        ["New line", send === "enter" ? ["Shift Enter"] : ["Enter", "Shift Enter"]],
        ["Allow the waiting request, with the box empty", [sendKey]],
        ["Deny the waiting request, or stop the turn", ["Esc"]],
        ["Commands and skills", ["/"]],
        ["Files and skills", ["@"]],
        ["Move in a menu", ["↑", "↓"]],
        ["Take the highlighted item", ["Enter", "Tab"]],
      ],
    },
    {
      label: "Sessions",
      rows: [
        ["Select one more", ["Ctrl click"]],
        ["Select a range", ["Shift click"]],
        ["Stop selecting", ["Esc"]],
        ["Save a new name", ["Enter"]],
      ],
    },
    {
      label: "Changes",
      rows: [
        ["Add a comment on a line", ["Enter"]],
        ["Cancel a comment", ["Esc"]],
      ],
    },
  ]
  return (
    <Section title="Keyboard">
      {groups.map((g) => (
        <Group key={g.label} label={g.label}>
          {g.rows.map(([label, keys]) => (
            <Row key={label} label={label} className="min-h-10 py-2">
              {keys.map((k) => (
                <Kbd key={k}>{k}</Kbd>
              ))}
            </Row>
          ))}
        </Group>
      ))}
    </Section>
  )
}

