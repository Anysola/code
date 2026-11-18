/** The account's sections: Account (plan, devices), Usage (limits, resets, requests), Billing. */
import { useEffect, useState } from "react"

import type { Account, Usage, Window as AccountWindow } from "../bridge"
import { capitalize } from "../composer"
import { currentBridge, loadAccount, requireSignIn, setAccount, signOut, useStore } from "../store"
import { ago, btn, cn, resetsIn } from "../ui"
import { Confirm, day, Group, LinkRow, Loaded, message, Row, Section, Segmented, useLoad, Value } from "./controls"

type Live = Extract<Account, { expired: false }>

/** The account as last read, read again when a section opens; an expired one asks to sign in. */
function useAccount(): Live | null | "expired" {
  const account = useStore((s) => s.account)
  useEffect(() => {
    void loadAccount()
  }, [])
  if (!account) return null
  return account.expired ? "expired" : account
}

function Expired() {
  return (
    <Group>
      <Row label="Your sign-in expired" hint="Sign in again to see your plan, devices and usage">
        <button onClick={() => requireSignIn()} className={btn("primary", "sm")}>
          Sign in
        </button>
      </Row>
    </Group>
  )
}

const plan = (a: Live) => (a.plan ? capitalize(a.plan) : "No plan")

// ---------------------------------------------------------------------------
// Account
// ---------------------------------------------------------------------------

export function AccountSection() {
  const account = useAccount()
  return (
    <Section title="Account">
      {account === "expired" ? (
        <Expired />
      ) : (
        <>
          <Group>
            <Row label="Plan">
              <Value>{account ? plan(account) : "…"}</Value>
            </Row>
            <Row label="Anysola Code">
              {account ? (
                account.included ? (
                  <Value>Included</Value>
                ) : (
                  <span className="text-dense text-warning">Not in this plan</span>
                )
              ) : (
                <Value>…</Value>
              )}
            </Row>
          </Group>
          <Devices />
          <Group label="On account.anysola.com">
            <LinkRow label="Manage account" url="https://account.anysola.com/" />
            <LinkRow label="Security" url="https://account.anysola.com/security" />
            <LinkRow label="Connected apps" url="https://account.anysola.com/apps" />
          </Group>
          <Group>
            <Row label="Sign out" hint="Your sessions stay on this computer">
              <button onClick={() => void signOut()} className={btn("secondary", "sm")}>
                Sign out
              </button>
            </Row>
          </Group>
        </>
      )}
    </Section>
  )
}

function Devices() {
  const devices = useLoad(() => currentBridge().devices(), [])
  return (
    <Loaded load={devices} rows={3}>
      {(list) => (
        <Group label="Signed in" aside={`${list.length} device${list.length === 1 ? "" : "s"} and tokens`}>
          {list.length === 0 && <Row label="Nothing is signed in" />}
          {[...list]
            .sort((a, b) => Number(b.current) - Number(a.current))
            .map((d) => {
              const used = d.lastUsedAt ? `Used ${ago(Date.parse(d.lastUsedAt))}` : "Never used"
              const expires = d.expiresAt ? ` · Expires ${day(Date.parse(d.expiresAt))}` : ""
              return (
                <Row
                  key={d.id}
                  label={
                    <span className="flex min-w-0 items-center gap-2">
                      <span className="truncate">{d.name}</span>
                      {d.kind === "ci" && <span className="text-2xs shrink-0 rounded-md bg-default px-1.5 py-0.5 text-muted">CI token</span>}
                    </span>
                  }
                  hint={d.current ? d.client : `${d.client} · ${used}${expires}`}
                >
                  {d.current ? (
                    <Value>This device</Value>
                  ) : (
                    <Confirm
                      label="Revoke"
                      question={d.kind === "ci" ? "Revoke this token?" : `Sign out ${d.name}?`}
                      action="Revoke"
                      run={async () => {
                        try {
                          devices.set(await currentBridge().deviceRevoke(d.id))
                        } catch (e) {
                          return message(e)
                        }
                      }}
                    />
                  )}
                </Row>
              )
            })}
        </Group>
      )}
    </Loaded>
  )
}

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

export function UsageSection() {
  const account = useAccount()
  return (
    <Section title="Usage">
      {account === "expired" ? (
        <Expired />
      ) : (
        <>
          <Limits account={account} />
          {account?.event && <Promotion event={account.event} />}
          {account && <Bank account={account} />}
          <Requests />
        </>
      )}
    </Section>
  )
}

