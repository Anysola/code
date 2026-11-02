import { Plugin } from "@opencode/plugin/tui"
import { AnysolaRpc } from "@opencode/schema/anysola"
import { createResource, createSignal, For, Show } from "solid-js"
import { Account, BILLING_URL, statusText, type Me } from "../../anysola/store"
import { formatCredits, formatDate, usageLines } from "../../anysola/format"
import { Keymap } from "../../context/keymap"
import { useTheme } from "../../context/theme"
import { DialogConfirm } from "../../ui/dialog-confirm"
import { DialogHeader, DialogLayout } from "../../ui/dialog"
import { DialogSelect } from "../../ui/dialog-select"

// Anysola account features in the TUI: the credits status line, low-balance and event banners, and the /credits,
// /usage and /devices dialogs. All calls go through the server's `anysola.account` RPC, which holds the token.

type Rpc = ReturnType<Plugin.Context["client"]["rpc"]>
type Reply = { ok: boolean; status: number; body: unknown }

const record = (value: unknown) => (value && typeof value === "object" ? (value as Record<string, unknown>) : {})
const str = (value: unknown) => (typeof value === "string" ? value : "")

function errorText(reply: Reply) {
  const error = record(record(reply.body).error)
  const message = str(error.message) || str(record(reply.body).message)
  if (reply.status === 404) return "This server does not offer it yet."
  return message || (reply.status ? `Request failed (HTTP ${reply.status})` : "Cannot reach Anysola")
}

