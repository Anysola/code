/**
 * The bridge outside Tauri: example sessions and a transcript, for working on the window in a
 * browser and for screenshots. `?state=` picks a moment: `signed-out`, `empty`, `approval`
 * (the default), `running`, `failed`, `question`, `update`, `rewound`, `plan`, `todos`, `done`.
 * Prompts in `refused` are refused before their answer comes back; in `stuck` they send no
 * events and Stop sends none either, as when nothing was running.
 */
import type { Account, Bridge, CoreEvent, FileDiff, Info, Item, Revert, SessionRow, Todo, View } from "./bridge"

const WEB = "/home/alex/code/anysola-web"
const CODE = "/home/alex/code/anysola-code"
const MAIL = "/home/alex/code/mailwick"
const now = Date.now()

const rows: SessionRow[] = [
  { id: "ses_pricing", title: "Yearly price on /pricing", directory: WEB, updated: now - 20_000 },
  { id: "ses_alerts", title: "Gateway health alerts", directory: CODE, updated: now - 60_000 },
  { id: "ses_search", title: "News search ranking", directory: WEB, updated: now - 2 * 3_600_000 },
  { id: "ses_footer", title: "Footer links on mobile", directory: WEB, updated: now - 26 * 3_600_000 },
  { id: "ses_load", title: "Load test for streams", directory: CODE, updated: now - 30 * 3_600_000 },
  { id: "ses_imap", title: "IMAP idle reconnect", directory: MAIL, updated: now - 3 * 86_400_000 },
]

const planCard: FileDiff = {
  file: "apps/web/src/components/plan-card.tsx",
  status: "modified",
  additions: 2,
  deletions: 2,
  patch: `Index: apps/web/src/components/plan-card.tsx
===================================================================
--- apps/web/src/components/plan-card.tsx
+++ apps/web/src/components/plan-card.tsx
@@ -12,8 +12,8 @@
 import type { Cycle, Plan } from "../lib/plans"

-export function PlanCard({ plan }: { plan: Plan }) {
-  const price = plan.prices[plan.defaultCycle]
+export function PlanCard({ plan, cycle }: { plan: Plan; cycle: Cycle }) {
+  const price = plan.prices[cycle]
   return (
     <Card className="plan-card">
`,
}

const pricing: FileDiff = {
  file: "apps/web/src/routes/pricing.tsx",
  status: "modified",
  additions: 1,
  deletions: 1,
  patch: `Index: apps/web/src/routes/pricing.tsx
===================================================================
--- apps/web/src/routes/pricing.tsx
+++ apps/web/src/routes/pricing.tsx
@@ -24,7 +24,7 @@ function PricingPage() {
       <CycleToggle value={cycle} onChange={setCycle} />
       {plans.map((plan) => (
-        <PlanCard key={plan.id} plan={plan} />
+        <PlanCard key={plan.id} plan={plan} cycle={cycle} />
       ))}
`,
}

const test: FileDiff = {
  file: "apps/web/test/pricing.test.tsx",
  status: "added",
  additions: 28,
  deletions: 0,
  patch: `Index: apps/web/test/pricing.test.tsx
===================================================================
--- apps/web/test/pricing.test.tsx
+++ apps/web/test/pricing.test.tsx
@@ -0,0 +1,6 @@
+import { render, screen } from "@testing-library/react"
+import userEvent from "@testing-library/user-event"
+
+it("shows the yearly price after switching", async () => {
+  render(<PricingPage />)
+  await userEvent.click(screen.getByRole("radio", { name: "Yearly" }))
`,
}

const tool = (
  id: string,
  name: string,
  target: string,
  extra: Partial<Extract<Item, { kind: "tool" }>> = {}
): Item => ({ kind: "tool", id, name, status: "completed", target, files: null, output: "", error: null, ...extra })