function Limits({ account }: { account: Live | null }) {
  return (
    <Group
      label="Limits"
      note="One limit shared by Chat and Code: a 5-hour window and a weekly window."
    >
      <Meter label="5-hour window" window={account?.fiveHour ?? null} blocking={account?.blockedBy === "five_hour"} />
      <Meter label="Weekly window" window={account?.weekly ?? null} blocking={account?.blockedBy === "weekly"} />
    </Group>
  )
}

function Meter({ label, window: w, blocking }: { label: string; window: AccountWindow | null; blocking: boolean }) {
  const resets = w ? resetsIn(w.resetsAt) : null
  return (
    <Row
      label={label}
      hint={
        !w ? (
          "Not reported for this plan"
        ) : blocking ? (
          <span className="text-warning">Full. Requests wait until it starts over{resets ? ` ${resets}` : ""}.</span>
        ) : resets ? (
          `Starts over ${resets}`
        ) : undefined
      }
    >
      {w && (
        <span className="flex items-center gap-3">
          <span
            className="block h-1 w-32 overflow-hidden rounded-full bg-default"
            role="meter"
            aria-label={label}
            aria-valuenow={w.percent}
            aria-valuemin={0}
            aria-valuemax={100}
          >
            <span
              className={cn("block h-full rounded-full", w.percent >= 90 ? "bg-warning" : "bg-foreground/70")}
              style={{ width: `${Math.min(100, w.percent)}%` }}
            />
          </span>
          <span className="text-dense w-10 text-right tabular-nums">{w.percent}%</span>
        </span>
      )}
    </Row>
  )
}

function Promotion({ event }: { event: NonNullable<Live["event"]> }) {
  const until = event.until ? ` until ${day(Date.parse(event.until))}` : ""
  return (
    <Group label="Running now">
      <Row
        label={event.label}
        hint={event.mode === "unlimited" ? `Use isn’t counted${until}` : `Use counts at a lower rate${until}`}
      />
    </Group>
  )
}

function Bank({ account }: { account: Live }) {
  // Ones that never expire go last, as the gateway spends them last.
  const at = (r: (typeof account.bank)[number]) => (r.expiresAt ? Date.parse(r.expiresAt) : Infinity)
  const bank = [...account.bank].sort((a, b) => at(a) - at(b))
  const soonest = bank[0]
  return (
    <Group label="Saved resets" aside={bank.length > 0 ? `${bank.length} saved` : undefined}>
      {bank.length === 0 && <Row label="No saved resets" hint="A reset starts a window over when you use it" />}
      {bank.map((r) => (
        <Row
          key={r.id}
          label={r.scope === "both" ? "Starts both windows over" : "Starts the 5-hour window over"}
          hint={r.expiresAt ? `Expires ${day(Date.parse(r.expiresAt))}` : "Doesn’t expire"}
        />
      ))}
      {soonest && (
        <Row label="Use a reset" hint="Takes the one that expires first">
          <Confirm
            label="Use a reset"
            variant="secondary"
            danger={false}
            question={soonest.scope === "both" ? "Start both windows over now?" : "Start the 5-hour window over now?"}
            action="Use it"
            run={async () => {
              try {
                setAccount(await currentBridge().limitsRedeem())
              } catch (e) {
                return message(e)
              }
            }}
          />
        </Row>
      )}
    </Group>
  )
}

const RANGES = [
  { id: "7", label: "7 days" },
  { id: "30", label: "30 days" },
  { id: "90", label: "90 days" },
] as const

function Requests() {
  const [days, setDays] = useState<"7" | "30" | "90">("30")
  const usage = useLoad(() => currentBridge().usage(Number(days)), [days])
  return (
    <section className="flex flex-col gap-2">
      <div className="flex min-h-8 items-end gap-3 px-1">
        <h2 className="text-2xs font-medium tracking-wide text-muted">Requests</h2>
        <div className="ml-auto">
          <Segmented label="Period" value={days} onChange={setDays} options={[...RANGES]} />
        </div>
      </div>
      {usage.data ? (
        <div className={cn("transition-opacity", usage.loading && "opacity-60")}>
          <UsageView usage={usage.data} />
        </div>
      ) : (
        <Loaded load={usage} rows={4} bare>
          {() => null}
        </Loaded>
      )}
      {usage.data && usage.error && <p className="text-dense px-1 text-danger">{usage.error}</p>}
    </section>
  )
}

