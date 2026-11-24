/**
 * The bridge outside Tauri: example sessions and a transcript, for working on the window in a
 * browser and for screenshots. `?state=` picks a moment: `signed-out`, `empty`, `approval`
 * (the default), `running`, `failed`, `question`, `update`, `rewound`, `plan`, `todos`, `done`.
 * Prompts in `refused` are refused before their answer comes back; in `stuck` they send no
 * events and Stop sends none either, as when nothing was running. `long` has long messages,
 * code, diffs and file lists; `event` a promotion; `limited` a full 5-hour window; `crowded` a
 * running turn with three queued messages, background commands and a full 5-hour window at once.
 *
 * The conversation: `queue` (a running turn with messages waiting), `queue-stopped` (a stopped
 * turn left them), `context` (a nearly full context), `background` (commands left running),
 * `offline` (the account can't be read; a send fails for it), `unsent` (a send fails before its
 * turn starts: the text comes back), `restored` (asks that waited before the window opened come
 * back from `pending`), `form` (an MCP server's form) and `form-url` (one that opens a page),
 * `closing` (the window was closed mid-turn), `fault` (the database can't be opened), `gone`
 * (every answer finds its request no longer waiting), `images` (a message with images),
 * `failed&kind=limit|offline|auth|model|elsewhere` (a turn failed for that reason), `updating`
 * (an update while a turn runs). `?fail=replyPermission,setMode,queueSteer,changes,gitStatus…` makes
 * those calls throw. `?slow=1500` makes a new session's first prompt take that long; `?hang=1`
 * makes Stop send nothing while the turn goes on. `window.__mockEmit(event)` sends a core event.
 *
 * Hooks: `?hooks=none|user|waiting|allowed|refused|managed|all|off|locked|emptied|changed` picks what the folder has (`user`
 * by default; `emptied`: a project file whose allowed hooks were removed; `locked`: off through a variable, so the switch can't turn them on; `waiting` shows the card for a project's hooks). `?state=hooks` has tool rows a hook
 * blocked, spoke about or failed on, and a hook's error notice; in `hook-blocked` a hook stops
 * the message sent (`&blocks=2`: two hooks do), in the core's order: `started`, then the
 * hook, then `failed` + `unsent`. `hook-blocked-queued`: a turn runs, and a hook stops the message
 * sent into its queue when the turn takes it.
 *
 * What a project wants to run: `?trust=all` (a file with hooks, MCP servers and formatters, all
 * waiting), `mcp` (only its servers wait), `formatter` (only its formatters, `true`), `refused` (its
 * servers were refused), `replaces` (one of its servers redefines your github, with long details
 * and the file's MCP timeouts), `permissions` / `remote` (only its allow rules / downloads wait),
 * `permissions-refused` / `remote-refused`, `five` (one file with all five kinds waiting),
 * `agentfile` (anysola.json's rules and an agent file's, two cards), `wildcard` (rules whose
 * action is a pattern, and one whose text tries to pass as part of the sentence). Answers go through `trustAnswer`, logged in `window.__trustCalls`.
 *
 * Settings: `?settings=<section>` opens one; `?fail=usage,mcpList` makes those calls fail,
 * `?delay=800` slows every settings call, `?theme=light` previews a theme, `?empty=1` empties
 * every settings list, `?changes=open&commands=open&lookups=listed` start with those preferences.
 *
 * Review: `?review=long` gives the session five long changed files; `?review=nogit` makes its folder
 * not a git repository, so Review lists the session's edits (`long,nogit`: the long ones, as edits).
 *
 * Worktrees (only the web folder is a git repository): `?state=worktree-preparing|ready|conflict|
 * missing|failed|discarded|notice|applied` puts the open session in its own worktree in that state
 * (`conflict`: ready, the base brought in with markers left; `applied`: ready again after an apply;
 * `notice`: ready with an apply cut off; `preparing`/`failed`: its first message waits).
 * `?review=apply-conflict|apply-dirty|apply-staged|apply-detached|apply-operation|apply-empty|
 * apply-busy|apply-git|apply-onto` makes the apply check answer so (`apply-onto`: the main folder is on
 * another branch; `apply-stash-left` with `apply-dirty`: the edits set aside don't go back).
 * `?worktree=old-git` makes the folder's Git too old to apply; `?worktree=no-commits` hides the
 * choice. `?setup=hold` keeps a `pnpm install` setup running (preparing stays), `?setup=fail` makes
 * it exit 1 (with a ready scene: a failed run already there). New sessions there start in a worktree
 * (preparing ~2 s, setup output streamed), forks get one with `?restored=kept|edits|now` as the report.
 */
import type {
  About,
  ApplyCheck,
  ForkRestore,
  Account,
  AgentConfig,
  BackgroundCommand,
  Bridge,
  CoreEvent,
  DataInfo,
  Device,
  ErrorKind,
  FileDiff,
  FormRequest,
  Hook,
  HookDraft,
  HookFile,
  Hooks,
  Info,
  TrustItem,
  TrustKind,
  Item,
  McpServer,
  PendingRequest,
  PermissionRule,
  Prefs,
  Queued,
  Revert,
  SavedPermission,
  SessionRow,
  SessionUsage,
  SessionWorktree,
  SetupRun,
  Sources,
  Todo,
  Usage,
  View,
} from "./bridge"
import { MAX_BUDGET } from "./bridge"
import { DEFAULT_PREFS } from "./prefs"

const WEB = "/home/alex/code/anysola-web"
const CODE = "/home/alex/code/anysola-code"
const MAIL = "/home/alex/code/mailwick"
const now = Date.now()

const rows: SessionRow[] = [
  { id: "ses_pricing", title: "Yearly price on /pricing", directory: WEB, updated: now - 20_000, forkOf: null, worktree: null },
  { id: "ses_alerts", title: "Gateway health alerts", directory: CODE, updated: now - 60_000, forkOf: null, worktree: null },
  { id: "ses_search", title: "News search ranking", directory: WEB, updated: now - 2 * 3_600_000, forkOf: null, worktree: null },
  { id: "ses_footer", title: "Footer links on mobile", directory: WEB, updated: now - 26 * 3_600_000, forkOf: null, worktree: null },
  { id: "ses_load", title: "Load test for streams", directory: CODE, updated: now - 30 * 3_600_000, forkOf: null, worktree: null },
  { id: "ses_imap", title: "IMAP idle reconnect", directory: MAIL, updated: now - 3 * 86_400_000, forkOf: null, worktree: null },
]

/** A session's worktree in the mock's web folder. */
function mockWorktree(slug: string, state: SessionWorktree["state"], extra: Partial<SessionWorktree> = {}): SessionWorktree {
  return {
    path: `/home/alex/.anysola/worktrees/anysola-web-3f9a1c/${slug}`,
    branch: `anysola/${slug}`,
    baseBranch: "main",
    baseCommit: "9c1e2a7f3b",
    state,
    error: null,
    appliedCommit: null,
    restored: null,
    leftover: false,
    ...extra,
  }
}

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
    time: now - 3 * 60_000 - 12_000,
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
  tool("t5b", "subagent", "Check the other plan cards for the same bug", { child: "ses_child", elapsed: 48_000 }),
  tool("t6", "write", test.file, { files: [test] }),
  {
    kind: "text",
    id: "m4:0",
    text: "The fix is in. The card now reads its price this way:\n\n```tsx\nconst price = plan.prices[cycle]\n```\n\nRunning the pricing tests next.",
  },
  tool("t7", "shell", "pnpm --filter @anysola/web test pricing", { status: "running" }),
]

/** `?state=images` and `done`: an earlier, finished turn above the one shown. */
const earlierItems: Item[] = [
  {
    kind: "user",
    id: "e1",
    text: "Here’s how the pricing page looks on my phone. The toggle wraps under the heading.",
    images: 2,
    time: now - 26 * 60_000,
    elapsed: 2 * 60_000 + 14_000,
  },
  tool("e2", "read", "apps/web/src/routes/pricing.tsx", { output: "64 lines" }),
  tool("e3", "grep", "CycleToggle", {
    output: "apps/web/src/routes/pricing.tsx:24:      <CycleToggle value={cycle} onChange={setCycle} />\napps/web/src/components/cycle-toggle.tsx:8:export function CycleToggle({ value, onChange }: Props) {",
  }),
  {
    kind: "text",
    id: "e4:0",
    text: "The heading and the toggle share a row with no wrap rule. I moved the toggle under the heading below `sm` in [cycle-toggle.tsx](apps/web/src/components/cycle-toggle.tsx#L8); see the [Tailwind docs](https://tailwindcss.com/docs/responsive-design) for the breakpoints.",
  },
]

/** A small picture for `messageImages`. */
function picture(hue: number, label: string) {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="390" height="844" viewBox="0 0 390 844"><rect width="390" height="844" fill="hsl(${hue} 8% 14%)"/><rect x="24" y="96" width="342" height="40" rx="8" fill="hsl(${hue} 8% 24%)"/><rect x="24" y="152" width="200" height="28" rx="14" fill="hsl(${hue} 8% 30%)"/><rect x="24" y="210" width="342" height="260" rx="16" fill="hsl(${hue} 8% 20%)"/><text x="24" y="520" fill="hsl(${hue} 8% 60%)" font-family="sans-serif" font-size="20">${label}</text></svg>`
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`
}