export default Plugin.define({
  id: "anysola.account",
  setup(context) {
    const rpc = context.client.rpc(AnysolaRpc.Definition) as unknown as {
      session: (input: { sessionID: string }) => Promise<AnysolaRpc.SessionCredits | null>
      me: (input: Record<string, never>) => Promise<Reply>
      usage: (input: { days: number }) => Promise<Reply>
      tokens: (input: Record<string, never>) => Promise<Reply>
      revoke: (input: { id: string }) => Promise<Reply>
      estimates: (input: Record<string, never>) => Promise<Record<string, number>>
      events: Rpc["events"]
    }

    const refreshMe = () =>
      rpc.me({}).then(
        (reply) => {
          if (reply.ok) Account.setMe(reply.body as Me)
        },
        () => undefined,
      )
    const refreshEstimates = () => rpc.estimates({}).then((value) => Account.setEstimates(value ?? {}), () => undefined)
    let pending: ReturnType<typeof setTimeout> | undefined
    const refreshSoon = () => {
      if (pending) clearTimeout(pending)
      pending = setTimeout(() => void refreshMe(), 1500)
    }
    void refreshMe()
    void refreshEstimates()
    rpc.events.on("credits", (event: { data: AnysolaRpc.SessionCredits }) => {
      Account.setSession(event.data)
      refreshSoon()
    })
    context.data.on("server.connected", () => {
      void refreshMe()
      void refreshEstimates()
    })
    setInterval(() => void refreshMe(), 5 * 60_000).unref?.()

    const showCredits = () => context.ui.dialog.show(() => <CreditsDialog />)
    const showUsage = () => context.ui.dialog.show(() => <UsageDialog />)
    const showDevices = () => context.ui.dialog.show(() => <DevicesDialog />)

    // Status line beside the session status: balance, this turn, this session. Hover shows the renewal date.
    context.ui.slot({
      append: "prompt.footer.status",
      render: (props) => {
        const theme = useTheme()
        const [hovered, setHovered] = createSignal(false)
        const text = () => {
          const base = statusText(props.sessionID)
          if (!base || !hovered()) return base
          const renews = formatDate(Account.me()?.credits?.renewsAt ?? Account.me()?.freeQuota?.resetsAt)
          return renews ? `${base} · renews ${renews}` : base
        }
        return (
          <Show when={text()}>
            <box
              flexShrink={0}
              onMouseOver={() => setHovered(true)}
              onMouseOut={() => setHovered(false)}
              onMouseUp={showCredits}
            >
              <text
                wrapMode="none"
                fg={Account.low(props.sessionID) ? theme.text.feedback.warning.base : theme.text.muted}
              >
                {text()}
              </text>
            </box>
          </Show>
        )
      },
    })

    const Banner = (props: { sessionID?: string }) => {
      const theme = useTheme()
      const event = () => Account.me()?.event
      return (
        <>
          <Show when={event()?.label}>
            <box flexDirection="row" flexShrink={0}>
              <text fg={theme.hue.interactive[200]} wrapMode="none">
                ◆ {event()!.label}
                {event()?.until ? ` · until ${formatDate(event()!.until)}` : ""}
              </text>
            </box>
          </Show>
          <Show when={Account.low(props.sessionID)}>
            <box flexDirection="row" flexShrink={0}>
              <text fg={theme.text.feedback.warning.base} wrapMode="none">
                Low balance: {formatCredits(Account.remaining(props.sessionID) ?? 0)} credits left · /credits
              </text>
            </box>
          </Show>
        </>
      )
    }
    context.ui.slot({ append: "session.composer.top", render: (props) => <Banner sessionID={props.sessionID} /> })
    context.ui.slot({ append: "home.footer.status", render: () => <Banner /> })

    context.ui.slot({
      append: "app",
      render() {
        context.keymap.layer(() => ({
          mode: "global",
          commands: [
            {
              id: "anysola.credits",
              title: "Credits and plan",
              group: "Anysola",
              slash: { name: "credits" },
              palette: true,
              run: showCredits,
            },
            {
              id: "anysola.usage",
              title: "Usage",
              group: "Anysola",
              slash: { name: "usage" },
              palette: true,
              run: showUsage,
            },
            {
              id: "anysola.devices",
              title: "Signed-in devices",
              group: "Anysola",
              slash: { name: "devices" },
              palette: true,
              run: showDevices,
            },
          ],
        }))
        return null
      },
    })

    function CreditsDialog() {
      const theme = useTheme().surface("dialog")
      void refreshMe()
      const rows = () => {
        const me = Account.me()
        if (!me) return [["Account", "Loading…"]]
        const session = context.ui.router.current()
        const sessionID = session.type === "session" ? session.sessionID : undefined
        const spent = sessionID ? Account.sessions[sessionID] : undefined
        return [
          ["Plan", `${me.plan ?? "unknown"}${me.access === "free" ? " (free)" : ""}`],
          ...(me.credits ? [["Balance", `${formatCredits(Account.remaining(sessionID) ?? me.credits.available ?? 0)} credits`]] : []),
          ...(me.credits?.renewsAt ? [["Renews", formatDate(me.credits.renewsAt)!]] : []),
          ...(me.freeQuota
            ? [["Free quota", `${me.freeQuota.usedTurns ?? 0}/${me.freeQuota.dailyTurns ?? 0} turns today`]]
            : []),
          ...(me.freeQuota?.resetsAt ? [["Resets", formatDate(me.freeQuota.resetsAt)!]] : []),
          ...(me.usageToday
            ? [["Today", `${me.usageToday.requests ?? 0} requests · ${formatCredits(me.usageToday.credits ?? 0)} credits`]]
            : []),
          ...(spent ? [["This session", `${spent.turns} turns · ${formatCredits(spent.total)} credits`]] : []),
          ...(me.event?.label
            ? [["Event", `${me.event.label}${me.event.until ? ` until ${formatDate(me.event.until)}` : ""}`]]
            : []),
        ]
      }
      return (
        <DialogLayout header={<DialogHeader title="Credits" />}>
          <box>
            <For each={rows()}>
              {(row) => (
                <box flexDirection="row" gap={2}>
                  <text fg={theme.text.muted} width={14} wrapMode="none">
                    {row[0]}
                  </text>
                  <text fg={theme.text.base}>{row[1]}</text>
                </box>
              )}
            </For>
            <box paddingTop={1}>
              <text fg={Account.low() ? theme.text.feedback.warning.base : theme.text.muted}>
                Upgrade or top up: {BILLING_URL}
              </text>
            </box>
          </box>
        </DialogLayout>
      )
    }

    function UsageDialog() {
      const theme = useTheme().surface("dialog")
      const [days, setDays] = createSignal(7)
      const [reply] = createResource(days, (value) => rpc.usage({ days: value }))
      Keymap.createLayer(() => ({
        mode: "modal",
        commands: [
          {
            bind: "t",
            title: "Toggle 7/30 days",
            group: "Dialog",
            run: () => {
              setDays((value) => (value === 7 ? 30 : 7))
            },
          },
        ],
      }))
      const lines = () => {
        const value = reply()
        if (!value) return ["Loading…"]
        if (!value.ok) return [errorText(value)]
        return usageLines(value.body)
      }
      return (
        <DialogLayout
          header={<DialogHeader title={`Usage · last ${days()} days`} />}
          footer={{ actions: { primary: [{ title: days() === 7 ? "30 days" : "7 days", key: "t", onTrigger: () => void setDays((value) => (value === 7 ? 30 : 7)) }] } }}
        >
          <box>
            <For each={lines()}>{(line) => <text fg={line.startsWith("  ") ? theme.text.base : theme.text.muted}>{line}</text>}</For>
          </box>
        </DialogLayout>
      )
    }

    function DevicesDialog() {
      const [reply, { refetch }] = createResource(() => rpc.tokens({}))
      const tokens = () => {
        const value = reply()
        if (!value?.ok) return []
        const list = record(value.body).tokens
        return Array.isArray(list) ? list.map(record) : []
      }
      const options = () =>
        tokens().map((token) => ({
          title: `${str(token.name) || "Unnamed device"}${token.current === true ? " (this device)" : ""}`,
          value: str(token.id),
          description: str(token.client),
          footer: token.lastUsedAt ? `used ${formatDate(str(token.lastUsedAt))}` : "never used",
        }))
      return (
        <Show
          when={reply() && reply()!.ok}
          fallback={
            <DialogLayout header={<DialogHeader title="Devices" />}>
              <text>{reply() ? errorText(reply()!) : "Loading…"}</text>
            </DialogLayout>
          }
        >
          <DialogSelect
            title="Devices"
            options={options()}
            onSelect={(option) => {
              const token = tokens().find((item) => item.id === option.value)
              if (token?.current === true) {
                context.ui.toast.show({ message: "This device: sign out with `anysola logout`.", variant: "info" })
                return
              }
              context.ui.dialog.show(() => (
                <DialogConfirm
                  title="Revoke device"
                  message={`Sign out "${option.title}"? It will need to log in again.`}
                  label={{ confirm: "Revoke" }}
                  onConfirm={() =>
                    void rpc.revoke({ id: option.value }).then((result) => {
                      context.ui.toast.show({
                        message: result.ok ? "Device signed out" : errorText(result),
                        variant: result.ok ? "success" : "error",
                      })
                      context.ui.dialog.show(() => <DevicesDialog />)
                      void refetch()
                    })
                  }
                />
              ))
            }}
          />
        </Show>
      )
    }
  },
})
