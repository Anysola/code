/**
 * The bridge outside Tauri: example sessions and a transcript, for working on the window in a
 * browser and for screenshots. `?state=` picks a moment: `signed-out`, `empty`, `approval`
 * (the default), `running`, `failed`, `question`, `update`, `rewound`, `plan`, `todos`, `done`.
 * Prompts in `refused` are refused before their answer comes back; in `stuck` they send no
 * events and Stop sends none either, as when nothing was running. `long` has long messages,
 * code, diffs and file lists; `event` a promotion; `limited` a full 5-hour window.
 *
 * Settings: `?settings=<section>` opens one; `?fail=usage,mcpList` makes those calls fail,
 * `?delay=800` slows every settings call, `?theme=light` previews a theme, `?empty=1` empties
 * every settings list, `?changes=open&commands=open&lookups=listed` start with those preferences.
 */
import type {
  About,
  Account,
  AgentConfig,
  Bridge,
  CoreEvent,
  DataInfo,
  Device,
  FileDiff,
  Info,
  Item,
  McpServer,
  PermissionRule,
  Prefs,
  Revert,
  SavedPermission,
  SessionRow,
  Sources,
  Todo,
  Usage,
  View,
} from "./bridge"
import { DEFAULT_PREFS } from "./prefs"

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

const iso = (ms: number) => new Date(ms).toISOString()

function accountFor(state: string, bank: number): Account {
  const limited = state === "limited"
  return {
    expired: false,
    plan: "pro",
    included: true,
    fiveHour: { percent: limited ? 100 : 34, resetsAt: iso(now + (limited ? 1.2 : 2.4) * 3_600_000) },
    weekly: { percent: limited ? 61 : 12, resetsAt: iso(now + 4 * 86_400_000) },
    blockedBy: limited ? "five_hour" : null,
    bank: [
      { id: "rst_1", expiresAt: iso(now + 9 * 86_400_000), scope: "five_hour" as const },
      { id: "rst_2", expiresAt: iso(now + 23 * 86_400_000), scope: "both" as const },
    ].slice(2 - bank),
    requestsToday: 143,
    event:
      state === "event"
        ? { mode: "multiplier", label: "Launch week: use counts at half", until: iso(now + 3 * 86_400_000) }
        : null,
  }
}

// ---------------------------------------------------------------------------
// `?state=long`: everything that can grow, at a length that tests its cap
// ---------------------------------------------------------------------------

const longCode = Array.from({ length: 60 }, (_, i) =>
  i === 0
    ? "export const yearly: Record<PlanId, number> = {"
    : i === 59
      ? "}"
      : `  plan_${String(i).padStart(2, "0")}: ${(i * 12.5).toFixed(2)}, // ${i % 7 === 0 ? "rounded to the nearest cent after the 10× discount, the same way the monthly table does it" : "monthly × 10"}`
).join("\n")

const longMessage = [
  "The yearly toggle on /pricing needs another pass. What I see:",
  "",
  "1. The cards still show the monthly price after switching.",
  "2. The team card has no yearly price at all.",
  "3. The enterprise card says “Contact us” in both cycles, which is right.",
  "4. The FAQ under the cards talks about monthly billing only.",
  "5. On mobile the toggle wraps under the heading.",
  "6. The checkout link keeps ?cycle=monthly.",
  "7. Analytics logs the cycle as undefined.",
  "",
  "Fix 1, 2, 6 and 7 now. Leave the FAQ copy for later, I’ll write it.",
  "Add tests for the cards and the checkout link.",
  "Don’t touch the enterprise card.",
  "Run the web tests when you’re done.",
].join("\n")

function bigPatch(file: string, lines: number): FileDiff {
  const body = Array.from({ length: lines }, (_, i) =>
    i % 5 === 0 ? `-  old_${i}: legacyPrice(plan, "${i}"),\n+  new_${i}: price(plan, cycle, "${i}"),` : `   keep_${i}: plan.features[${i}],`
  ).join("\n")
  const adds = Math.ceil(lines / 5)
  return {
    file,
    status: "modified",
    additions: adds,
    deletions: adds,
    patch: `Index: ${file}\n===\n--- ${file}\n+++ ${file}\n@@ -1,${lines} +1,${lines} @@\n${body}\n`,
  }
}