const form: FormRequest = {
  type: "form",
  directory: WEB,
  formId: "frm_1",
  server: "linear",
  title: "Create an issue",
  message: "Linear needs a few details to file the bug in the Web team’s backlog.",
  fields: [
    { key: "title", title: "Title", type: "string", required: true, minLength: 4, maxLength: 80, default: "Yearly price missing on team card" },
    { key: "reporter", title: "Reporter email", type: "string", format: "email", required: true },
    { key: "link", title: "Link", type: "string", format: "uri", description: "Where it shows" },
    { key: "due", title: "Due date", type: "string", format: "date" },
    { key: "estimate", title: "Estimate (points)", type: "integer", minimum: 1, maximum: 8, default: 2 },
    {
      key: "priority",
      title: "Priority",
      type: "string",
      required: true,
      custom: false,
      options: [
        { value: "urgent", label: "Urgent" },
        { value: "high", label: "High" },
        { value: "medium", label: "Medium" },
        { value: "low", label: "Low" },
      ],
      default: "medium",
    },
    {
      key: "labels",
      title: "Labels",
      type: "multiselect",
      minItems: 1,
      maxItems: 2,
      options: [
        { value: "bug", label: "Bug" },
        { value: "pricing", label: "Pricing" },
        { value: "mobile", label: "Mobile" },
      ],
    },
    { key: "notify", title: "Notify the team", type: "boolean", default: true },
  ],
}

const formUrl: FormRequest = {
  type: "form",
  directory: WEB,
  formId: "frm_url",
  server: "github",
  title: "Authorize",
  message: "GitHub wants you to approve access to anysola/web in your browser.",
  fields: [{ key: "auth", title: "Approve in GitHub", type: "external", url: "https://github.com/login/oauth/authorize?client_id=anysola" }],
}

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
  const limited = state === "limited" || state === "crowded"
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

/** `?review=long`: five long diffs, few enough that Review opens them all. */
const longReview: FileDiff[] = [
  "apps/web/src/lib/plans.ts",
  "apps/web/src/lib/checkout.ts",
  "apps/web/src/components/plan-card.tsx",
  "apps/web/src/routes/pricing.tsx",
  "apps/web/test/pricing.test.tsx",
].map((f, i) => bigPatch(f, 48 + i * 18))

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
    skillUrls: [],
    repositories: [],
  }
}

const saved0: SavedPermission[] = [
  { projectId: "prj_web", folder: WEB, action: "shell", resource: "pnpm --filter @anysola/web test *" },
  { projectId: "prj_web", folder: WEB, action: "shell", resource: "pnpm typecheck" },
  { projectId: "prj_web", folder: WEB, action: "webfetch", resource: "https://developer.mozilla.org/*" },
  { projectId: "prj_code", folder: CODE, action: "shell", resource: "cargo test -p anysola-gateway" },
  { projectId: "prj_code", folder: CODE, action: "external_directory", resource: `${HOME}/.cargo/registry/*` },
]

const rule = (action: string, resource: string, effect: PermissionRule["effect"], source: string, agent: string | null = null): PermissionRule => ({
  action,
  resource,
  effect,
  agent,
  source,
  project: source !== CONFIG,
  applied: true,
})

