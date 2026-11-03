import { Plugin } from "@opencode/plugin/tui"
import { AnysolaRpc } from "@opencode/schema/anysola"
import { createEffect, createResource, createSignal, For, onCleanup, Show } from "solid-js"
import { Account, BILLING_URL, statusText, type Me } from "../../anysola/store"
import { formatCredits, formatDate, formatShortDate, planName, usageLines } from "../../anysola/format"
import { Keymap } from "../../context/keymap"
import { useTheme } from "../../context/theme"
import { DialogConfirm } from "../../ui/dialog-confirm"
import { DialogHeader, DialogLayout } from "../../ui/dialog"
import { DialogSelect } from "../../ui/dialog-select"

// Anysola account features in the TUI: the credits status line, low-balance and event banners, the /credits,
// /usage and /devices dialogs, and /budget with the question asked before a turn once a Session is over budget. All calls go through the server's `anysola.account` RPC, which holds the token.

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

function detail(error: unknown): string {
  if (error instanceof Error) return error.message
  const value = record(error)
  const nested = record(value.error)
  const text = str(value.message) || str(nested.message) || str(value.reason) || str(value._tag) || str(nested._tag)
  if (text) return text
  try {
    return JSON.stringify(error).slice(0, 120)
  } catch {
    return String(error)
  }
}