const pricingItems: Item[] = [
  {
    kind: "user",
    id: "m1",
    text: "On /pricing the Monthly/Yearly toggle switches, but the cards keep showing the monthly price. Fix it and add a test.",
    images: 0,
  },
  { kind: "text", id: "m2:0", text: "Let me find where the cards get their price." },
  tool("t1", "read", "apps/web/src/routes/pricing.tsx", { output: "64 lines" }),
  tool("t2", "grep", "defaultCycle", { output: "4 matches" }),
  tool("t3", "read", "apps/web/src/components/plan-card.tsx", { output: "41 lines" }),
  {
    kind: "text",
    id: "m3:0",
    text: "The toggle updates `cycle` on the page, but `PlanCard` never receives it: it reads `plan.defaultCycle`, which is always monthly. I’ll pass the selected cycle down and read the price from it.",
  },
  tool("t4", "edit", planCard.file, { files: [planCard] }),
  tool("t5", "edit", pricing.file, { files: [pricing] }),
  tool("t5b", "subagent", "Check the other plan cards for the same bug", { child: "ses_child" }),
  tool("t6", "write", test.file, { files: [test] }),
  {
    kind: "text",
    id: "m4:0",
    text: "The fix is in. The card now reads its price this way:\n\n```tsx\nconst price = plan.prices[cycle]\n```\n\nRunning the pricing tests next.",
  },
  tool("t7", "shell", "pnpm --filter @anysola/web test pricing", { status: "running" }),
]

const efforts = ["low", "medium", "high", "xhigh", "max"]
const agents = [
  { id: "build", name: "Build", description: "The default agent." },
  { id: "plan", name: "Plan", description: "Read-only agent for planning." },
]

const info: Info = {
  agent: "build",
  agents,
  model: "Opus 5.5",
  modelId: "claude-opus-5-5",
  variant: "high",
  fiveHour: 34,
  models: [
    { id: "claude-opus-5-5", name: "Opus 5.5", note: "Most capable", efforts, images: true },
    { id: "claude-sonnet-5-5", name: "Sonnet 5.5", note: "Fast and capable", efforts, images: true },
    { id: "claude-haiku-5-5", name: "Haiku 5.5", note: "Fastest", efforts: [], images: true },
  ],
}

const account: Account = {
  expired: false,
  plan: "pro",
  included: true,
  fiveHour: { percent: 34, resetsAt: new Date(now + 2.4 * 3_600_000).toISOString() },
  weekly: { percent: 12, resetsAt: new Date(now + 4 * 86_400_000).toISOString() },
}

const planItems: Item[] = [
  { kind: "user", id: "p1", text: "Plan how to add yearly prices to the team plan.", images: 0 },
  tool("p2", "read", "apps/web/src/lib/plans.ts", { output: "88 lines" }),
  tool("p3", "grep", "teamPrices", { output: "3 matches" }),
  {
    kind: "text",
    id: "p4:0",
    text: "Here’s the plan:\n\n1. Add `yearly` to `teamPrices` in `lib/plans.ts`, at 10× monthly like the other plans.\n2. Let `PlanCard` show the toggle for the team card.\n3. Extend `pricing.test.tsx` with a team-plan case.\n\nNothing else reads `teamPrices`, so the change stays in these three files.",
  },
]

const todos: Todo[] = [
  { content: "Pass the selected cycle to PlanCard", status: "completed" },
  { content: "Read the price from the cycle", status: "completed" },
  { content: "Add a pricing test", status: "in_progress" },
  { content: "Check the team and enterprise cards", status: "pending" },
]

const rewound: Revert = {
  turns: 1,
  files: [
    { file: planCard.file, status: "modified", additions: 2, deletions: 2 },
    { file: pricing.file, status: "modified", additions: 1, deletions: 1 },
    { file: test.file, status: "added", additions: 28, deletions: 0 },
  ],
}