function UsageView({ usage }: { usage: Usage }) {
  const max = Math.max(1, ...usage.byDay.map((d) => d.requests))
  const label = (date: string) =>
    new Date(`${date}T00:00:00Z`).toLocaleDateString("en-GB", { day: "numeric", month: "short", timeZone: "UTC" })
  const total = usage.byModel.reduce((n, m) => n + m.requests, 0) || 1
  return (
    <div className="flex flex-col divide-y divide-separator rounded-xl bg-surface">
      <dl className="grid grid-cols-3 gap-4 px-4 py-3.5">
        <Stat label={`Last ${usage.days} days`} value={usage.requests.toLocaleString("en-GB")} />
        <Stat label="Today" value={usage.today.toLocaleString("en-GB")} />
        <Stat
          label="Reused from earlier"
          title="How much of what the agent sends was reused from earlier requests instead of read again"
          value={usage.cacheHitRate === null ? "Not known" : `${Math.round(usage.cacheHitRate * 100)}%`}
        />
      </dl>
      {usage.requests === 0 ? (
        <p className="text-dense px-4 py-3.5 text-muted">No requests in these {usage.days} days.</p>
      ) : (
      <div className="px-4 pt-4 pb-3">
        <div className="flex h-28 items-end gap-px" aria-hidden>
          {usage.byDay.map((d, i) => {
            const today = i === usage.byDay.length - 1
            return (
              <div key={d.date} className="group relative flex h-full flex-1 items-end" title={`${label(d.date)}: ${d.requests} requests`}>
                <div
                  className={cn(
                    "w-full rounded-t-[2px] transition-colors",
                    today ? "bg-foreground/80" : "bg-foreground/25 group-hover:bg-foreground/50"
                  )}
                  style={{ height: d.requests ? `${Math.max(2, (d.requests / max) * 100)}%` : "1px" }}
                />
              </div>
            )
          })}
        </div>
        <div className="text-2xs mt-2 flex justify-between text-muted tabular-nums" aria-hidden>
          <span>{label(usage.byDay[0]?.date ?? "")}</span>
          <span>Today</span>
        </div>
        <table className="sr-only">
          <caption>Requests per day</caption>
          <tbody>
            {usage.byDay.map((d) => (
              <tr key={d.date}>
                <th>{label(d.date)}</th>
                <td>{d.requests}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      )}
      {usage.byModel.length > 0 && (
        <div className="flex flex-col py-1.5">
          {[...usage.byModel]
            .sort((a, b) => b.requests - a.requests)
            .map((m) => (
              <div key={m.model} className="text-dense flex h-9 items-center gap-3 px-4">
                <span className="min-w-0 flex-1 truncate">{m.name}</span>
                <span className="text-muted tabular-nums">{m.requests.toLocaleString("en-GB")}</span>
                <span className="w-10 text-right text-muted tabular-nums">{Math.round((m.requests / total) * 100)}%</span>
              </div>
            ))}
        </div>
      )}
    </div>
  )
}

function Stat({ label, value, title }: { label: string; value: string; title?: string }) {
  return (
    <div className="flex flex-col gap-0.5" title={title}>
      <dt className="text-2xs text-muted">{label}</dt>
      <dd className="text-md tabular-nums">{value}</dd>
    </div>
  )
}

// ---------------------------------------------------------------------------
// Billing
// ---------------------------------------------------------------------------

export function BillingSection() {
  const account = useAccount()
  return (
    <Section title="Billing">
      {account === "expired" ? (
        <Expired />
      ) : (
        <>
          <Group note="Payments happen on account.anysola.com. The app never asks for card details.">
            <Row label="Plan" hint="One limit shared by Chat and Code: a 5-hour window and a weekly window">
              <Value>{account ? plan(account) : "…"}</Value>
            </Row>
          </Group>
          <Group>
            <LinkRow label="Manage billing" url="https://account.anysola.com/billing" />
            <LinkRow label="See plans" url="https://www.anysola.com/pricing" />
          </Group>
        </>
      )}
    </Section>
  )
}