export default Plugin.define({
  id: "anysola.account",
  setup(context) {
    const [meError, setMeError] = createSignal<string>()
    const raw = context.client.rpc(AnysolaRpc.Definition) as unknown as {
      session: (input: { sessionID: string }) => Promise<AnysolaRpc.SessionCredits | null>
      budget: (input: { sessionID: string; limit: number | null }) => Promise<AnysolaRpc.SessionCredits>
      budgetDefault: (input: { limit?: number | null }) => Promise<{ limit: number | null }>
      me: (input: Record<string, never>) => Promise<Reply>
      usage: (input: { days: number }) => Promise<Reply>
      tokens: (input: Record<string, never>) => Promise<Reply>
      revoke: (input: { id: string }) => Promise<Reply>
      estimates: (input: Record<string, never>) => Promise<Record<string, number>>
      events: Rpc["events"]
    }
    // Every call resolves: a failed RPC (for example an older background service that has no `anysola.account`)
    // becomes an inline message instead of an error that reaches the crash screen.
    const settle = (promise: () => Promise<unknown>): Promise<Reply> =>
      Promise.resolve()
        .then(promise)
        .then(
          (value) => {
            const reply = record(value)
            return typeof reply.ok === "boolean" && typeof reply.status === "number"
              ? (reply as Reply)
              : { ok: false, status: 0, body: { error: { message: "Unexpected reply from the Anysola Code server" } } }
          },
          (error: unknown) => ({
            ok: false,
            status: 0,
            body: {
              error: {
                message: `Account details are unavailable (${detail(error)}). If you just updated, run \`anysola service restart\`.`,
              },
            },
          }),
        )
    const rpc = {
      events: raw.events,
      me: () => settle(() => raw.me({})),
      usage: (days: number) => settle(() => raw.usage({ days })),
      tokens: () => settle(() => raw.tokens({})),
      revoke: (id: string) => settle(() => raw.revoke({ id })),
      estimates: () =>
        Promise.resolve()
          .then(() => raw.estimates({}))
          .then(
            (value) => (value && typeof value === "object" ? value : {}),
            () => ({}),
          ),
    }

    /** Latest credits of a Session from the server (it keeps them across restarts); undefined when unavailable. */
    const loadSession = (sessionID: string) =>
      Promise.resolve()
        .then(() => raw.session({ sessionID }))
        .then(
          (value) => {
            if (value && typeof value === "object" && typeof value.sessionID === "string") Account.setSession(value)
            return value ?? undefined
          },
          () => undefined,
        )
    const setBudget = (sessionID: string, limit: number | null) =>
      Promise.resolve()
        .then(() => raw.budget({ sessionID, limit }))
        .then((value) => {
          Account.setSession(value)
          return value
        })
    const budgetDefault = (limit?: number | null) =>
      Promise.resolve()
        .then(() => raw.budgetDefault(limit === undefined ? {} : { limit }))
        .then((value) => value.limit)

    // Asked before a turn starts once the Session has spent its budget; the turn in progress is never cut.
    const budgetGate = async (sessionID: string) => {
      const session = await loadSession(sessionID)
      if (!session || typeof session.budget !== "number" || session.total < session.budget) return true
      const budget = session.budget
      const raised = Math.max(budget, session.total) + budget
      const choice = await context.ui.dialog.select({
        title: `Budget reached · ${formatCredits(session.total)}/${formatCredits(budget)} credits`,
        options: [
          { title: "Continue once", value: "once", description: "Ask again before the next prompt" },
          { title: `Add ${formatCredits(budget)} credits`, value: "add", description: `Budget ${formatCredits(raised)}` },
          { title: "Remove the budget", value: "off" },
          { title: "Stop", value: "stop", description: "Keep the prompt in the composer" },
        ],
      })
      if (choice === "once") return true
      if (choice === "add" || choice === "off") {
        await setBudget(sessionID, choice === "add" ? raised : null)
        return true
      }
      return false
    }

    const budgetCommand = async (input?: string) => {
      const route = context.ui.router.current()
      const sessionID = route.type === "session" ? route.sessionID : undefined
      const words = (input ?? "").trim().toLowerCase().split(/\s+/).filter(Boolean)
      const toast = (message: string, variant: "info" | "success" | "warning" | "error" = "info") =>
        context.ui.toast.show({ message, variant })
      const parse = (word: string | undefined) => {
        if (word === "off") return null
        const value = Number(word)
        return Number.isInteger(value) && value >= 1 ? value : undefined
      }
      try {
        if (words[0] === "default") {
          if (words.length === 1) {
            const limit = await budgetDefault()
            return toast(limit === null ? "No default budget" : `New sessions start with a ${formatCredits(limit)} credit budget`)
          }
          const limit = parse(words[1])
          if (limit === undefined) return toast("Usage: /budget default <credits> | off", "warning")
          await budgetDefault(limit)
          return toast(
            limit === null ? "Default budget removed" : `New sessions start with a ${formatCredits(limit)} credit budget`,
            "success",
          )
        }
        if (!sessionID) return toast("Open a session first, or set one for new sessions: /budget default <credits>", "warning")
        if (words.length === 0) {
          const session = await loadSession(sessionID)
          const spent = formatCredits(session?.total ?? 0)
          return toast(
            typeof session?.budget === "number"
              ? `Session budget ${spent}/${formatCredits(session.budget)} credits`
              : `No budget · ${spent} credits spent. Set one: /budget <credits>`,
          )
        }
        const limit = parse(words[0])
        if (limit === undefined || words.length > 1) return toast("Usage: /budget <credits> | off | default <credits>", "warning")
        const session = await setBudget(sessionID, limit)
        toast(
          limit === null
            ? "Budget removed"
            : `Session budget ${formatCredits(session.total)}/${formatCredits(limit)} credits`,
          "success",
        )
      } catch (error) {
        toast(`Budget unavailable (${detail(error)}). If you just updated, run \`anysola service restart\`.`, "error")
      }
    }

    const refreshMe = () =>
      rpc.me().then((reply) => {
        if (reply.ok && reply.body && typeof reply.body === "object") {
          Account.setMe(reply.body as Me)
          setMeError(undefined)
        } else setMeError(errorText(reply))
      })
    const refreshEstimates = () => rpc.estimates().then((value) => Account.setEstimates(value as Record<string, number>))
    let pending: ReturnType<typeof setTimeout> | undefined
    const refreshSoon = () => {
      if (pending) clearTimeout(pending)
      pending = setTimeout(() => void refreshMe(), 1500)
    }
    // Subscriptions live as long as the app slot, so they close with the TUI instead of outliving its server.
    const subscribe = () => {
      Account.setBudgetGate(budgetGate)
      onCleanup(() => Account.setBudgetGate(undefined))
      void refreshMe()
      void refreshEstimates()
      const listen = () => {
        try {
          return rpc.events.on("credits", (event: { data: AnysolaRpc.SessionCredits }) => {
            if (event?.data && typeof event.data.sessionID === "string") Account.setSession(event.data)
            refreshSoon()
          })
        } catch {
          return undefined
        }
      }
      const unsubscribe = [
        listen(),
        context.data.on("server.connected", () => {
          void refreshMe()
          void refreshEstimates()
        }),
      ]
      const timer = setInterval(() => void refreshMe(), 5 * 60_000)
      onCleanup(() => {
        clearInterval(timer)
        if (pending) clearTimeout(pending)
        for (const stop of unsubscribe) if (typeof stop === "function") stop()
      })
    }

    const showCredits = () => context.ui.dialog.show(() => <CreditsDialog />)
    const showUsage = () => context.ui.dialog.show(() => <UsageDialog />)
    const showDevices = () => context.ui.dialog.show(() => <DevicesDialog />)

    // Status line beside the session status: balance, this turn, this session. Hover shows the renewal date.
    context.ui.slot({
      append: "prompt.footer.status",
      render: (props) => {
        const theme = useTheme()
        const [hovered, setHovered] = createSignal(false)
        // A reopened Session shows its stored total and budget before it spends anything in this process.
        createEffect(() => {
          if (props.sessionID) void loadSession(props.sessionID)
        })
        const text = () => {
          const base = statusText(props.sessionID)
          if (!base || !hovered()) return base
          const me = Account.me()
          const renews = me?.credits?.renewsAt ? formatShortDate(me.credits.renewsAt) : formatDate(me?.freeQuota?.resetsAt)
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
                fg={
                  Account.low(props.sessionID) || Account.overBudget(props.sessionID)
                    ? theme.text.feedback.warning.base
                    : theme.text.muted
                }
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
              <text fg={theme.text.feedback.info.base} wrapMode="none">
                {event()!.label}
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
    // Home footer: the plan and when it renews ("Team · renews 23 Nov"); the balance sits in the composer footer.
    const AccountLine = () => {
      const theme = useTheme()
      const text = () => {
        const me = Account.me()
        if (!me?.plan) return undefined
        const renews = formatShortDate(me.credits?.renewsAt)
        return renews ? `${planName(me.plan)} · renews ${renews}` : planName(me.plan)
      }
      return (
        <Show when={text()}>
          <box flexShrink={0} onMouseUp={showCredits}>
            <text fg={theme.text.faint} wrapMode="none">
              {text()}
            </text>
          </box>
        </Show>
      )
    }
    context.ui.slot({ append: "session.composer.top", render: (props) => <Banner sessionID={props.sessionID} /> })
    context.ui.slot({
      append: "home.footer.status",
      render: () => (
        <>
          <AccountLine />
          <Banner />
        </>
      ),
    })

    context.ui.slot({
      append: "app",
      render() {
        subscribe()
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
              id: "anysola.budget",
              title: "Session budget",
              group: "Anysola",
              slash: { name: "budget", arguments: true },
              palette: true,
              run: (input?: string) => void budgetCommand(input),
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
        if (!me) return [["Account", meError() ?? "Loading…"]]
        const session = context.ui.router.current()
        const sessionID = session.type === "session" ? session.sessionID : undefined
        const spent = sessionID ? Account.sessions[sessionID] : undefined
        return [
          ["Plan", `${planName(me.plan ?? "unknown")}${me.access === "free" ? " (free)" : ""}`],
          ...(me.credits ? [["Balance", `${formatCredits(Account.remaining(sessionID) ?? me.credits.available ?? 0)} credits`]] : []),
          ...(me.credits?.renewsAt ? [["Renews", formatShortDate(me.credits.renewsAt)!]] : []),
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
      const [reply] = createResource(days, (value) => rpc.usage(value))
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
      const [reply, { refetch }] = createResource(() => rpc.tokens())
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
                    void rpc.revoke(option.value).then((result) => {
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
