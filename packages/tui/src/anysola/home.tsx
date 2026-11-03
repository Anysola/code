import path from "path"
import { createResource, createSignal, For, Show } from "solid-js"
import { useTerminalDimensions } from "@opentui/solid"
import { Project } from "@opencode/schema/project"
import { withTimestampedFallback } from "@opencode/util/session-title-fallback"
import { useClient } from "../context/client"
import { useData } from "../context/data"
import { Keymap } from "../context/keymap"
import { useLocation } from "../context/location"
import { useRoute } from "../context/route"
import { useTheme } from "../context/theme"
import { Locale } from "../util/locale"
import { formatAgo } from "./format"

const LIMIT = 3

// Home: the latest sessions of this directory under the composer, to pick one up again with a click.
// Hidden when there are none, when they cannot be read, and on short terminals.
export function RecentSessions() {
  const client = useClient()
  const data = useData()
  const route = useRoute()
  const theme = useTheme()
  const location = useLocation()
  const keymap = Keymap.use()
  const shortcuts = Keymap.useShortcuts()
  const dimensions = useTerminalDimensions()
  const [hovered, setHovered] = createSignal<string>()

  // A failed read hides the list instead of reaching the crash screen.
  const [sessions] = createResource(
    () => location.ref ?? data.location.default(),
    (ref) => load(ref).catch(() => []),
  )
  async function load(ref: ReturnType<typeof data.location.default>) {
    if (!data.location.info(ref)) await data.location.sync(ref)
    const current = data.location.info(ref)
    if (!current) return []
    const response = await client.api.session.list({
      ...(current.project.id === Project.ID.global
        ? { directory: current.directory }
        : {
            project: current.project.id,
            subpath: path.relative(current.project.directory, current.directory).replaceAll("\\", "/"),
          }),
      limit: LIMIT,
      order: "desc",
      parentID: null,
    })
    return response.data
  }
  const list = () => sessions.latest ?? []
  const width = () => Math.min(75, dimensions().width - 4)

  return (
    <Show when={dimensions().height >= 24 && list().length > 0}>
      <box width="100%" maxWidth={75} paddingTop={1} flexShrink={0}>
        <box flexDirection="row" justifyContent="space-between">
          <text fg={theme.text.faint} wrapMode="none">
            Recent
          </text>
          <Show when={shortcuts.get("session.list")}>
            {(key) => (
              <text fg={theme.text.muted} wrapMode="none" onMouseUp={() => keymap.dispatch("session.list")}>
                {key()} <span style={{ fg: theme.text.faint }}>all sessions</span>
              </text>
            )}
          </Show>
        </box>
        <For each={list()}>
          {(session) => {
            const when = formatAgo(session.time.updated)
            return (
              <box
                flexDirection="row"
                justifyContent="space-between"
                gap={2}
                onMouseOver={() => setHovered(session.id)}
                onMouseOut={() => setHovered(undefined)}
                onMouseUp={() => route.navigate({ type: "session", sessionID: session.id })}
              >
                <text fg={hovered() === session.id ? theme.text.base : theme.text.muted} wrapMode="none" truncate>
                  {Locale.truncateWidth(withTimestampedFallback(session), Math.max(8, width() - when.length - 2))}
                </text>
                <text fg={theme.text.faint} wrapMode="none" flexShrink={0}>
                  {when}
                </text>
              </box>
            )
          }}
        </For>
      </box>
    </Show>
  )
}