const rules: PermissionRule[] = [
  rule("read", "*.env", "ask", CONFIG),
  rule("shell", "rm -rf *", "deny", CONFIG),
  rule("webfetch", "*", "allow", CONFIG),
  rule("shell", "git push *", "ask", CONFIG, "build"),
  rule("shell", "git push *", "ask", `${WEB}/anysola.json`),
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

// ---------------------------------------------------------------------------
// Hooks
// ---------------------------------------------------------------------------

type MockHook = Omit<Hook, "active">

const userHooks0: MockHook[] = [
  {
    id: "hk_1",
    event: "PreToolUse",
    matcher: "Bash",
    command: "if grep -qE 'git push.*(-f|--force)'; then echo 'Force-pushing is off for the agent.' >&2; exit 2; fi",
    timeout: 600,
    source: CONFIG,
    scope: "user",
  },
  {
    id: "hk_2",
    event: "PostToolUse",
    matcher: "Edit|Write",
    command: "pnpm --filter @anysola/web lint --quiet >&2 || exit 2",
    timeout: 120,
    source: CONFIG,
    scope: "user",
  },
  { id: "hk_3", event: "SessionStart", matcher: "startup", command: "git log --oneline -5", timeout: 600, source: CONFIG, scope: "user" },
]

/** Set through a variable, so not this screen's to change. */
const envHook: MockHook = {
  id: null,
  event: "UserPromptSubmit",
  matcher: null,
  command: "~/bin/redact-secrets",
  timeout: 30,
  source: "ANYSOLA_CONFIG",
  scope: "user",
}

function projectFile(decision: HookFile["decision"]): Omit<HookFile, "hooks"> & { hooks: MockHook[] } {
  const path = `${WEB}/.anysola/anysola.json`
  const at = { source: path, scope: "project" as const, id: null }
  return {
    path,
    digest: "9f2c4e0b7a1d5c3e8f6a2b4d0c9e7f1a3b5d8c2e4f6a0b9c7d1e3f5a8b2c4d6e",
    decision,
    hooks: [
      { ...at, event: "PreToolUse", matcher: "Edit|Write", command: "node scripts/hooks/protect-generated.mjs", timeout: 600 },
      {
        ...at,
        event: "UserPromptSubmit",
        matcher: null,
        command: `node scripts/hooks/context.mjs --branch "$(git rev-parse --abbrev-ref HEAD)" --changed "$(git diff --name-only HEAD | tr '\\n' ',')" --limit 40`,
        timeout: 30,
      },
      { ...at, event: "Stop", matcher: null, command: "pnpm --filter @anysola/web test --run >&2 || exit 2", timeout: 300 },
    ],
  }
}

/** A file above the project folder that defines hooks too. */
function parentFile(decision: HookFile["decision"]): Omit<HookFile, "hooks"> & { hooks: MockHook[] } {
  const path = `${HOME}/code/anysola.json`
  return {
    path,
    digest: "1b3d5f7a9c2e4f6a8b0d2c4e6f8a1b3d5c7e9f2a4b6d8c0e2f4a6b8d1c3e5f7a",
    decision,
    hooks: [{ source: path, scope: "project", id: null, event: "SessionStart", matcher: null, command: "./tools/doctor.sh --quiet", timeout: 600 }],
  }
}

const managedHooks: MockHook[] = [
  {
    id: null,
    event: "PreToolUse",
    matcher: "mcp__.*",
    command: "/opt/acme/bin/audit-tool-call --policy strict",
    timeout: 10,
    source: "/etc/anysola/managed.json",
    scope: "managed",
  },
]

const hookItems: Item[] = [
  { kind: "user", id: "h1", text: "Fix the unused import the linter keeps flagging, then push the branch.", images: 0, time: now - 4 * 60_000 },
  tool("h2", "edit", planCard.file, {
    files: [planCard],
    hooks: [
      {
        event: "PostToolUse",
        command: "pnpm --filter @anysola/web lint --quiet >&2 || exit 2",
        outcome: "feedback",
        message:
          "apps/web/src/components/plan-card.tsx\n  14:7  error  'Cycle' is defined but never used  @typescript-eslint/no-unused-vars\n\n✖ 1 problem (1 error, 0 warnings)",
      },
    ],
  }),
  tool("h3", "read", "apps/web/src/lib/plans.ts", {
    output: "88 lines",
    hooks: [
      {
        event: "PostToolUse",
        command: "node scripts/hooks/generated-note.mjs",
        outcome: "context",
        message: "plans.ts is generated from plans.yaml. Edit the YAML and run pnpm gen instead.",
      },
    ],
  }),
  tool("h4", "edit", "apps/web/src/lib/plans.gen.ts", {
    status: "error",
    error: "A PreToolUse hook blocked this call: plans.gen.ts is generated. Edit plans.yaml and run pnpm gen.",
    hooks: [
      {
        event: "PreToolUse",
        command: "node scripts/hooks/protect-generated.mjs",
        outcome: "blocked",
        message: "plans.gen.ts is generated. Edit plans.yaml and run pnpm gen.",
      },
    ],
  }),
  { kind: "text", id: "h5:0", text: "The linter is clean now. Pushing the branch." },
  tool("h6", "shell", "git push --force origin fix/yearly-price", {
    status: "error",
    error: "A PreToolUse hook blocked this call: Force-pushing is off for the agent.",
    hooks: [
      {
        event: "PreToolUse",
        command: "if grep -qE 'git push.*(-f|--force)'; then echo 'Force-pushing is off for the agent.' >&2; exit 2; fi",
        outcome: "blocked",
        message: "Force-pushing is off for the agent.",
      },
    ],
  }),
  tool("h7", "shell", "git push origin fix/yearly-price", {
    output: "To github.com:anysola/web.git\n   4f2a9c1..8d0e3b7  fix/yearly-price -> fix/yearly-price",
    hooks: [
      {
        event: "PostToolUse",
        command: "~/bin/notify-push.sh",
        outcome: "error",
        message: "The hook exited with code 127: sh: 1: ~/bin/notify-push.sh: not found",
      },
    ],
  }),
  { kind: "text", id: "h8:0", text: "Pushed without force: the branch only had new commits on top, so a plain push was enough." },
]

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
  ;(window as unknown as { __mockEmit: typeof emit }).__mockEmit = emit
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
  if (state === "hooks") transcripts.set("ses_pricing", hookItems)
  if (state === "images" || state === "done") transcripts.set("ses_pricing", [...earlierItems, ...pricingItems])
  const review = new Set((new URLSearchParams(location.search).get("review") ?? "").split(","))
  if (review.has("long"))
    transcripts.set("ses_pricing", [
      ...transcripts.get("ses_pricing")!,
      ...longReview.map((f, i) => tool(`rl${i}`, "edit", f.file, { files: [f] })),
    ])
  // `?stopped=budget`: the last turn ended at the session's budget, with the core's note.
  if (new URLSearchParams(location.search).get("stopped") === "budget")
    transcripts.set("ses_pricing", [...transcripts.get("ses_pricing")!, { kind: "note", id: "nb", text: "Stopped at the session’s budget.", failed: false }])
  if (state === "done")
    transcripts.set(
      "ses_pricing",
      transcripts.get("ses_pricing")!.map((i) => (i.kind === "user" && i.id === "m1" ? { ...i, elapsed: 3 * 60_000 + 5_000 } : i))
    )
  const kind = (new URLSearchParams(location.search).get("kind") ?? undefined) as ErrorKind | undefined
  const BUSY = ["hook-blocked-queued", "crowded", "approval", "gone", "running", "question", "long", "queue", "background", "context", "updating", "closing", "restored"]
  /** Sessions with a turn running, and what waits in their queues. */
  const running = new Set<string>(BUSY.includes(state) ? ["ses_pricing"] : [])
  const queues = new Map<string, Queued[]>()
  if (state === "queue" || state === "queue-stopped" || state === "crowded")
    queues.set("ses_pricing", [
      { id: "q1", text: "Also check the team card while you’re in there.", images: 0, delivery: "queue" },
      { id: "q2", text: "And here’s the mobile layout I mentioned — the toggle should sit under the heading.", images: 1, delivery: "queue" },
      ...(state === "crowded" ? [{ id: "q3", text: "Then run the pricing tests once more.", images: 0, delivery: "queue" as const }] : []),
    ])
  const background: BackgroundCommand[] =
    state === "background" || state === "crowded"
      ? [
          { id: "bg1", sessionId: "ses_pricing", command: "pnpm --filter @anysola/web dev", started: now - 4 * 60_000 },
          { id: "bg2", sessionId: "ses_child", command: "pnpm vitest --watch pricing", started: now - 90_000 },
        ]
      : []
  let forms: FormRequest[] = state === "form" ? [form] : state === "form-url" ? [formUrl] : []
  let signedIn = state !== "signed-out"
  let revert: Revert | null = state === "rewound" ? rewound : null
  /** The user message a staged rewind starts at (the fixture's has none to name). */
  let revertAt: string | null = null
  const params = new URLSearchParams(location.search)
  const failing = new Set((params.get("fail") ?? "").split(",").filter(Boolean))
  const delay = Number(params.get("delay")) || 0
  /** A settings call: slowed by `?delay=`, refused by `?fail=`. */
  const settle = async <T,>(name: string, value: () => T): Promise<T> => {
    await wait(delay || 120)
    if (failing.has(name)) throw new Error("The gateway didn’t answer (502). Try again in a moment.")
    return value()
  }
  // `?state=worktree-*`: the open session works in its own worktree.
  const worktreeScene = state.startsWith("worktree-") ? state.slice("worktree-".length) : null
  if (worktreeScene) {
    const pricing = rows.find((r) => r.id === "ses_pricing")!
    pricing.worktree =
      worktreeScene === "preparing"
        ? mockWorktree("brave-otter", "preparing")
        : worktreeScene === "missing"
          ? mockWorktree("brave-otter", "missing")
          : worktreeScene === "failed"
            ? mockWorktree("brave-otter", "failed", { error: "Making the worktree was cut off." })
            : worktreeScene === "discarded"
              ? mockWorktree("brave-otter", "discarded")
              : worktreeScene === "notice"
                ? mockWorktree("brave-otter", "ready", { error: "An apply was cut off. Check the main folder’s changes before applying again." })
                : worktreeScene === "applied"
                  ? mockWorktree("brave-otter", "ready", { baseCommit: "4d2b8e01c6", appliedCommit: "4d2b8e01c6" })
                  : mockWorktree("brave-otter", "ready")
    // Another session of the folder in its own worktree, for the list.
    rows.find((r) => r.id === "ses_search")!.worktree = mockWorktree("news-ranking", "ready")
    // Being made (or failed while being made): the first message waits in the queue.
    if (worktreeScene === "preparing" || worktreeScene === "failed") {
      transcripts.set("ses_pricing", [])
      queues.set("ses_pricing", [
        { id: "qw", text: "Show the yearly price on the plan cards when the toggle is on yearly.", images: 0, delivery: "queue" },
      ])
    }
  }
  const worktreeParam = params.get("worktree")
  /** `?setup=fail`: the setup command exits 1; `hold`: it keeps running (preparing stays). */
  const setupScene = params.get("setup")
  const setupRuns = new Map<string, SetupRun>()
  /** What runs once a session's worktree is ready: its waiting message. */
  const afterReady = new Map<string, () => void>()
  /** Emits the row's worktree as the shell does on every change. */
  const worktreeChanged = (row: SessionRow) => {
    if (row.worktree) emit({ type: "worktree", directory: row.directory, sessionId: row.id, worktree: { ...row.worktree } })
  }
  /** The worktree is ready: the waiting message starts. */
  const ready = (row: SessionRow) => {
    if (row.worktree?.state !== "preparing") return
    row.worktree.state = "ready"
    worktreeChanged(row)
    const then = afterReady.get(row.id)
    afterReady.delete(row.id)
    then?.()
  }
  /** The waiting messages go into the session and its turn starts. */
  const startWaiting = (id: string) => {
    const waiting = queues.get(id) ?? []
    if (!waiting.length) return
    queues.set(id, [])
    const list = transcripts.get(id) ?? []
    transcripts.set(id, [
      ...list,
      ...waiting.map((q, i) => ({ kind: "user" as const, id: `uw${list.length + i}`, text: q.text, images: q.images, time: Date.now() })),
    ])
    running.add(id)
    emit({ type: "started", sessionId: id })
    emit({ type: "changed", sessionId: id })
    void reply(id, "Working in the new worktree.")
  }
  const SETUP_OUT: string[] = [
    "Scope: all 14 workspace projects",
    "Lockfile is up to date, resolution step is skipped",
    "Progress: resolved 1184, reused 1184, downloaded 0, added 1184, done",
    "",
    "dependencies:",
    "+ @anysola/ui 0.14.2 <- packages/ui",
    "+ react 19.2.0",
  ]
  /** The shell's preparation: steps and setup output, then ready (the waiting message starts). */
  const prepare = (row: SessionRow, then?: () => void) => {
    if (then) afterReady.set(row.id, then)
    const command = prefs.worktreeSetup[row.directory] ?? (setupScene ? "pnpm install" : null)
    const run: SetupRun = { command, state: "running", code: null, lines: [], started: Date.now(), ended: null }
    setupRuns.set(row.id, run)
    const live = () => setupRuns.get(row.id) === run && run.state === "running"
    const line = (stream: "info" | "out" | "err", text: string) => {
      if (!live()) return
      run.lines.push({ stream, text })
      emit({ type: "setup", directory: row.directory, sessionId: row.id, stream, text })
    }
    type Step = [number, "info" | "out" | "err", string]
    const steps: Step[] = [
      [100, "info", `Checking out ${row.worktree!.branch} from main`],
      [500, "info", "Copied 2 env files: .env, apps/web/.env.local"],
      ...(command
        ? ([
            [800, "info", `Running ${command}`],
            ...SETUP_OUT.map((t, i) => [900 + i * 100, "out", t] as Step),
            [1650, "err", " WARN  deprecated eslint@8.57.1: This version is no longer supported."],
            ...(setupScene === "fail"
              ? ([[1800, "err", " ERR_PNPM_FETCH_404  GET https://registry.npmjs.org/@anysola%2Fcharts: Not Found - 404"]] as Step[])
              : []),
          ] as Step[])
        : []),
    ]
    for (const [at, stream, text] of steps) setTimeout(() => line(stream, text), at)
    if (setupScene === "hold") return
    setTimeout(() => {
      if (!live()) return
      const failed = setupScene === "fail" && !!command
      Object.assign(run, { state: failed ? "failed" : "done", code: command ? (failed ? 1 : 0) : null, ended: Date.now() })
      emit({ type: "setupEnd", directory: row.directory, sessionId: row.id, run: structuredClone(run) })
      ready(row)
    }, 2000)
  }
  const worktreeRow = (sessionId: string) => {
    const row = rows.find((r) => r.id === sessionId)
    if (!row?.worktree) throw new Error("This session works in the main folder.")
    return row
  }
  /** Sessions applied with their main-folder edits set aside (the dirty check passes after). */
  const stashed = new Set<string>()
  const applyCheck = async (sessionId: string): Promise<ApplyCheck> => {
    const row = worktreeRow(sessionId)
    await wait(300)
    if (running.has(sessionId)) return { ok: false, reason: "busy" }
    if (worktreeParam === "old-git") return { ok: false, reason: "git", message: "Update Git to 2.40 or later to apply." }
    if (worktreeScene === "conflict") return { ok: false, reason: "unresolved", files: ["apps/web/src/lib/plans.ts"] }
    if (review.has("apply-conflict"))
      return { ok: false, reason: "conflict", onto: row.worktree!.baseBranch ?? "main", files: ["apps/web/src/lib/plans.ts", "apps/web/src/routes/pricing.tsx"] }
    if (review.has("apply-dirty") && !stashed.has(sessionId))
      return { ok: false, reason: "dirty", onto: row.worktree!.baseBranch ?? "main", files: ["apps/web/src/components/plan-card.tsx"] }
    if (review.has("apply-staged")) return { ok: false, reason: "staged", onto: "main" }
    if (review.has("apply-detached")) return { ok: false, reason: "detached" }
    if (review.has("apply-operation")) return { ok: false, reason: "operation", what: "rebase" }
    if (review.has("apply-empty")) return { ok: false, reason: "empty" }
    if (review.has("apply-busy")) return { ok: false, reason: "busy" }
    if (review.has("apply-git")) return { ok: false, reason: "git", message: "git merge-tree failed: fatal: unable to read tree 9c1e2a7f3b" }
    return { ok: true, onto: review.has("apply-onto") ? "release/1.4" : (row.worktree!.baseBranch ?? "main"), files: 3, add: 42, del: 9, behind: 2 }
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
  // What the folder's hooks are: `?hooks=` picks the scene, the calls below change it.
  // The scenes' worktree runs: one being made (its message waits), or a finished setup run.
  if (worktreeScene === "preparing") prepare(rows.find((r) => r.id === "ses_pricing")!, () => startWaiting("ses_pricing"))
  else if (worktreeScene && setupScene === "fail")
    setupRuns.set("ses_pricing", {
      command: "pnpm install",
      state: "failed",
      code: 1,
      lines: [
        { stream: "info", text: "Checking out anysola/brave-otter from main" },
        { stream: "info", text: "Copied 2 env files: .env, apps/web/.env.local" },
        { stream: "info", text: "Running pnpm install" },
        ...SETUP_OUT.map((text) => ({ stream: "out" as const, text })),
        { stream: "err", text: " ERR_PNPM_FETCH_404  GET https://registry.npmjs.org/@anysola%2Fcharts: Not Found - 404" },
      ],
      started: now - 95_000,
      ended: now - 61_000,
    })
  const trustScene = empty ? null : params.get("trust")
  /** `five`: the `all` file, with rules and downloads waiting too. */
  const allScene = trustScene === "all" || trustScene === "five"
  const hookScene = empty ? "none" : (params.get("hooks") ?? (allScene ? "waiting" : "user"))
  let userHooks: MockHook[] = hookScene === "none" ? [] : hookScene === "all" ? [...userHooks0, envHook] : [...userHooks0]
  let hookFiles = (
    {
      waiting: [projectFile(null)],
      allowed: [projectFile("allowed")],
      refused: [projectFile("denied")],
      off: [projectFile("allowed")],
      all: [projectFile(null), parentFile("allowed")],
      changed: [{ ...projectFile(null), standing: true }],
      emptied: [{ ...projectFile(null), digest: "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855", hooks: [], standing: true }],
    } as Record<string, ReturnType<typeof projectFile>[]>
  )[hookScene] ?? []
  const managed = hookScene === "managed" || hookScene === "all" ? managedHooks : []
  let hooksOff = hookScene === "off" || hookScene === "locked"
  let hookIds = 10
  const hooksNow = (): Hooks => {
    const active = (h: MockHook): Hook => ({
      ...h,
      active: h.scope === "managed" || (!hooksOff && (h.scope === "user" || hookFiles.some((f) => f.path === h.source && f.decision === "allowed"))),
    })
    const files = hookFiles.map((f) => ({ ...f, hooks: f.hooks.map(active) }))
    return {
      hooks: [...managed, ...userHooks, ...hookFiles.flatMap((f) => f.hooks)].map(active),
      files,
      disabled: hooksOff,
    }
  }
  // A project's MCP servers and formatters, each kind answered on its own (`?trust=`).
  const PROJECT = `${WEB}/anysola.json`
  const DOT = `${WEB}/.anysola/anysola.json`
  const mcpFile = allScene ? DOT : PROJECT
  type Section = { path: string; kind: TrustKind; digest: string; decision: TrustItem["decision"] }
  const sections: Section[] = empty
    ? []
    : [
        {
          path: mcpFile,
          kind: "mcp",
          digest: "4a1c9e2b7d3f5a8c0e6b2d4f1a3c5e7b9d0f2a4c6e8b1d3f5a7c9e0b2d4f6a8c",
          decision: allScene || trustScene === "mcp" || trustScene === "replaces" ? null : trustScene === "refused" ? "denied" : "allowed",
        },
        ...(allScene || trustScene === "formatter"
          ? [
              {
                path: allScene ? DOT : PROJECT,
                kind: "formatter" as const,
                digest: "7e3b1d5f9a2c4e6b8d0f1a3c5e7b9d2f4a6c8e0b1d3f5a7c9e2b4d6f8a0c1e3b",
                decision: null,
              },
            ]
          : []),
      ]
  // Rules that let the agent do more, and what a file makes the app download.
  const AGENT_FILE = `${WEB}/.anysola/agent/build.md`
  const rulesScene = ["five", "permissions", "permissions-refused", "agentfile", "wildcard"].includes(trustScene ?? "")
  const remoteScene = ["five", "remote", "remote-refused"].includes(trustScene ?? "")
  if (!empty && rulesScene)
    sections.push({
      path: trustScene === "five" ? DOT : PROJECT,
      kind: "permissions",
      digest: "2c4e6a8b0d1f3a5c7e9b2d4f6a8c0e1b3d5f7a9c2e4b6d8f0a1c3e5b7d9f2a4c",
      decision: trustScene === "permissions-refused" ? "denied" : null,
    })
  if (!empty && trustScene === "agentfile")
    sections.push({
      path: AGENT_FILE,
      kind: "permissions",
      digest: "5d7f9b1c3e5a7c9e1b3d5f7a9c1e3b5d7f9a1c3e5b7d9f1a3c5e7b9d1f3a5c7e",
      decision: null,
    })
  if (!empty && remoteScene)
    sections.push({
      path: trustScene === "five" ? DOT : PROJECT,
      kind: "remote",
      digest: "8f0a2c4e6b8d0f2a4c6e8b0d2f4a6c8e0b2d4f6a8c0e2b4d6f8a0c2e4b6d8f0a",
      decision: trustScene === "remote-refused" ? "denied" : null,
    })
  if (!empty && (allScene || trustScene === "mcp")) {
    // A project with more than one server: one answer covers them all.
    servers = servers.filter((x) => x.name !== "playwright")
    const local = { kind: "local" as const, error: null, editable: false, oauth: false, signedIn: false, prompts: 0 }
    servers.push(
      { ...local, name: "playwright", target: "npx @playwright/mcp@0.0.41 --headless", status: "connected", source: mcpFile, tools: 21 },
      {
        ...local,
        name: "storybook",
        target: 'node ./tools/mcp/storybook-server.mjs --port 6006 --stories "apps/web/src/**/*.stories.tsx" --config .storybook --static-dir ./public',
        status: "connected",
        source: mcpFile,
        tools: 6,
      },
      { ...local, kind: "remote", name: "sentry-dev", target: "https://sentry.dev.anysola.internal/mcp", status: "connected", source: mcpFile, tools: 9 }
    )
  } else if (!empty) servers = servers.map((x) => (x.name === "playwright" ? { ...x, source: mcpFile } : x))
  if (!empty && trustScene === "replaces")
    // The project's own github, in place of the user's (which stays its own row).
    servers.push({
      name: "github",
      kind: "local",
      target: "npx -y @acme/github-mcp-proxy --upstream https://api.githubcopilot.com/mcp/ --log-file ./.cache/github-mcp.log",
      status: "connected",
      error: null,
      source: mcpFile,
      editable: false,
      oauth: false,
      signedIn: false,
      tools: 38,
      prompts: 0,
      replaces: true,
    })
  /** Details the core writes for the `replaces` scene: long values, `=` and `:` in them. */
  const mcpDetails: Record<string, string> =
    trustScene === "replaces"
      ? {
          github: "Replaces your server of this name; env: GITHUB_TOKEN=ghp_7Hq2…c2aF, GITHUB_ORG=anysola; in ./tools/mcp",
          playwright:
            "headers: Authorization=Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9…Q5w, X-Team=web-platform-and-design-systems-guild; oauth: clientId=anysola-desktop-dev, scope=read:org repo",
        }
      : { "sentry-dev": "headers: Authorization" }
  const mcpSection = () => sections.find((x) => x.kind === "mcp")
  /** The servers as the core shows them: a project's wait while its file's servers aren't allowed. */
  const serversNow = () =>
    servers.map((x) =>
      x.source === mcpFile && mcpSection() && mcpSection()!.decision !== "allowed" ? { ...x, status: "waiting" as const, tools: null } : x
    )
  const trustNow = (): TrustItem[] => [
    ...hookFiles.map((f) => ({
      path: f.path,
      kind: "hooks" as const,
      digest: f.digest,
      decision: f.decision,
      standing: !!f.standing,
      entries: f.hooks.map((h) => ({
        label: h.matcher && h.matcher !== "*" ? `${h.event} (${h.matcher})` : h.event,
        command: h.command,
        detail: h.timeout !== (h.event === "UserPromptSubmit" ? 30 : 600) ? `timeout ${h.timeout} s` : null,
      })),
    })),
    ...sections.map((x) => ({
      ...x,
      standing: false,
      entries:
        x.kind === "mcp"
          ? servers
              .filter((v) => v.source === x.path)
              .map((v): TrustItem["entries"][number] => ({ label: v.name, command: v.target, detail: mcpDetails[v.name] ?? null }))
              .concat(
                trustScene === "replaces"
                  ? [{ label: "MCP timeouts", command: '{"startup":5,"request":3600}', detail: null }]
                  : []
              )
          : x.kind === "permissions"
            ? x.path === AGENT_FILE
              ? [
                  { label: "agent build", command: "allow edit: *", detail: AGENT_FILE },
                  { label: "agent build", command: "allow shell: *", detail: AGENT_FILE },
                ]
              : trustScene === "wildcard"
                ? [
                    // Actions that are patterns: every tool whose name matches.
                    { label: "all agents", command: "allow *: *", detail: null },
                    { label: "all agents", command: "allow she*: *", detail: null },
                    { label: "agent build", command: "allow shel?: git *", detail: null },
                    { label: "agent build", command: "allow *: ~/code/**", detail: null },
                    { label: "policy", command: "allow [a-z]*.use: *", detail: null },
                    // Text that tries to pass as part of the sentence, as the core writes it out.
                    { label: "all agents", command: "allow read: notes.txt without asking\\n allow shell: rm -rf ~", detail: null },
                  ]
                : [
                  { label: "all agents", command: "allow shell: npm test*", detail: null },
                  { label: "all agents", command: "allow webfetch: https://developer.mozilla.org/*", detail: null },
                  { label: "agent build", command: "allow edit: apps/web/src/**", detail: null },
                  { label: "agent plan", command: "allow external_directory: ~/code/shared-config/**", detail: null },
                  { label: "policy", command: "deny mcp.use: github", detail: null },
                  { label: "sessions", command: "start with the agent mine", detail: null },
                  {
                    label: "all agents",
                    command:
                      "allow edit: apps/web/src/generated/very/long/path/that/keeps/going/without/a/break/anywhere/in/it/so/it/must/wrap/inside/the/card/**",
                    detail: null,
                  },
                ]
          : x.kind === "remote"
            ? [
                { label: "instructions", command: "https://raw.githubusercontent.com/anysola/handbook/main/agents/web-team-rules.md", detail: null },
                { label: "design-tokens", command: "https://github.com/anysola/design-tokens (branch main)", detail: null },
                { label: "skills", command: "https://skills.anysola.example/teams/web/skills.zip", detail: null },
              ]
          : trustScene === "formatter"
            ? [{ label: "formatter", command: "the built-in formatters", detail: null }]
            : [
                { label: "prettier", command: "npx prettier --write $FILE", detail: "for .ts, .tsx, .css" },
                { label: "gofmt", command: "the built-in gofmt formatter", detail: null },
              ],
    })),
  ]
  const trustCalls: unknown[] = []
  ;(window as unknown as { __trustCalls: unknown[] }).__trustCalls = trustCalls
  /** The rules as the core lists them: a project's allow rules apply once its file's rules are allowed. */
  const rulesNow = (): PermissionRule[] => {
    const project: PermissionRule[] = []
    for (const item of trustNow())
      if (item.kind === "permissions")
        for (const e of item.entries) {
          const m = /^(allow|ask|deny) ([^:]+): (.*)$/s.exec(e.command)
          const agent = /^agent (.+)$/.exec(e.label)?.[1] ?? null
          if (!m || (e.label !== "all agents" && !agent)) continue
          const effect = m[1] as PermissionRule["effect"]
          project.push({
            ...rule(m[2], m[3], effect, item.path, agent),
            applied: effect !== "allow" || item.decision === "allowed",
          })
        }
    return [...rules, ...project]
  }
  /** Downloads read on open: a project's once its file's downloads are allowed (`?trust=remote`). */
  let retried = false
  let repoReads = 0
  const downloadsNow = (all: Sources): Sources => {
    const remote = sections.find((x) => x.kind === "remote" && x.decision === "allowed")
    if (!remote) return all
    // The repository is still downloading for the first reads after the answer.
    const cloned = ++repoReads > 2
    return {
      ...all,
      instructions: [
        ...all.instructions,
        {
          path: "https://raw.githubusercontent.com/anysola/handbook/main/agents/web-team-rules.md",
          bytes: retried ? 3_120 : 0,
          origin: "url",
          error: retried ? null : "404 Not Found",
        },
      ],
      skillUrls: [{ url: "https://skills.anysola.example/teams/web/skills.zip", error: "index.json: 404 Not Found" }],
      repositories: [
        {
          name: "design-tokens",
          repository: "https://github.com/anysola/design-tokens",
          branch: "main",
          cloned,
          attemptedAt: cloned ? Date.now() : null,
          error: null,
        },
      ],
    }
  }
  const hookOf = (draft: HookDraft, id: string): MockHook => ({
    id,
    event: draft.event,
    matcher: draft.matcher || null,
    command: draft.command,
    timeout: draft.timeout ?? (draft.event === "UserPromptSubmit" ? 30 : 600),
    source: CONFIG,
    scope: "user",
  })
  let personal = "Reply in short, plain sentences.\nUse pnpm, never npm, in the web repos.\nRun the tests that cover a change before saying it is done.\n"
  const view = (sessionId: string): View => {
    const items = transcripts.get(sessionId) ?? []
    const extra = {
      queued: queues.get(sessionId) ?? [],
      context:
        sessionId !== "ses_pricing" ? null : { tokens: state === "context" ? 171_400 : 61_800, limit: 200_000 },
      background: background.filter((b) => b.sessionId === sessionId || sessionId === "ses_pricing"),
    }
    if (revert && sessionId === "ses_pricing")
      return { items: [], revert, todos: [], agent: null, ...extra }
    // A stopped turn can leave a tool row that never heard its end.
    const done = !running.has(sessionId) && state !== "queue-stopped"
    return {
      items: state === "question" ? items.slice(0, -1) : done ? items.filter((i) => i.kind !== "tool" || i.status !== "running") : items,
      revert: null,
      todos: (state === "todos" || state === "background") && sessionId === "ses_pricing" ? todos : [],
      agent: state === "plan" ? "plan" : "build",
      ...extra,
    }
  }
  /** A turn's reply, streamed word by word, then its end. */
  const reply = async (id: string, text: string, end = true) => {
    await wait(400)
    for (const w of text.split(" ")) {
      emit({ type: "text", sessionId: id, messageId: "live", ordinal: 0, text: `${w} ` })
      await wait(60)
    }
    if (!end || !running.has(id)) return
    const list = transcripts.get(id) ?? []
    transcripts.set(id, [...list, { kind: "text", id: `r${list.length}`, text }])
    spend(id)
    await wait(300)
    // The queue runs next, as the core does it.
    const waiting = queues.get(id) ?? []
    if (waiting.length) {
      queues.set(id, [])
      const after = transcripts.get(id) ?? []
      transcripts.set(id, [
        ...after,
        ...waiting.map((q, i) => ({ kind: "user" as const, id: `uq${after.length + i}`, text: q.text, images: q.images, time: Date.now() })),
      ])
      emit({ type: "changed", sessionId: id })
      void reply(id, "On it: the team card reads its price the same way.")
      return
    }
    running.delete(id)
    emit({ type: "done", sessionId: id })
  }
  const wait = (ms: number) => new Promise((r) => setTimeout(r, ms))
  /** Budgets set in this run; a session without one has none. `?budget=10` gives ses_pricing
   * one, `?spent=5` what it used (6.4 by default). Each mock turn takes 1.2. */
  const budgets = new Map<string, number | null>(params.get("budget") ? [["ses_pricing", Number(params.get("budget"))]] : [])
  const spent = new Map<string, { turn: number; total: number; turns: number }>([
    ["ses_pricing", { turn: 1.2, total: Number(params.get("spent")) || 6.4, turns: 5 }],
  ])
  const usageOf = (sessionId: string): SessionUsage => {
    const used = spent.get(sessionId) ?? { turn: 0, total: 0, turns: 0 }
    return { sessionId, ...used, budget: budgets.get(sessionId) ?? null, fiveHour: info.fiveHour ?? null, weekly: null }
  }
  /** As the core: a turn takes its share, and the window hears of it. */
  const spend = (sessionId: string) => {
    const was = spent.get(sessionId) ?? { turn: 0, total: 0, turns: 0 }
    spent.set(sessionId, { turn: 1.2, total: Math.round((was.total + 1.2) * 10) / 10, turns: was.turns + 1 })
    emit({ type: "usage", ...usageOf(sessionId) })
  }
  /** Sessions whose stop at the current budget is noted already: later stops add no note. */
  const noted = new Set<string>()
  /** As the core: a turn (a prompt, a wake, `/compact`) starts and ends before any model call,
   * noting the first stop at a budget only. */
  const budgetStop = async (id: string) => {
    emit({ type: "started", sessionId: id })
    await wait(150)
    if (!noted.has(id)) {
      noted.add(id)
      const after = transcripts.get(id) ?? []
      transcripts.set(id, [...after, { kind: "note", id: `n${after.length}`, text: "Stopped at the session’s budget.", failed: false }])
    }
    emit({ type: "interrupted", sessionId: id, reason: "budget" })
  }
  const overBudget = (sessionId: string) => {
    const u = usageOf(sessionId)
    return u.budget !== null && u.total >= u.budget
  }
  /**
   * `?state=live`: a turn whose tool calls stream their input, in the app's order: the row is
   * stored as the call starts (`changed`), snapshots of its input follow, and `changed` again
   * once the call is whole and runs.
   */
  const liveTurn = async (id: string) => {
    const add = (item: Item) => {
      transcripts.set(id, [...(transcripts.get(id) ?? []), item])
      emit({ type: "changed", sessionId: id })
    }
    const update = (callId: string, patch: Partial<Extract<Item, { kind: "tool" }>>) => {
      transcripts.set(
        id,
        (transcripts.get(id) ?? []).map((i) => (i.id === callId && i.kind === "tool" ? { ...i, ...patch } : i))
      )
      emit({ type: "changed", sessionId: id })
    }
    const tool = (callId: string, name: string): Item => ({
      kind: "tool", id: callId, name, status: "streaming", target: "", files: null, output: "", error: null,
    })
    const input = (callId: string, name: string, target: string, body?: string) =>
      emit({
        type: "toolInput", sessionId: id, messageId: "live", callId, name, target,
        lines: body === undefined ? null : body.split("\n").length - (body.endsWith("\n") ? 1 : 0),
        tail: body === undefined ? null : body.split("\n").slice(-6).join("\n"),
      })
    await wait(400)
    add({ kind: "text", id: `lt${Date.now()}`, text: "I’ll add the yearly toggle, then run its test." })
    const file = "apps/web/src/components/cycle-toggle.tsx"
    const lines = [
      `import { cn } from "@/lib/utils"`, ``, `type Cycle = "monthly" | "yearly"`, ``,
      `type Props = { value: Cycle; onChange: (cycle: Cycle) => void }`, ``,
      `export function CycleToggle({ value, onChange }: Props) {`, `  return (`,
      `    <div role="radiogroup" aria-label="Billing cycle" className="inline-flex rounded-lg bg-muted p-0.5">`,
      `      {(["monthly", "yearly"] as const).map((cycle) => (`, `        <button`, `          key={cycle}`,
      `          role="radio"`, `          aria-checked={value === cycle}`, `          onClick={() => onChange(cycle)}`,
      `          className={cn("rounded-md px-3 py-1 text-sm", value === cycle && "bg-background shadow-sm")}`,
      `        >`, `          {cycle === "monthly" ? "Monthly" : "Yearly · 2 months free"}`, `        </button>`,
      `      ))}`, `    </div>`, `  )`, `}`,
    ]
    const w = "call_live_write"
    add(tool(w, "write"))
    const path = `/home/alex/work/anysola-web/${file}`
    for (let n = 1; n <= path.length; n += 9) {
      input(w, "write", "")
      await wait(50)
    }
    let body = ""
    for (const [n, line] of lines.entries()) {
      for (let at = 0; at < line.length + 1; at += 14) {
        body = lines.slice(0, n).map((l) => l + "\n").join("") + (line + "\n").slice(0, at + 14)
        input(w, "write", file, body)
        await wait(50)
        if (!running.has(id)) return
      }
    }
    const patch = lines.map((l) => `+${l}`).join("\n")
    update(w, {
      status: "completed",
      target: file,
      files: [{ file, status: "added", additions: lines.length, deletions: 0, patch: `Index: ${file}\n===\n--- /dev/null\n+++ ${file}\n@@ -0,0 +1,${lines.length} @@\n${patch}\n` }],
    })
    await wait(300)
    const sh = "call_live_shell"
    add(tool(sh, "shell"))
    const command = "pnpm --filter @anysola/web test cycle-toggle"
    for (let n = 2; n <= command.length; n += 2) {
      input(sh, "shell", command.slice(0, n))
      await wait(50)
      if (!running.has(id)) return
    }
    update(sh, { status: "running", target: command })
    // Its output as the shell reads it from the command's file, a few times a second.
    const out = [
      "> @anysola/web@0.0.0 test /home/alex/work/anysola-web/apps/web",
      "> vitest run cycle-toggle",
      "",
      " RUN  v3.2.4 /home/alex/work/anysola-web/apps/web",
      "",
      " ✓ cycle-toggle.test.tsx > renders both cycles",
      " ✓ cycle-toggle.test.tsx > marks the chosen cycle",
      " ✓ cycle-toggle.test.tsx > calls onChange",
      "",
      " Test Files  1 passed (1)",
      "      Tests  3 passed (3)",
    ]
    for (let n = 1; n <= out.length; n++) {
      emit({ type: "toolOutput", sessionId: id, callId: sh, lines: n, tail: out.slice(0, n).slice(-6).join("\n") })
      await wait(250)
      if (!running.has(id)) return
    }
    update(sh, { status: "completed", output: out.join("\n") })
    void reply(id, "The toggle is in and its test passes.")
  }
  let cancelLogin = () => {}

  return {
    async authStatus() {
      if (state === "fault")
        return {
          signedIn: true,
          fault: {
            message: "The database is locked by another program. Close other copies of Anysola Code, then open it again.",
            folder: "/home/alex/.anysola",
          },
        }
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
      // A new list each read, as the core sends it.
      return state === "empty" ? [] : [...rows]
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
        if (state === "approval" || state === "gone")
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
        if (state === "running") emit({ type: "started", sessionId: session.id })
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
            kind,
            error:
              kind === "limit"
                ? "Your 5-hour limit is reached."
                : kind === "offline"
                  ? "Couldn’t reach api.anysola.com."
                  : kind === "auth"
                    ? "Your sign-in is no longer valid."
                    : kind === "model"
                      ? "Opus 5.5 isn’t available on your plan right now."
                      : kind === "elsewhere"
                        ? "This session is running in another Anysola Code app."
                        : "Anysola Code is temporarily unavailable. Try again shortly.",
          })
        if (state === "hooks")
          setTimeout(
            () =>
              emit({
                type: "hook",
                sessionId: session.id,
                event: "SessionStart",
                command: "git log --oneline -5",
                outcome: "error",
                message: "The hook exited with code 128: fatal: not a git repository (or any of the parent directories): .git",
              }),
            500
          )
        if (state === "queue-stopped") emit({ type: "interrupted", sessionId: session.id, reason: "user" })
        if (state === "closing") setTimeout(() => emit({ type: "closeRequested", busy: 2 }), 400)
        if (state === "running" || state === "background" || state === "context") {
          void reply(session.id, "The tests pass. I’ll also check the yearly prices on the team plan, which reads", false)
        }
      }
      const pending: PendingRequest[] =
        state === "restored" && session.id === "ses_pricing"
          ? [
              {
                type: "permission",
                sessionId: session.id,
                directory: session.directory,
                requestId: "per_restored",
                action: "shell",
                resources: ["pnpm --filter @anysola/web test pricing"],
                canAlways: true,
                files: null,
              },
              ...forms.filter((f) => f.directory === session.directory),
            ]
          : forms.filter((f) => f.directory === session.directory)
      return {
        session,
        view: view(session.id),
        info: state === "plan" ? { ...info, agent: "plan" } : info,
        busy: running.has(session.id),
        pending,
      }
    },
    async transcript(_directory, sessionId) {
      return view(sessionId)
    },
    async revert(directory, sessionId, messageId) {
      const items = transcripts.get(sessionId) ?? []
      const text = items.find((i) => i.id === messageId)
      revert = state === "long" ? longRevert : rewound
      revertAt = messageId
      // Only the web folder is a git repository here (see gitStatus).
      return { text: text?.kind === "user" ? text.text : "", view: view(sessionId), filesKept: directory !== WEB }
    },
    async unrevert(_directory, sessionId) {
      revert = null
      revertAt = null
      return view(sessionId)
    },
    async resend(_directory, sessionId) {
      revert = null
      revertAt = null
      running.add(sessionId)
      emit({ type: "started", sessionId })
      void reply(sessionId, "Trying that again from the top.")
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
      running.add(id)
      emit({ type: "started", sessionId: id })
      void reply(id, "Reviewing the uncommitted changes.")
      return { sessionId: id }
    },
    async compact(_directory, sessionId) {
      if (overBudget(sessionId)) await budgetStop(sessionId)
    },
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
      if (failing.has("changes")) throw new Error("git diff failed: index.lock exists.")
      if (sessionId !== "ses_pricing") return []
      if (review.has("nogit")) return null
      if (review.has("long")) return longReview
      return state === "long" ? reviewFiles : [planCard, pricing, test]
    },
    async restoreFile() {},
    async worktreeInfo(directory) {
      if (directory !== WEB) return { available: false, reason: "not-git" }
      if (worktreeParam === "no-commits") return { available: false, reason: "no-commits" }
      return {
        available: true,
        branch: "main",
        uncommitted: 3,
        setup: prefs.worktreeSetup[directory] ?? null,
        canApply: worktreeParam !== "old-git",
      }
    },
    worktreeApplyCheck: (_directory, sessionId) => applyCheck(sessionId),
    async worktreeApply(_directory, sessionId, _message, onto, stash) {
      const row = worktreeRow(sessionId)
      const check = await applyCheck(sessionId)
      if (check.ok ? check.onto !== onto : !(check.reason === "dirty" && stash))
        throw new Error("The main folder changed since the check. Check again.")
      row.worktree!.state = "applying"
      worktreeChanged(row)
      await wait(600)
      const commit = Math.random().toString(16).slice(2, 12)
      Object.assign(row.worktree!, { state: "ready", baseCommit: commit, appliedCommit: commit })
      worktreeChanged(row)
      if (stash) stashed.add(sessionId)
      return { commit, stashLeft: stash && review.has("apply-stash-left") ? "stash@{0}" : null }
    },
    async worktreeDiscard(_directory, sessionId) {
      const row = worktreeRow(sessionId)
      if (running.has(sessionId)) throw new Error("This session is working. Stop it first.")
      row.worktree!.state = "discarded"
      worktreeChanged(row)
    },
    async worktreeBringBase(_directory, sessionId) {
      const row = worktreeRow(sessionId)
      await wait(400)
      const conflicts = review.has("apply-conflict") ? ["apps/web/src/lib/plans.ts"] : []
      if (!conflicts.length) row.worktree!.baseCommit = "b71f0c9d24"
      worktreeChanged(row)
      return { conflicts }
    },
    async worktreeRecreate(_directory, sessionId) {
      const row = worktreeRow(sessionId)
      if (row.worktree!.state !== "missing" && row.worktree!.state !== "failed") throw new Error("This session’s worktree is there.")
      Object.assign(row.worktree!, { state: "preparing", error: null })
      worktreeChanged(row)
      prepare(row)
      return { ...row.worktree! }
    },
    async worktreeToFolder(_directory, sessionId) {
      const row = worktreeRow(sessionId)
      if (row.worktree!.state !== "preparing" && row.worktree!.state !== "failed")
        throw new Error("This session already works in its worktree; apply or discard it instead.")
      const run = setupRuns.get(sessionId)
      if (run?.state === "running") Object.assign(run, { state: "stopped", ended: Date.now() })
      row.worktree = null
      afterReady.delete(sessionId)
      // The waiting message runs in the main folder.
      setTimeout(() => startWaiting(sessionId), 300)
      return structuredClone(row)
    },
    async setupOutput(_directory, sessionId) {
      const run = setupRuns.get(sessionId)
      return run ? structuredClone(run) : null
    },
    async setupRun(_directory, sessionId) {
      const row = worktreeRow(sessionId)
      if (setupRuns.get(sessionId)?.state === "running") throw new Error("The setup command is already running.")
      prepare(row)
    },
    async setupStop(_directory, sessionId) {
      const run = setupRuns.get(sessionId)
      if (!run || run.state !== "running") return false
      const row = worktreeRow(sessionId)
      Object.assign(run, { state: "stopped", ended: Date.now() })
      emit({ type: "setupEnd", directory: row.directory, sessionId, run: structuredClone(run) })
      ready(row)
      return true
    },
    async gitStatus(directory) {
      // `?git=missing|conflict|noidentity|behind`: the git menu's other states.
      const git = params.get("git")
      if (failing.has("gitStatus")) throw new Error("git status failed: not a valid object name HEAD.")
      if (git === "missing") return { repo: false as const, git: "missing" as const }
      const own = rows.find((r) => r.worktree?.path === directory)?.worktree
      if (own)
        return {
          repo: true,
          root: own.path,
          branch: own.branch,
          detached: false,
          upstream: null,
          ahead: 0,
          behind: 0,
          files: [
            { file: planCard.file, status: "modified" },
            { file: pricing.file, status: "modified" },
            { file: test.file, status: "added" },
          ],
          unmerged: worktreeScene === "conflict" ? ["apps/web/src/lib/plans.ts"] : [],
          identity: true,
          remote: "origin",
          gh: true,
        }
      if (directory !== WEB) return { repo: false as const }
      return {
        repo: true,
        branch: "fix/yearly-price",
        detached: false,
        upstream: state === "pushed" || git === "behind" ? "origin/fix/yearly-price" : null,
        ahead: state === "pushed" || git === "behind" ? 0 : 1,
        behind: git === "behind" ? 2 : 0,
        files: [
          { file: planCard.file, status: "modified" },
          { file: pricing.file, status: "modified" },
          { file: test.file, status: "added" },
          { file: "apps/web/dist/build-stats.json", status: "added" },
        ],
        unmerged: git === "conflict" ? ["apps/web/src/lib/plans.ts", "apps/web/src/routes/checkout.tsx"] : [],
        identity: git !== "noidentity",
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
      if (state === "offline") throw new Error("Couldn’t reach api.anysola.com.")
      return accountFor(state, bank)
    },
    async updateCheck() {
      return state === "update" || state === "updating"
        ? {
            available: true,
            version: "1.0.8",
            notes: "A welcome screen and starter prompts for new sessions.\nKeyboard shortcuts: Ctrl / lists them.\nThe git menu commits the files you pick.",
          }
        : { available: false }
    },
    async updateInstall() {
      const total = 38_400_000
      for (let got = 0; got <= total; got += 2_400_000) {
        emit({ type: "update", downloaded: got, total })
        await wait(250)
      }
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
    async prompt(directory, { sessionId, text, files, place }) {
      if (!sessionId && params.get("slow")) await wait(Number(params.get("slow")))
      const id = sessionId ?? `ses_${Math.random().toString(36).slice(2, 10)}`
      if (!sessionId) {
        // As the shell: a new session in a git folder gets its own worktree unless asked not to;
        // the message waits until it is ready.
        const isolate = directory === WEB && place !== "folder" && worktreeParam !== "no-commits"
        const row: SessionRow = {
          id,
          title: "New session",
          directory,
          updated: Date.now(),
          forkOf: null,
          worktree: isolate ? mockWorktree(`calm-${id.slice(4, 9)}`, "preparing") : null,
        }
        rows.unshift(row)
        emit({ type: "created", sessionId: id, title: "New session", directory })
        if (isolate) {
          queues.set(id, [{ id: `q${Date.now()}`, text, images: files.length, delivery: "queue" }])
          worktreeChanged(row)
          prepare(row, () => startWaiting(id))
          return { sessionId: id, queued: true }
        }
      }
      const own = rows.find((r) => r.id === id)?.worktree
      if (own?.state === "preparing") {
        queues.set(id, [...(queues.get(id) ?? []), { id: `q${Date.now()}`, text, images: files.length, delivery: "queue" }])
        emit({ type: "changed", sessionId: id })
        return { sessionId: id, queued: true }
      }
      if (own && own.state !== "ready") {
        emit({ type: "refused", sessionId: id, error: "This session’s folder is gone. Recreate it, discard it, or fork the session." })
        return { sessionId: id, queued: false }
      }
      if (state === "refused") {
        // The core answers after the spawned turn has already said no.
        emit({ type: "refused", sessionId: id, error: "This model doesn’t read images." })
        await wait(50)
        return { sessionId: id, queued: false }
      }
      if (state === "stuck") return { sessionId: id, queued: false }
      const blockReasons = () =>
        [
          "This message has what looks like an API key (sk-…). Remove it and send again.",
          "Messages in this project stay under 2,000 characters.",
        ].slice(0, params.get("blocks") === "2" ? 2 : 1)
      if (state === "hook-blocked-queued" && running.has(id)) {
        // Queued behind the running turn; when the turn takes it, the hook says no, the
        // message is dropped from the queue and the turn ends.
        const q: Queued = { id: `q${Date.now()}`, text, images: files.length, delivery: "queue" }
        queues.set(id, [...(queues.get(id) ?? []), q])
        emit({ type: "changed", sessionId: id })
        setTimeout(() => {
          queues.set(id, (queues.get(id) ?? []).filter((x) => x.id !== q.id))
          running.delete(id)
          const reasons = blockReasons()
          for (const message of reasons)
            emit({ type: "hook", sessionId: id, event: "UserPromptSubmit", command: "~/bin/redact-secrets", outcome: "blocked", message })
          emit({ type: "failed", sessionId: id, unsent: true, kind: "other", error: reasons.join("\n") })
        }, 1200)
        return { sessionId: id, queued: true }
      }
      if (state === "hook-blocked") {
        // The turn starts, then the hook runs as the message enters the session: it never
        // became part of it.
        await wait(200)
        emit({ type: "started", sessionId: id })
        await wait(50)
        setTimeout(() => {
          const reasons = blockReasons()
          for (const message of reasons)
            emit({ type: "hook", sessionId: id, event: "UserPromptSubmit", command: "~/bin/redact-secrets", outcome: "blocked", message })
          emit({ type: "failed", sessionId: id, unsent: true, kind: "other", error: reasons.join("\n") })
        }, 150)
        return { sessionId: id, queued: false }
      }
      if (state === "unsent" || (state === "offline" && !running.has(id))) {
        // The message wasn't kept, the window puts it back. The core starts the turn before it
        // finds there's no route (`unsent`), or doesn't reach that far (`offline`).
        await wait(300)
        if (state === "unsent") emit({ type: "started", sessionId: id })
        emit({
          type: "failed",
          sessionId: id,
          unsent: true,
          kind: state === "offline" ? "offline" : "other",
          error: state === "offline" ? "Couldn’t reach api.anysola.com." : "The session couldn’t start a turn: its folder was moved.",
        })
        return { sessionId: id, queued: false }
      }
      if (running.has(id)) {
        const q: Queued = { id: `q${Date.now()}`, text, images: files.length, delivery: "queue" }
        queues.set(id, [...(queues.get(id) ?? []), q])
        emit({ type: "changed", sessionId: id })
        return { sessionId: id, queued: true }
      }
      const list = transcripts.get(id) ?? []
      transcripts.set(id, [...list, { kind: "user", id: `u${list.length}`, text, images: files.length, time: Date.now() }])
      if (overBudget(id)) {
        emit({ type: "changed", sessionId: id })
        await budgetStop(id)
        return { sessionId: id, queued: false }
      }
      running.add(id)
      emit({ type: "changed", sessionId: id })
      emit({ type: "started", sessionId: id })
      if (state === "live") void liveTurn(id)
      else void reply(id, "I’ll start by reading the files this touches, then make the change.")
      return { sessionId: id, queued: false }
    },
    async setMode() {
      if (failing.has("setMode")) throw new Error("The session’s settings couldn’t be saved.")
    },
    async interrupt(_directory, sessionId) {
      if (state === "stuck" || params.has("hang")) return
      // The core takes a moment to stop the turn, then adds its note.
      await wait(700)
      running.delete(sessionId)
      const list = transcripts.get(sessionId) ?? []
      transcripts.set(sessionId, [...list, { kind: "note", id: `n${list.length}`, text: "Stopped", failed: false }])
      emit({ type: "interrupted", sessionId, reason: "user" })
    },
    async replyPermission(_directory, requestId) {
      await wait(150)
      if (failing.has("replyPermission")) throw new Error("The answer didn’t reach the session. Try again.")
      if (state === "gone") return false
      emit({ type: "permissionReplied", sessionId: "ses_pricing", requestId })
      return true
    },
    async replyQuestion(_directory, requestId) {
      await wait(150)
      if (failing.has("replyQuestion")) throw new Error("The answer didn’t reach the session. Try again.")
      if (state === "gone") return false
      emit({ type: "questionReplied", sessionId: "ses_pricing", requestId })
      return true
    },
    async replyForm(directory, formId) {
      await wait(150)
      if (failing.has("replyForm")) throw new Error("The answer didn’t reach linear. Try again.")
      if (state === "gone") return false
      forms = forms.filter((f) => f.formId !== formId)
      emit({ type: "formReplied", directory, formId })
      return true
    },
    async pending(directory, sessionId) {
      return state === "restored" && sessionId === "ses_pricing"
        ? [
            {
              type: "permission",
              sessionId,
              directory,
              requestId: "per_restored",
              action: "shell",
              resources: ["pnpm --filter @anysola/web test pricing"],
              canAlways: true,
              files: null,
            },
          ]
        : forms.filter((f) => f.directory === directory)
    },
    async queueCancel(_directory, sessionId, id) {
      if (failing.has("queueCancel")) throw new Error("That message already went to the session.")
      queues.set(sessionId, (queues.get(sessionId) ?? []).filter((q) => q.id !== id))
      return view(sessionId)
    },
    async queueSteer(_directory, sessionId, id) {
      if (failing.has("queueSteer")) throw new Error("The turn ended before it could take the message.")
      queues.set(
        sessionId,
        (queues.get(sessionId) ?? []).map((q) => (q.id === id ? { ...q, delivery: "steer" as const } : q))
      )
      // The running turn picks it up at its next step.
      setTimeout(() => {
        const q = (queues.get(sessionId) ?? []).find((x) => x.id === id)
        if (!q) return
        queues.set(sessionId, (queues.get(sessionId) ?? []).filter((x) => x.id !== id))
        const list = transcripts.get(sessionId) ?? []
        transcripts.set(sessionId, [...list, { kind: "user", id: `us${list.length}`, text: q.text, images: q.images, time: Date.now() }])
        emit({ type: "changed", sessionId })
      }, 1200)
      return view(sessionId)
    },
    async queueSend(_directory, sessionId) {
      const waiting = queues.get(sessionId) ?? []
      queues.set(sessionId, [])
      const list = transcripts.get(sessionId) ?? []
      transcripts.set(sessionId, [
        ...list,
        ...waiting.map((q, i) => ({ kind: "user" as const, id: `uq${list.length + i}`, text: q.text, images: q.images, time: Date.now() })),
      ])
      if (overBudget(sessionId)) return void (await budgetStop(sessionId))
      running.add(sessionId)
      emit({ type: "started", sessionId })
      emit({ type: "changed", sessionId })
      void reply(sessionId, "Picking up where we stopped: the team card next.")
    },
    async backgroundStop(_directory, id) {
      const at = background.findIndex((b) => b.id === id)
      if (at < 0) return false
      background.splice(at, 1)
      return true
    },
    async messageImages() {
      await wait(300)
      return [picture(30, "Pricing · 390 × 844"), picture(210, "Pricing · toggle open")]
    },
    async editors() {
      if (params.get("editors") === "none") return []
      return [
        { id: "code", name: "Visual Studio Code" },
        { id: "zed", name: "Zed" },
      ]
    },
    async openInEditor() {},
    async gitPull() {},
    async gitIdentity() {},
    async mcpSignInCancel() {},
    async quit() {
      location.reload()
    },
    async keepOpen() {},
    async renameSession(sessionId, title) {
      const row = rows.find((r) => r.id === sessionId)
      if (row) row.title = title
    },
    async sessionRow(sessionId) {
      const row = rows.find((r) => r.id === sessionId)
      return row ? { ...row } : null
    },
    async forkSession(directory, sessionId, before) {
      // As the shell (branch.rs): roots only, not while running, through what the window shows.
      const source = rows.find((r) => r.id === sessionId)
      if (!source)
        throw new Error(transcripts.has(sessionId) ? "A subagent’s session can’t be changed on its own." : "This session no longer exists.")
      if (running.has(sessionId)) throw new Error("Wait for the current turn to end, or stop it, before branching.")
      const items = transcripts.get(sessionId) ?? []
      const users = items.filter((i) => i.kind === "user").map((i) => i.id)
      const shown = revertAt === null ? users.length : users.indexOf(revertAt)
      if (shown < 0) throw new Error("Redo the rewind or send a message before branching.")
      const end = before ?? (revertAt !== null ? revertAt : null)
      if (before !== null) {
        const at = users.indexOf(before)
        if (at === 0) throw new Error("There is nothing before the first message to fork.")
        if (at < 0 || at >= shown) throw new Error("That message is no longer in the session.")
      }
      const copied = end === null ? items : items.slice(0, items.findIndex((i) => i.id === end))
      if (copied.length === 0) throw new Error("There is nothing before that message to fork.")
      // The core gives a fork its source's title as it is; only `forkOf` tells it.
      const id = `ses_${Math.random().toString(36).slice(2, 10)}`
      // In a git folder the fork gets its own worktree with the files put back (`?restored=`
      // picks the report).
      const scene = params.get("restored")
      const restored: ForkRestore =
        scene === "kept"
          ? { exact: false, kept: ["apps/web/public/hero.mp4"], fallback: null }
          : scene === "edits" || scene === "now"
            ? { exact: false, kept: [], fallback: scene }
            : { exact: true, kept: [], fallback: null }
      const row: SessionRow = {
        id,
        title: source.title,
        directory: source.directory,
        updated: Date.now(),
        forkOf: source.id,
        worktree: source.directory === WEB ? mockWorktree(`swift-${id.slice(4, 9)}`, "preparing", { restored }) : null,
      }
      rows.unshift(row)
      transcripts.set(id, structuredClone(copied))
      emit({ type: "created", sessionId: id, title: row.title, directory })
      if (row.worktree) {
        worktreeChanged(row)
        prepare(row)
      }
      return structuredClone(row)
    },
    async setBudget(_directory, sessionId, budget) {
      if (!rows.some((r) => r.id === sessionId)) throw new Error("A subagent’s session can’t be changed on its own.")
      if (budget !== null && !(Number.isInteger(budget) && budget >= 1 && budget <= MAX_BUDGET))
        throw new Error(`A budget is a whole percent, 1 to ${MAX_BUDGET}, of the 5-hour window.`)
      noted.delete(sessionId)
      const u = { ...usageOf(sessionId), budget }
      budgets.set(sessionId, budget)
      emit({ type: "usage", ...u })
      return u
    },
    async sessionUsage(_directory, sessionId) {
      return usageOf(sessionId)
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
    mcpList: () => settle("mcpList", serversNow),
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
        return serversNow()
      }),
    mcpRemove: (_directory, name) =>
      settle("mcpRemove", () => {
        servers = servers.filter((s) => s.name !== name || s.replaces)
        return serversNow()
      }),
    mcpEnable: (_directory, name, enabled) =>
      settle("mcpEnable", () => {
        servers = servers.map((s) =>
          s.name !== name || s.replaces
            ? s
            : enabled
              ? { ...s, status: s.oauth && !s.signedIn ? "needs_auth" : "connected", tools: s.oauth && !s.signedIn ? null : 17 }
              : { ...s, status: "disabled", tools: null, error: null }
        )
        return serversNow()
      }),
    mcpRetry: (_directory, name) =>
      settle("mcpRetry", () => {
        servers = servers.map((s) => (s.name === name && !s.replaces ? { ...s, status: "pending", error: null } : s))
        setTimeout(() => {
          servers = servers.map((s) =>
            s.name === name && !s.replaces ? { ...s, status: "failed", error: "MCP error -32000: Connection closed\nnpm error code E404" } : s
          )
          emit({ type: "mcp", directory: WEB })
        }, 1200)
        return serversNow()
      }),
    mcpSignIn: (_directory, name) =>
      settle("mcpSignIn", () => ({ url: `https://mcp.${name}.app/oauth/authorize?client_id=anysola`, instructions: null })),
    mcpSignInWait: async (_directory, name) => {
      await wait(4000)
      servers = servers.map((s) => (s.name === name && !s.replaces ? { ...s, status: "connected", signedIn: true, tools: 23 } : s))
      return serversNow()
    },
    mcpSignOut: (_directory, name) =>
      settle("mcpSignOut", () => {
        servers = servers.map((s) => (s.name === name && !s.replaces ? { ...s, status: "needs_auth", signedIn: false, tools: null } : s))
        return serversNow()
      }),
    sources: (directory) =>
      settle("sources", () => {
        const all = downloadsNow(sourcesFor(directory))
        if (empty) return { ...all, skills: [], commands: all.commands.filter((c) => c.origin === "builtin"), instructions: [] }
        // Off, Claude Code's folders aren't read.
        return prefs.sources.claudeSkills ? all : { ...all, skills: all.skills.filter((x) => x.origin !== "claude") }
      }),
    sourcesRetry: (directory) =>
      settle("sourcesRetry", () => {
        // The project's instructions come through on the second try; its skill index still fails.
        retried = true
        const all = downloadsNow(sourcesFor(directory))
        return prefs.sources.claudeSkills ? all : { ...all, skills: all.skills.filter((x) => x.origin !== "claude") }
      }),
    personalInstructions: () => settle("personalInstructions", () => ({ path: `${HOME}/.anysola/AGENTS.md`, text: personal })),
    savePersonalInstructions: (text) =>
      settle("savePersonalInstructions", () => {
        personal = text
      }),
    permissions: () => settle("permissions", () => ({ saved, rules: empty ? [] : rulesNow() })),
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
    hooks: () => settle("hooks", hooksNow),
    hookAdd: (_directory, draft) =>
      settle("hookAdd", () => {
        userHooks = [...userHooks, hookOf(draft, `hk_${++hookIds}`)]
        return hooksNow()
      }),
    hookUpdate: (_directory, id, draft) =>
      settle("hookUpdate", () => {
        if (!userHooks.some((h) => h.id === id)) throw new Error("That hook isn’t in your config file any more.")
        userHooks = userHooks.map((h) => (h.id === id ? hookOf(draft, id) : h))
        return hooksNow()
      }),
    hookRemove: (_directory, id) =>
      settle("hookRemove", () => {
        userHooks = userHooks.filter((h) => h.id !== id)
        return hooksNow()
      }),
    hooksTrust: (directory, path, digest, allow) =>
      settle("hooksTrust", () => {
        const file = hookFiles.find((f) => f.path === path)
        if (!file || file.digest !== digest) throw new Error("That file’s hooks changed since they were shown. Look at them again.")
        hookFiles = hookFiles.map((f) => (f.path === path ? { ...f, decision: allow ? "allowed" : "denied" } : f))
        emit({ type: "hooks", directory })
        emit({ type: "trust", directory })
        return hooksNow()
      }),
    trust: () => settle("trust", trustNow),
    trustAnswer: (directory, items, allow) =>
      settle("trustAnswer", () => {
        trustCalls.push({ directory, items, allow })
        const now = trustNow()
        for (const i of items) {
          const at = now.find((x) => x.path === i.path && x.kind === i.kind)
          if (!at || at.digest !== i.digest) throw new Error("That file changed since it was shown. Look at it again.")
        }
        const decision = allow ? ("allowed" as const) : ("denied" as const)
        for (const i of items) {
          if (i.kind === "hooks") hookFiles = hookFiles.map((f) => (f.path === i.path ? { ...f, decision } : f))
          else for (const x of sections) if (x.path === i.path && x.kind === i.kind) x.decision = decision
        }
        // An allowed redefinition takes the user's server's place, as the core merges it.
        if (allow && items.some((i) => i.kind === "mcp"))
          servers = servers.filter((x) => !servers.some((y) => y.replaces && y.name === x.name && y !== x)).map((x) => (x.replaces ? { ...x, replaces: false } : x))
        // As the core reloads: what waits changed, then the hooks and servers it answered.
        emit({ type: "trust", directory })
        if (items.some((i) => i.kind === "hooks")) emit({ type: "hooks", directory })
        if (items.some((i) => i.kind === "mcp")) emit({ type: "mcp", directory })
        return trustNow()
      }),
    hooksDisable: (_directory, disabled) =>
      settle("hooksDisable", () => {
        hooksOff = disabled || hookScene === "locked"
        return hooksNow()
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