const reviewFiles: FileDiff[] = [
  "apps/web/src/lib/plans.ts",
  "apps/web/src/lib/checkout.ts",
  "apps/web/src/lib/analytics.ts",
  "apps/web/src/components/plan-card.tsx",
  "apps/web/src/components/cycle-toggle.tsx",
  "apps/web/src/routes/pricing.tsx",
  "apps/web/test/pricing.test.tsx",
  "apps/web/test/checkout.test.ts",
].map((f, i) => bigPatch(f, 6 + i * 3))

const longItems: Item[] = [
  { kind: "user", id: "l1", text: longMessage, images: 0 },
  tool("l2", "read", "apps/web/src/routes/pricing.tsx", { output: "64 lines" }),
  tool("l3", "read", "apps/web/src/lib/plans.ts", { output: "188 lines" }),
  tool("l4", "grep", "cycle=monthly", { output: "2 matches" }),
  tool("l5", "glob", "apps/web/test/**/*.test.ts*", { output: "14 files" }),
  {
    kind: "text",
    id: "l6:0",
    text: `The yearly table is generated from the monthly one. Here it is as it stands:\n\n\`\`\`ts\n${longCode}\n\`\`\`\n\nI’ll read the price from the selected cycle everywhere it’s shown.`,
  },
  tool("l7", "edit", reviewFiles[0].file, { files: [reviewFiles[0]] }),
  tool("l8", "shell", "pnpm --filter @anysola/web typecheck", {
    status: "error",
    error: `src/lib/checkout.ts:41:18 - error TS2345: Argument of type 'Cycle | undefined' is not assignable to parameter of type 'Cycle'.\n  Type 'undefined' is not assignable to type 'Cycle'.\n\n41   return link(plan, params.get("cycle"))\n                    ~~~~~~~~~~~~~~~~~~~\n\nFound 1 error in src/lib/checkout.ts:41`,
  }),
]