export function mockBridge(): Bridge {
  const state = new URLSearchParams(location.search).get("state") ?? "approval"
  const handlers = new Set<(e: CoreEvent) => void>()
  const emit = (e: CoreEvent) => setTimeout(() => handlers.forEach((h) => h(e)), 0)
  const transcripts = new Map<string, Item[]>([
    ["ses_pricing", pricingItems],
    [
      "ses_child",
      [
        { kind: "user", id: "c1", text: "Check the other plan cards for the same bug.", images: 0 },
        tool("c2", "grep", "plan.defaultCycle", { output: "1 match" }),
        { kind: "text", id: "c3:0", text: "Only `PlanCard` read `defaultCycle`; the team and enterprise cards already take the cycle as a prop." },
      ],
    ],
  ])
  if (state === "plan") transcripts.set("ses_pricing", planItems)
  let signedIn = state !== "signed-out"
  let revert: Revert | null = state === "rewound" ? rewound : null
  const view = (sessionId: string): View => {
    const items = transcripts.get(sessionId) ?? []
    if (revert && sessionId === "ses_pricing")
      return { items: [], revert, todos: [], agent: null }
    const done = state === "done" || state === "plan" || state === "todos"
    return {
      items: state === "question" ? items.slice(0, -1) : done ? items.filter((i) => i.kind !== "tool" || i.status !== "running") : items,
      revert: null,
      todos: state === "todos" && sessionId === "ses_pricing" ? todos : [],
      agent: state === "plan" ? "plan" : "build",
    }
  }
  const wait = (ms: number) => new Promise((r) => setTimeout(r, ms))
  let cancelLogin = () => {}

  return {
    async authStatus() {
      return { signedIn }
    },
    async loginStart() {
      return { url: "https://account.anysola.com/device", code: "QWKD-7RTM" }
    },
    async loginWait() {
      await new Promise<void>((done) => {
        cancelLogin = done
        setTimeout(done, 1_000_000)
      })
      return { signedIn }
    },
    async loginCancel() {
      cancelLogin()
    },
    async sessions() {
      return state === "empty" ? [] : rows
    },
    async openSession(_directory, sessionId) {
      const session = rows.find((r) => r.id === sessionId) ?? rows[0]
      if (session.id === "ses_pricing") {
        if (state === "approval")
          emit({
            type: "permission",
            sessionId: session.id,
            directory: session.directory,
            requestId: "per_1",
            action: "shell",
            resources: ["pnpm --filter @anysola/web test pricing"],
            canAlways: true,
            files: null,
          })
        if (state === "running") {
          emit({ type: "started", sessionId: session.id })
          const words = "The tests pass. I’ll also check the yearly prices on the team plan, which reads".split(" ")
          void (async () => {
            for (const w of words) {
              emit({ type: "text", sessionId: session.id, messageId: "live", ordinal: 0, text: `${w} ` })
              await wait(15)
            }
          })()
        }
        if (state === "question")
          emit({
            type: "question",
            sessionId: session.id,
            directory: session.directory,
            requestId: "que_1",
            questions: [
              {
                header: "Team plan",
                question: "The team plan has no yearly price yet. What should the card show?",
                options: [
                  { label: "Hide the toggle", description: "Show monthly only on the team card" },
                  { label: "Use 10× monthly", description: "The same discount as the other plans" },
                ],
              },
            ],
          })
        if (state === "failed")
          emit({
            type: "failed",
            sessionId: session.id,
            error: "Anysola Code is temporarily unavailable. Try again shortly.",
          })
      }
      const busy = session.id === "ses_pricing" && ["approval", "running", "question"].includes(state)
      return {
        session,
        view: view(session.id),
        info: state === "plan" ? { ...info, agent: "plan" } : info,
        busy,
      }
    },
    async transcript(_directory, sessionId) {
      return view(sessionId)
    },
    async revert(_directory, sessionId, messageId) {
      const items = transcripts.get(sessionId) ?? []
      const text = items.find((i) => i.id === messageId)
      revert = rewound
      return { text: text?.kind === "user" ? text.text : "", view: view(sessionId) }
    },
    async unrevert(_directory, sessionId) {
      revert = null
      return view(sessionId)
    },
    async resend(_directory, sessionId) {
      revert = null
      emit({ type: "started", sessionId })
      return { sessionId }
    },
    async palette() {
      return {
        commands: [
          { name: "init", description: "create/update AGENTS.md" },
          { name: "review", description: "review changes [commit|branch|pr], defaults to uncommitted" },
          { name: "goal", description: "keep working until an objective is met [pause|resume|clear]" },
          { name: "release-notes", description: "Draft release notes from the commits since the last tag" },
        ],
        skills: [
          { name: "frontend-design", description: "Distinctive, intentional visual design for new UI" },
          { name: "pdf", description: "Read, fill and create PDF files" },
        ],
      }
    },
    async runCommand(_directory, { sessionId }) {
      const id = sessionId ?? "ses_pricing"
      emit({ type: "started", sessionId: id })
      return { sessionId: id }
    },
    async compact() {},
    async selectAgent(_directory, _sessionId, agent) {
      return { ...info, agent }
    },
    async projectFiles() {
      return [
        "apps/",
        "apps/web/",
        "apps/web/src/",
        "apps/web/src/routes/pricing.tsx",
        "apps/web/src/components/plan-card.tsx",
        "apps/web/src/lib/plans.ts",
        "apps/web/test/pricing.test.tsx",
        "package.json",
        "README.md",
      ]
    },
    async changes(_directory, sessionId) {
      return sessionId === "ses_pricing" ? [planCard, pricing, test] : []
    },
    async restoreFile() {},
    async gitStatus(directory) {
      if (directory !== WEB) return { repo: false }
      return {
        repo: true,
        branch: "fix/yearly-price",
        detached: false,
        upstream: state === "pushed" ? "origin/fix/yearly-price" : null,
        ahead: state === "pushed" ? 0 : 1,
        behind: 0,
        files: [
          { file: planCard.file, status: "modified" },
          { file: pricing.file, status: "modified" },
          { file: test.file, status: "added" },
        ],
        remote: "origin",
        gh: true,
      }
    },
    async gitBranch() {},
    async gitCommit() {
      return { commit: "4f2a9c1" }
    },
    async gitPush() {},
    async gitPr() {
      return { url: "https://github.com/anysola/web/pull/412", created: true }
    },
    async suggestCommitMessage() {
      await wait(500)
      return "Show the selected cycle's price on the plan cards"
    },
    async reveal() {},
    async scratchFolder() {
      return "/home/alex/.anysola/no-project"
    },
    async account() {
      return account
    },
    async updateCheck() {
      return state === "update" ? { available: true, version: "1.0.6" } : { available: false }
    },
    async updateInstall() {
      await wait(1_000_000)
    },
    async logout() {
      signedIn = false
      return { signedIn }
    },
    async folderInfo() {
      return info
    },
    async selectModel(_directory, _sessionId, model, variant) {
      const m = info.models.find((x) => x.id === model) ?? info.models[0]
      return { ...info, model: m.name, modelId: m.id, variant }
    },
    async prompt(directory, { sessionId, text, files }) {
      const id = sessionId ?? `ses_${Math.random().toString(36).slice(2, 10)}`
      if (!sessionId) {
        rows.unshift({ id, title: "New session", directory, updated: Date.now() })
        emit({ type: "created", sessionId: id, title: "New session", directory })
      }
      if (state === "refused") {
        // The core answers after the spawned turn has already said no.
        emit({ type: "refused", sessionId: id, error: "This model doesn’t read images." })
        await wait(50)
        return { sessionId: id, queued: false }
      }
      if (state === "stuck") return { sessionId: id, queued: false }
      const list = transcripts.get(id) ?? []
      transcripts.set(id, [...list, { kind: "user", id: `u${list.length}`, text, images: files.length }])
      emit({ type: "changed", sessionId: id })
      emit({ type: "started", sessionId: id })
      const words = "I’ll start by reading the files this touches, then make the change.".split(" ")
      void (async () => {
        await wait(400)
        for (const w of words) {
          emit({ type: "text", sessionId: id, messageId: "live", ordinal: 0, text: `${w} ` })
          await wait(60)
        }
      })()
      return { sessionId: id, queued: false }
    },
    async setMode() {},
    async interrupt(_directory, sessionId) {
      if (state !== "stuck") emit({ type: "interrupted", sessionId, reason: "user" })
    },
    async replyPermission(_directory, requestId) {
      emit({ type: "permissionReplied", sessionId: "ses_pricing", requestId })
      return true
    },
    async replyQuestion(_directory, requestId) {
      emit({ type: "permissionReplied", sessionId: "ses_pricing", requestId })
      return true
    },
    async renameSession(sessionId, title) {
      const row = rows.find((r) => r.id === sessionId)
      if (row) row.title = title
    },
    async deleteSession(_directory, sessionId) {
      const at = rows.findIndex((r) => r.id === sessionId)
      if (at >= 0) rows.splice(at, 1)
      return at >= 0
    },
    async pickFolder() {
      return WEB
    },
    async openUrl() {},
    onEvent(handler) {
      handlers.add(handler)
      return () => handlers.delete(handler)
    },
    window: { minimize() {}, toggleMaximize() {}, close() {} },
  }
}