const longRevert: Revert = {
  turns: 3,
  files: reviewFiles.concat(bigPatch("apps/web/src/lib/format.ts", 4)).map((f) => ({
    file: f.file,
    status: f.status,
    additions: f.additions,
    deletions: f.deletions,
  })),
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

const HOME = "/home/alex"
const CONFIG = `${HOME}/.anysola/config.json`

const devices0: Device[] = [
  {
    id: "tok_1",
    name: "alex-desktop",
    client: "Anysola Code for Linux 1.0.6",
    kind: "device",
    createdAt: iso(now - 40 * 86_400_000),
    lastUsedAt: iso(now - 60_000),
    expiresAt: null,
    current: true,
  },
  {
    id: "tok_2",
    name: "MacBook Air",
    client: "anysola CLI 1.0.5",
    kind: "device",
    createdAt: iso(now - 90 * 86_400_000),
    lastUsedAt: iso(now - 3 * 86_400_000),
    expiresAt: null,
    current: false,
  },
  {
    id: "tok_3",
    name: "GitHub Actions: anysola/web",
    client: "anysola CLI 1.0.4",
    kind: "ci",
    createdAt: iso(now - 20 * 86_400_000),
    lastUsedAt: iso(now - 9 * 3_600_000),
    expiresAt: iso(now + 70 * 86_400_000),
    current: false,
  },
]

function usageFor(days: number): Usage {
  const byDay = Array.from({ length: days }, (_, i) => {
    const d = new Date(now - (days - 1 - i) * 86_400_000)
    const weekday = d.getUTCDay()
    const weekend = weekday === 0 || weekday === 6
    // A working week's shape, a quiet stretch three weeks back, and today so far.
    const base = weekend ? 18 : 120
    const wave = Math.round(40 * Math.sin(i * 1.7) + 25 * Math.cos(i * 0.6))
    const quiet = days - 1 - i >= 19 && days - 1 - i <= 23 ? 0.15 : 1
    const requests = i === days - 1 ? 143 : Math.max(0, Math.round((base + wave) * quiet))
    return { date: d.toISOString().slice(0, 10), requests }
  })
  const requests = byDay.reduce((n, d) => n + d.requests, 0)
  return {
    days,
    requests,
    cacheHitRate: 0.71,
    today: byDay[byDay.length - 1].requests,
    byDay,
    byModel: [
      { model: "claude-opus-5-5", name: "Opus 5.5", requests: Math.round(requests * 0.62) },
      { model: "claude-sonnet-5-5", name: "Sonnet 5.5", requests: Math.round(requests * 0.31) },
      { model: "claude-haiku-5-5", name: "Haiku 5.5", requests: requests - Math.round(requests * 0.62) - Math.round(requests * 0.31) },
    ],
  }
}

function mcp0(): McpServer[] {
  return [
    {
      name: "github",
      kind: "remote",
      target: "https://api.githubcopilot.com/mcp/",
      status: "connected",
      error: null,
      source: CONFIG,
      editable: true,
      oauth: false,
      signedIn: false,
      tools: 38,
      prompts: 2,
    },
    {
      name: "linear",
      kind: "remote",
      target: "https://mcp.linear.app/mcp",
      status: "needs_auth",
      error: null,
      source: CONFIG,
      editable: true,
      oauth: true,
      signedIn: false,
      tools: null,
      prompts: 0,
    },
    {
      name: "postgres",
      kind: "local",
      target: "npx -y @modelcontextprotocol/server-postgres postgresql://localhost:5432/anysola_dev",
      status: "failed",
      error:
        "MCP error -32000: Connection closed\nnpm error code E404\nnpm error 404 Not Found - GET https://registry.npmjs.org/@modelcontextprotocol%2fserver-postgres - Not found",
      source: CONFIG,
      editable: true,
      oauth: false,
      signedIn: false,
      tools: null,
      prompts: 0,
    },
    {
      name: "figma",
      kind: "remote",
      target: "http://127.0.0.1:3845/mcp",
      status: "pending",
      error: null,
      source: CONFIG,
      editable: true,
      oauth: false,
      signedIn: false,
      tools: null,
      prompts: 0,
    },
    {
      name: "sentry",
      kind: "remote",
      target: "https://mcp.sentry.dev/mcp",
      status: "disabled",
      error: null,
      source: CONFIG,
      editable: true,
      oauth: true,
      signedIn: true,
      tools: null,
      prompts: 0,
    },
    {
      name: "playwright",
      kind: "local",
      target: "npx @playwright/mcp@0.0.41 --headless",
      status: "connected",
      error: null,
      source: `${WEB}/anysola.json`,
      editable: false,
      oauth: false,
      signedIn: false,
      tools: 21,
      prompts: 0,
    },
  ]
}

function sourcesFor(directory: string): Sources {
  const project = directory
  return {
    skills: [
      { name: "release-notes", description: "Draft release notes from the commits since the last tag", path: `${HOME}/.anysola/skills/release-notes/SKILL.md`, origin: "personal" },
      { name: "changelog-style", description: "House style for changelog entries", path: `${HOME}/.anysola/skills/changelog-style/SKILL.md`, origin: "personal" },
      { name: "pricing-copy", description: "How plan names and prices are written on the site", path: `${project}/.anysola/skills/pricing-copy/SKILL.md`, origin: "project" },
      { name: "frontend-design", description: "Distinctive, intentional visual design for new UI", path: `${HOME}/.claude/skills/frontend-design/SKILL.md`, origin: "claude" },
      { name: "pdf", description: "Read, fill and create PDF files", path: `${HOME}/.claude/skills/pdf/SKILL.md`, origin: "claude" },
      { name: "e2e-playwright", description: null, path: `${project}/.agents/skills/e2e-playwright/SKILL.md`, origin: "agents" },
    ],
    commands: [
      { name: "init", description: "create/update AGENTS.md", origin: "builtin", source: null },
      { name: "review", description: "review changes [commit|branch|pr], defaults to uncommitted", origin: "builtin", source: null },
      { name: "goal", description: "keep working until an objective is met [pause|resume|clear]", origin: "builtin", source: null },
      { name: "release-notes", description: "Draft release notes from the commits since the last tag", origin: "personal", source: `${HOME}/.anysola/commands/release-notes.md` },
      { name: "deploy-check", description: "Check the build before a deploy", origin: "project", source: `${project}/.anysola/commands/deploy-check.md` },
      { name: "pr-summary", description: "Summarize a pull request", origin: "mcp", source: "github" },
      { name: "issue-triage", description: "Label and route a new issue", origin: "mcp", source: "github" },
    ],
    agents: [
      { id: "build", name: "Build", description: "The default agent.", origin: "builtin", source: null },
      { id: "plan", name: "Plan", description: "Read-only agent for planning.", origin: "builtin", source: null },
    ],
    instructions: [
      { path: `${HOME}/.anysola/AGENTS.md`, bytes: 1_840, origin: "personal" },
      { path: `${project}/AGENTS.md`, bytes: 6_212, origin: "project" },
      { path: `${project}/CLAUDE.md`, bytes: 2_390, origin: "claude" },
    ],
    places: [
      { path: `${HOME}/.anysola/skills`, holds: "skills", origin: "personal", exists: true },
      { path: `${project}/.anysola/skills`, holds: "skills", origin: "project", exists: true },
      { path: `${HOME}/.claude/skills`, holds: "skills", origin: "claude", exists: true },
      { path: `${project}/.claude/skills`, holds: "skills", origin: "claude", exists: false },
      { path: `${project}/.agents/skills`, holds: "skills", origin: "agents", exists: true },
      { path: `${HOME}/.opencode/skills`, holds: "skills", origin: "legacy", exists: false },
      { path: `${HOME}/.anysola/commands`, holds: "commands", origin: "personal", exists: true },
      { path: `${project}/.anysola/commands`, holds: "commands", origin: "project", exists: true },
      { path: `${HOME}/.anysola/AGENTS.md`, holds: "instructions", origin: "personal", exists: true },
      { path: `${project}/AGENTS.md`, holds: "instructions", origin: "project", exists: true },
      { path: `${project}/CLAUDE.md`, holds: "instructions", origin: "claude", exists: true },
      { path: `${HOME}/.claude/CLAUDE.md`, holds: "instructions", origin: "claude", exists: false },
      { path: CONFIG, holds: "config", origin: "personal", exists: true },
      { path: `${project}/anysola.json`, holds: "config", origin: "project", exists: true },
    ],
  }
}

const saved0: SavedPermission[] = [
  { projectId: "prj_web", folder: WEB, action: "shell", resource: "pnpm --filter @anysola/web test *" },
  { projectId: "prj_web", folder: WEB, action: "shell", resource: "pnpm typecheck" },
  { projectId: "prj_web", folder: WEB, action: "webfetch", resource: "https://developer.mozilla.org/*" },
  { projectId: "prj_code", folder: CODE, action: "shell", resource: "cargo test -p anysola-gateway" },
  { projectId: "prj_code", folder: CODE, action: "external_directory", resource: `${HOME}/.cargo/registry/*` },
]

const rules: PermissionRule[] = [
  { action: "read", resource: "*.env", effect: "ask", source: CONFIG },
  { action: "shell", resource: "rm -rf *", effect: "deny", source: CONFIG },
  { action: "shell", resource: "git push *", effect: "ask", source: `${WEB}/anysola.json` },
  { action: "webfetch", resource: "*", effect: "allow", source: CONFIG },
]

function config0(): AgentConfig {
  return {
    file: CONFIG,
    model: null,
    models: info.models,
    websearch: true,
    todo: true,
    subagentDepth: 2,
    autoCompact: true,
    snapshots: true,
    shell: null,
    shellDetected: "/usr/bin/zsh",
    overridden: [{ key: "websearch", file: `${WEB}/anysola.json` }],
    warnings: [],
  }
}

function data0(): DataInfo {
  return {
    folder: `${HOME}/.anysola`,
    database: { path: `${HOME}/.anysola/anysola.db`, bytes: 48_211_968 },
    sessions: rows.length + 212,
    caches: [
      { id: "tool-output", path: `${HOME}/.anysola/tool-output`, bytes: 31_457_280 },
      { id: "cache", path: `${HOME}/.anysola/cache`, bytes: 214_958_080 },
      { id: "shell", path: `${HOME}/.anysola/shell`, bytes: 1_310_720 },
    ],
    log: `${HOME}/.anysola/log/desktop.log`,
    imported: { sessions: 186, at: now - 12 * 86_400_000 },
  }
}

const about: About = { version: "1.0.6", os: "Linux 6.8 (Ubuntu 24.04)", arch: "x86_64" }

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
  if (state === "long") transcripts.set("ses_pricing", longItems)
  let signedIn = state !== "signed-out"
  let revert: Revert | null = state === "rewound" ? rewound : null
  const params = new URLSearchParams(location.search)
  const failing = new Set((params.get("fail") ?? "").split(",").filter(Boolean))
  const delay = Number(params.get("delay")) || 0
  /** A settings call: slowed by `?delay=`, refused by `?fail=`. */
  const settle = async <T,>(name: string, value: () => T): Promise<T> => {
    await wait(delay || 120)
    if (failing.has(name)) throw new Error("The gateway didn’t answer (502). Try again in a moment.")
    return value()
  }
  const empty = params.has("empty")
  let prefs: Prefs = structuredClone({ ...DEFAULT_PREFS, discord: { ...DEFAULT_PREFS.discord, enabled: !empty } })
  for (const key of ["changes", "commands", "lookups"] as const) {
    const v = params.get(key)
    if (v) (prefs.conversation as Record<string, unknown>)[key] = v
  }
  let bank = empty ? 0 : 2
  let devices = empty ? devices0.filter((d) => d.current) : [...devices0]
  let servers = empty ? [] : mcp0()
  let saved = empty ? [] : [...saved0]
  let config = config0()
  let data = empty ? { ...data0(), imported: null, sessions: 0 } : data0()
  let personal = "Reply in short, plain sentences.\nUse pnpm, never npm, in the web repos.\nRun the tests that cover a change before saying it is done.\n"
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
        if (state === "long")
          emit({
            type: "permission",
            sessionId: session.id,
            directory: session.directory,
            requestId: "per_long",
            action: "edit",
            resources: [reviewFiles[3].file],
            canAlways: true,
            files: [bigPatch(reviewFiles[3].file, 80)],
          })
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
      const busy = session.id === "ses_pricing" && ["approval", "running", "question", "long"].includes(state)
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
    async revert(directory, sessionId, messageId) {
      const items = transcripts.get(sessionId) ?? []
      const text = items.find((i) => i.id === messageId)
      revert = state === "long" ? longRevert : rewound
      // Only the web folder is a git repository here (see gitStatus).
      return { text: text?.kind === "user" ? text.text : "", view: view(sessionId), filesKept: directory !== WEB }
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
      if (sessionId !== "ses_pricing") return []
      return state === "long" ? reviewFiles : [planCard, pricing, test]
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
      return accountFor(state, bank)
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
    async revealPath() {},
    prefsGet: () => settle("prefsGet", () => structuredClone(prefs)),
    prefsSet: (next) =>
      settle("prefsSet", () => {
        prefs = structuredClone(next)
        return structuredClone(prefs)
      }),
    devices: () => settle("devices", () => devices),
    deviceRevoke: (id) =>
      settle("deviceRevoke", () => {
        devices = devices.filter((d) => d.id !== id)
        return devices
      }),
    usage: (days) =>
      settle("usage", () => {
        const u = usageFor(days)
        if (!empty) return u
        return { ...u, requests: 0, today: 0, cacheHitRate: null, byDay: u.byDay.map((d) => ({ ...d, requests: 0 })), byModel: [] }
      }),
    limitsRedeem: () =>
      settle("limitsRedeem", () => {
        bank = Math.max(0, bank - 1)
        const a = accountFor(state, bank)
        if (a.expired) return a
        return { ...a, fiveHour: a.fiveHour && { ...a.fiveHour, percent: 0 }, blockedBy: null }
      }),
    configGet: (_directory) => settle("configGet", () => config),
    configSet: (_directory, patch) =>
      settle("configSet", () => {
        config = { ...config, ...patch }
        return config
      }),
    mcpList: () => settle("mcpList", () => servers),
    mcpAdd: (_directory, draft) =>
      settle("mcpAdd", () => {
        if (servers.some((s) => s.name === draft.name)) throw new Error(`A server named “${draft.name}” already exists.`)
        servers = [
          ...servers,
          {
            name: draft.name,
            kind: draft.kind,
            target: draft.kind === "remote" ? draft.url : draft.command.join(" "),
            status: "pending",
            error: null,
            source: CONFIG,
            editable: true,
            oauth: false,
            signedIn: false,
            tools: null,
            prompts: 0,
          },
        ]
        setTimeout(() => {
          servers = servers.map((s) => (s.name === draft.name ? { ...s, status: "connected", tools: 12 } : s))
          emit({ type: "mcp", directory: WEB })
        }, 1500)
        return servers
      }),
    mcpRemove: (_directory, name) =>
      settle("mcpRemove", () => {
        servers = servers.filter((s) => s.name !== name)
        return servers
      }),
    mcpEnable: (_directory, name, enabled) =>
      settle("mcpEnable", () => {
        servers = servers.map((s) =>
          s.name !== name
            ? s
            : enabled
              ? { ...s, status: s.oauth && !s.signedIn ? "needs_auth" : "connected", tools: s.oauth && !s.signedIn ? null : 17 }
              : { ...s, status: "disabled", tools: null, error: null }
        )
        return servers
      }),
    mcpRetry: (_directory, name) =>
      settle("mcpRetry", () => {
        servers = servers.map((s) => (s.name === name ? { ...s, status: "pending", error: null } : s))
        setTimeout(() => {
          servers = servers.map((s) =>
            s.name === name ? { ...s, status: "failed", error: "MCP error -32000: Connection closed\nnpm error code E404" } : s
          )
          emit({ type: "mcp", directory: WEB })
        }, 1200)
        return servers
      }),
    mcpSignIn: (_directory, name) =>
      settle("mcpSignIn", () => ({ url: `https://mcp.${name}.app/oauth/authorize?client_id=anysola`, instructions: null })),
    mcpSignInWait: async (_directory, name) => {
      await wait(4000)
      servers = servers.map((s) => (s.name === name ? { ...s, status: "connected", signedIn: true, tools: 23 } : s))
      return servers
    },
    mcpSignOut: (_directory, name) =>
      settle("mcpSignOut", () => {
        servers = servers.map((s) => (s.name === name ? { ...s, status: "needs_auth", signedIn: false, tools: null } : s))
        return servers
      }),
    sources: (directory) =>
      settle("sources", () => {
        const all = sourcesFor(directory)
        if (empty) return { ...all, skills: [], commands: all.commands.filter((c) => c.origin === "builtin"), instructions: [] }
        // Off, Claude Code's folders aren't read.
        return prefs.sources.claudeSkills ? all : { ...all, skills: all.skills.filter((x) => x.origin !== "claude") }
      }),
    personalInstructions: () => settle("personalInstructions", () => ({ path: `${HOME}/.anysola/AGENTS.md`, text: personal })),
    savePersonalInstructions: (text) =>
      settle("savePersonalInstructions", () => {
        personal = text
      }),
    permissions: () => settle("permissions", () => ({ saved, rules: empty ? [] : rules })),
    permissionForget: (projectId, action, resource) =>
      settle("permissionForget", () => {
        saved = saved.filter((p) => !(p.projectId === projectId && p.action === action && p.resource === resource))
        return saved
      }),
    permissionForgetAll: (projectId) =>
      settle("permissionForgetAll", () => {
        saved = saved.filter((p) => p.projectId !== projectId)
        return saved
      }),
    dataInfo: () => settle("dataInfo", () => data),
    dataClear: (id) =>
      settle("dataClear", () => {
        data = { ...data, caches: data.caches.map((c) => (c.id === id ? { ...c, bytes: 0 } : c)) }
        return data
      }),
    deleteAllSessions: () =>
      settle("deleteAllSessions", () => {
        // The open session's turn is running in most states, so it stays.
        const kept = rows.filter((r) => r.id === "ses_pricing")
        const deleted = data.sessions - kept.length
        rows.splice(0, rows.length, ...kept)
        data = { ...data, sessions: kept.length }
        return { deleted, kept: kept.length }
      }),
    exportSession: (_directory, sessionId) => settle("exportSession", () => `${HOME}/Downloads/${sessionId}.json`),
    about: () => settle("about", () => about),
    notifyTest: () => settle("notifyTest", () => undefined),
    discordStatus: () =>
      settle("discordStatus", () =>
        prefs.discord.enabled
          ? { state: "connected" as const, detail: "Discord 0.0.92 on this computer" }
          : { state: "off" as const, detail: null }
      ),
    async presence() {},
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
