/**
 * The window's only way to the agent: the Rust commands in src-tauri/src/main.rs and their
 * `core` events. Outside Tauri (a plain browser, screenshots) the same calls go to `mock.ts`.
 */
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"

/**
 * A root session as the list shows it. `forkOf`: the session it was forked from (`forkSession`),
 * kept through renames and restarts; null for any other session. The source may since be
 * deleted, so look it up rather than assume it is listed.
 *
 * `directory` is always the main folder, also for a session working in its own worktree: every
 * call routes by it (the folder's core), and the list groups by it. `worktree` says where such a
 * session works; null for a session in the main folder (every session from before 1.1.9).
 */
export type SessionRow = {
  id: string
  title: string
  directory: string
  updated: number
  forkOf: string | null
  worktree: SessionWorktree | null
}

// ---------------------------------------------------------------------------
// Worktrees: a session in its own git worktree of the main folder
// ---------------------------------------------------------------------------

/**
 * Where a new session in a git folder works: its own worktree (the default) or the main folder.
 * A worktree is made from the main folder's `HEAD`; uncommitted main-folder changes are not in it.
 */
export type Place = "worktree" | "folder"

export type WorktreeState =
  /** Being made (checkout, env files, submodules, setup); the first message waits for it. */
  | "preparing"
  /** The session works there. Also after an apply: the session goes on from the applied commit. */
  | "ready"
  /** The folder is gone (deleted outside the app): Recreate, Discard or Fork. Turns refuse. */
  | "missing"
  /** Making it failed or was cut off; `error` says why. Try again (`worktreeRecreate`) or `worktreeToFolder`. */
  | "failed"
  /** An apply runs; everything else on the session waits. */
  | "applying"
  /** Reserved: an apply that ends the session. Not reached in 1.1.9 (an apply goes back to `ready`). */
  | "applied"
  /** Discarded: the worktree and branch are gone; the session stays readable, turns refuse. */
  | "discarded"

/** How a fork's files were put back to its fork point. */
export type ForkRestore = {
  /** Every file the snapshots cover is as it was at the fork point. */
  exact: boolean
  /** Files left as on the commit the fork started from: over 2 MB and never captured. */
  kept: string[]
  /**
   * How it was done when not from the fork point's snapshot: "edits" = the fork started from the
   * main folder's current commit with only the source session's own edits as of that message put
   * back (history from before 1.1.9); other uncommitted changes of the main folder are not in it.
   * "now" = the fork point's files are gone (`snapshots: false`, or pruned after ~2 weeks), so
   * the source's files as they are now. null when restored from the fork point's snapshot.
   * Ignored files (`.env`, `node_modules`, build output) are never in snapshots in any case.
   */
  fallback: "edits" | "now" | null
}

export type SessionWorktree = {
  /** The folder the session works in (the worktree's top; for a subfolder project the session works in the same subfolder of it). */
  path: string
  /** `anysola/<slug>`. */
  branch: string
  /** The branch it was made from; null when the main folder was on a detached HEAD. */
  baseBranch: string | null
  /** What the session's changes are measured from: moves when the base is brought in, and to the applied commit after an apply. */
  baseCommit: string
  state: WorktreeState
  /** Why it failed; with `ready`, a notice to show once (an apply cut off by a crash). */
  error: string | null
  /** The last commit an apply made on the base branch; the session goes on from it. */
  appliedCommit: string | null
  /** For a fork: how its files were put back. */
  restored: ForkRestore | null
  /** The folder or branch couldn't be removed yet (a program may hold it); retried at start. */
  leftover: boolean
  /**
   * Bringing the base in was started and not finished (conflicts open). Since 1.4.3 a Recreate
   * makes that merge again with Git when the session's last step ran during it (still `bringing`
   * afterwards); otherwise it is dropped and must be done again (`bringing` false afterwards).
   */
  bringing?: boolean
  /**
   * The main folder's branch the last apply went to (`refs/heads/<name>`, core
   * `SessionWorktreeRow.apply_branch`); null when unknown (applied before 1.4.3, detached HEAD).
   * Undo refuses unless the main folder's HEAD is that branch.
   */
  applyBranch?: string | null
}

export type WorktreeInfo =
  /** "no-commits": the repository has no commit yet; new sessions use the main folder. */
  | { available: false; reason: "not-git" | "git-missing" | "no-commits" }
  | {
      available: true
      /** null: detached HEAD. */
      branch: string | null
      /** Uncommitted files in the main folder (not carried into a new worktree: say so). */
      uncommitted: number
      /** This folder's setup command (`Prefs.worktreeSetup`). */
      setup: string | null
      /** Git older than 2.40: worktrees work, but applying needs a newer Git ("Update Git to apply"). */
      canApply: boolean
    }

/**
 * Whether the session's changes can be applied onto the main folder's current branch, checked
 * without touching the main folder. `onto`: that branch; when it differs from `baseBranch` the
 * window confirms "Apply to `onto` instead of `baseBranch`?". `behind`: commits on `onto` since
 * the base (the apply still lands on its tip).
 */
export type ApplyCheck =
  /**
   * `ignored`: files the commit would carry that the main folder's `.gitignore` ignores (a
   * force-added `.env`, say). Warn before applying; it doesn't stop the apply.
   */
  | { ok: true; onto: string; files: number; add: number; del: number; behind: number; ignored: string[] }
  /** Nothing to apply. */
  | { ok: false; reason: "empty" }
  /** A turn, a subagent or the setup command runs. */
  | { ok: false; reason: "busy" }
  /**
   * Conflict markers or unmerged files left in the worktree: after bringing the base in, or
   * markers found in any changed file with no merge in progress. Say neither cause; list the files.
   * The user may apply anyway (`worktreeApply` with `markers`) after a confirm that lists them:
   * markers can be meant (docs, test fixtures).
   */
  | {
      ok: false
      reason: "unresolved"
      files: string[]
      /** The main folder's branch now, when on one. */
      onto?: string | null
      /** "Apply anyway" is possible: no merge is open in the worktree (its markers are conflicts to finish). */
      anyway?: boolean
    }
  /** The changes conflict with `onto`: bring the base in, open the files, or discard. */
  | { ok: false; reason: "conflict"; onto: string; files: string[] }
  /** Main-folder edits in files the apply touches; apply with `stash` to set them aside and back. */
  | { ok: false; reason: "dirty"; onto: string; files: string[] }
  /** Staged changes in the main folder would go into the commit. */
  | { ok: false; reason: "staged"; onto: string }
  /** The main folder is on a detached HEAD: switch it to a branch to apply. */
  | { ok: false; reason: "detached" }
  | { ok: false; reason: "operation"; what: "merge" | "rebase" | "cherry-pick" | "revert" }
  /** Git failed, or is older than 2.40 ("Update Git to 2.40 or later to apply"); `message` says it. */
  | { ok: false; reason: "git"; message: string }

/** A session whose worktree holds changes not applied to the main folder (`files` of them; null: they couldn't be counted). */
export type WorktreeWithWork = { sessionId: string; title: string; directory: string; files: number | null }

/** The setup command's last run in a session's worktree. */
export type SetupRun = {
  command: string | null
  state: "running" | "done" | "failed" | "stopped" | "timeout"
  code: number | null
  /** The last 2000 lines; "info" lines are the app's own (preparation steps). */
  lines: { stream: "info" | "out" | "err"; text: string }[]
  started: number
  ended: number | null
}

/**
 * A session's share of the account's 5-hour window, as `sessionUsage` and `setBudget` return it
 * and the `usage` event carries it. Percents of the 5-hour window: `turn` the running (or last)
 * turn, `total` every turn of the session and of its subagents (a root's total includes them), `budget` its limit (1 to `MAX_BUDGET`, null for none).
 * `turns`: turns that used any. `fiveHour` and `weekly`: the account's windows after the last
 * model call this run of the app saw, null before one. A fork starts at zero with the default
 * budget, not its source's numbers. Subagents' model calls count in their own sessions, not in
 * `total` of the session that started them.
 */
export type SessionUsage = {
  sessionId: string
  turn: number
  total: number
  turns: number
  budget: number | null
  fiveHour: number | null
  weekly: number | null
}

/** The highest budget: the whole 5-hour window (core `usage::MAX_BUDGET`). */
export const MAX_BUDGET = 100

export type FileDiff = {
  file: string
  patch: string
  status: "added" | "modified" | "deleted"
  additions: number
  deletions: number
}

/**
 * What went wrong, for the window to pick words and actions by: `limit` a usage window is full,
 * `offline` the service can't be reached, `auth` the sign-in is gone, `model` the model isn't
 * available, `elsewhere` another Anysola Code process drives the session.
 */
export type ErrorKind = "limit" | "offline" | "auth" | "model" | "elsewhere" | "other"

export type Item =
  /**
   * `command`: the `/name args` line a command's text was rendered from. `time`: when it was sent
   * (ms). `elapsed`: how long its turn worked (ms); null while it runs or when unknown.
   */
  | {
      kind: "user"
      id: string
      text: string
      images: number
      command?: string | null
      time?: number | null
      elapsed?: number | null
    }
  | { kind: "text"; id: string; text: string }
  | { kind: "reasoning"; id: string; text: string }
  | {
      kind: "tool"
      id: string
      name: string
      status: "streaming" | "running" | "completed" | "error"
      target: string
      files: FileDiff[] | null
      output: string
      error: string | null
      /** A subagent's session. */
      child?: string | null
      /** How long it ran (ms), once it ended. */
      elapsed?: number | null
      /** What the user's hooks did around this call; absent when they did nothing worth showing. */
      hooks?: HookRun[]
    }
  /** `retry`: sending again can work (false for a full limit, a missing model, a lost sign-in). */
  | { kind: "error"; id: string; message: string; errorKind?: ErrorKind; retry?: boolean }
  /** A marker in the session, such as a compaction. */
  | { kind: "note"; id: string; text: string; failed: boolean }

export type ModelOption = { id: string; name: string; note: string | null; efforts: string[]; images: boolean }

export type AgentOption = { id: string; name: string; description: string | null }

export type Info = {
  agent: string | null
  agents: AgentOption[]
  model: string | null
  modelId: string | null
  variant: string | null
  models: ModelOption[]
  fiveHour: number | null
}

export type Window = { percent: number; resetsAt: string | null }

/** A banked reset: starts the 5-hour window over, or both windows; `expiresAt` null never expires. */
export type BankedReset = { id: string; expiresAt: string | null; scope: "five_hour" | "both" }

export type Account =
  | { expired: true; rejected?: string | null }
  | {
      expired: false
      plan: string | null
      included: boolean
      fiveHour: Window | null
      weekly: Window | null
      /** The window that refuses requests now, if one is full. */
      blockedBy: "five_hour" | "weekly" | null
      bank: BankedReset[]
      requestsToday: number | null
      /** A promotion an admin is running: unlimited use, or use counted at a multiplier. */
      event: { mode: "unlimited" | "multiplier"; label: string; until: string | null } | null
    }

/** A signed-in device or CI token of the account (`GET /tokens`). */
export type Device = {
  id: string
  name: string
  client: string
  kind: "device" | "ci"
  createdAt: string
  lastUsedAt: string | null
  expiresAt: string | null
  /** This app's own sign-in. */
  current: boolean
}

/** `GET /usage?days=`: requests only; the gateway shows no money. */
export type Usage = {
  days: number
  requests: number
  /** Prompt tokens read from the cache over all prompt tokens, 0 to 1. */
  cacheHitRate: number | null
  today: number
  /** Every day of the window, oldest first (UTC dates). */
  byDay: { date: string; requests: number }[]
  byModel: { model: string; name: string; requests: number }[]
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

/**
 * The app's own preferences, kept in `~/.anysola/desktop.json` by the Rust side (it reads
 * some itself: notifications, Discord, updates). `prefsSet` takes the whole object.
 */
export type Prefs = {
  general: {
    /** Where new sessions start (Ctrl N); null asks each time. */
    defaultFolder: string | null
    /** The mode a new session starts in; "last" keeps the one used last. */
    startMode: Mode | "last"
    /** Look for a new version when the app starts. */
    checkUpdates: boolean
    /** The editor files and folders open in (an `Editor.id`); null picks the first one found. */
    editor: string | null
  }
  appearance: {
    theme: "dark" | "light" | "system"
    textSize: "small" | "default" | "large"
    /** The conversation column. */
    width: "narrow" | "default" | "wide"
    /** Long lines in code and diffs wrap instead of scrolling sideways. */
    wrapCode: boolean
    reduceMotion: boolean
  }
  conversation: {
    /** File changes in the conversation: one row each, or the diff shown. */
    changes: "closed" | "open"
    /** A command's output. */
    commands: "closed" | "open"
    /** Reads and searches: one line for a run of them, or a row each. */
    lookups: "grouped" | "listed"
    /** Long messages and long code blocks are cut to a few lines with "Show all". */
    shorten: boolean
    send: "enter" | "ctrl-enter"
  }
  notifications: {
    enabled: boolean
    approval: boolean
    question: boolean
    finished: boolean
    failed: boolean
  }
  discord: {
    enabled: boolean
    /** What the status may name. */
    project: boolean
    session: boolean
    model: boolean
    elapsed: boolean
    /** A Discord application of the user's own; null uses the app's. */
    applicationId: string | null
  }
  sources: {
    /** Read skills from Claude Code's folders (`~/.claude/skills`, a project's `.claude/skills`). */
    claudeSkills: boolean
  }
  /** Each main folder's worktree setup command (`folderKey` of the folder); empty by default. Only the user's own, never a repository's. */
  worktreeSetup: Record<string, string>
}

/** The agent's settings in `~/.anysola/config.json`. */
export type AgentConfig = {
  file: string
  /** `model`; null follows the account's default. */
  model: string | null
  models: ModelOption[]
  websearch: boolean
  /** `experimental.todo`: the agent keeps a task list. */
  todo: boolean
  /** `experimental.subagent_depth`: how deep subagents may start their own (1 to 3). */
  subagentDepth: number
  /** `compaction.auto`: summarize the session when the context fills. */
  autoCompact: boolean
  /** `snapshots`: keep what rewinding needs to put files back. */
  snapshots: boolean
  /** `shell`; null picks one. */
  shell: string | null
  /** What picking one gives on this computer. */
  shellDetected: string | null
  /**
   * Where hooks without a `shell` of their own run with this config: core `Core::hook_shell(None)`,
   * the same function the core runs them with (the legacy, global and managed files' `shell` all
   * count; on Windows Git Bash when installed and no `shell` is set).
   */
  hooksShellDetected?: string | null
  /** `lsp`: diagnostics from language servers after edits; on by default for approved projects (`false` disables everywhere). */
  lsp: boolean
  /** `sandbox` turned on (`true`, or an object without `"enabled": false`); off by default. */
  sandbox: boolean
  /** Why the shell sandbox can't start on this computer; null when it can. */
  sandboxUnavailable: string | null
  /** Settings another file overrides for the open folder: the key and that file. */
  overridden: { key: string; file: string }[]
  /** What the config loader complained about. */
  warnings: string[]
}

export type AgentConfigPatch = Partial<
  Pick<AgentConfig, "model" | "websearch" | "todo" | "subagentDepth" | "autoCompact" | "snapshots" | "shell" | "lsp" | "sandbox">
>

export type McpServer = {
  name: string
  kind: "remote" | "local"
  /** The address, or the command line. */
  target: string
  /** `waiting`: a project file defines it and that file's MCP servers are not allowed (yet). */
  status: "connected" | "pending" | "disabled" | "failed" | "needs_auth" | "waiting"
  error: string | null
  /** The file that defines it. */
  source: string
  /** Defined in `~/.anysola/config.json`: it can be changed and removed here. */
  editable: boolean
  /** It signs in with OAuth; `signedIn` says a login is stored. */
  oauth: boolean
  signedIn: boolean
  /** Null when the count isn't known (not connected). */
  tools: number | null
  prompts: number
  /**
   * A waiting row only: a project file redefines the user's server of this name; allowing it
   * replaces theirs (which is listed as its own row).
   */
  replaces?: boolean
}

export type McpDraft =
  | { name: string; kind: "remote"; url: string; headers: Record<string, string> }
  | { name: string; kind: "local"; command: string[]; environment: Record<string, string> }

/** Where a skill, command or agent was read from: a file, or what stands in for one. */
export type Origin = "builtin" | "app" | "personal" | "project" | "claude" | "agents" | "legacy" | "mcp" | "url" | "config"

/** What the agent picks up for a folder, each with where it came from. */
export type Sources = {
  skills: { name: string; description: string | null; path: string; origin: Origin }[]
  /** `source`: a file, an MCP server's name, or null for the built-ins. */
  commands: { name: string; description: string | null; origin: Origin; source: string | null }[]
  agents: { id: string; name: string; description: string | null; origin: Origin; source: string | null }[]
  /** Instruction files in the order the agent reads them; a URL that couldn't be downloaded keeps its place, with `error`. */
  instructions: { path: string; bytes: number; origin: Origin; error?: string | null }[]
  /** Every place that is looked in, found or not: the answer to "where from". */
  places: { path: string; holds: "skills" | "commands" | "agents" | "instructions" | "config"; origin: Origin; exists: boolean }[]
  /** Each configured `skills` URL and why its index couldn't be downloaded (null: it was). */
  skillUrls: { url: string; error: string | null }[]
  /** The git repositories the agent may read, and how each download stands. */
  repositories: Repository[]
}

/**
 * A git repository from config `references`. `cloned`: a copy is on this computer; `attemptedAt`
 * (ms): the last download or update; `error`: why that one failed.
 */
export type Repository = {
  name: string
  repository: string | null
  branch: string | null
  cloned: boolean
  attemptedAt: number | null
  error: string | null
}

/**
 * The moments a hook can run at (core `hooks::EVENTS`; the last four since 1.6.0). Which can block:
 * PreToolUse, UserPromptSubmit, Stop and SubagentStop (keeps the subagent going); PreCompact only
 * a manual `/compact` (an automatic one goes on, its block shown as a notice); PostToolUse tells
 * the agent; SessionStart, Notification and SessionEnd never block (exit 2 shows stderr as a
 * notice). SessionEnd hooks share one 1.5 s budget whatever their `timeout`.
 */
export type HookEvent =
  | "PreToolUse"
  | "PostToolUse"
  | "UserPromptSubmit"
  | "Stop"
  | "SubagentStop"
  | "PreCompact"
  | "Notification"
  | "SessionStart"
  | "SessionEnd"

/**
 * Something a hook did that the user should see. `blocked`: it stopped a tool call or a message;
 * `feedback`: it told the agent something after a tool ran, or kept the turn going; `context`: it
 * added context; `error`: the hook itself failed (the work went on); `notice`: it has a message
 * for the user.
 */
export type HookRun = {
  event: HookEvent
  command: string
  outcome: "blocked" | "feedback" | "context" | "error" | "notice"
  message: string
}

/** A command the user configured to run around the agent's work. */
export type Hook = {
  event: HookEvent
  /** Which tools (or, for SessionStart, which start) it applies to; null for all. */
  matcher: string | null
  command: string
  /** Seconds. */
  timeout: number
  /** The file that defines it. */
  source: string
  scope: "user" | "project" | "managed"
  /** Whether it runs: a project hook needs its file approved, and hooks can be turned off. */
  active: boolean
  /** Set for a hook in `~/.anysola/config.json`: it can be changed and removed here by this id. */
  id: string | null
  /**
   * The hook's own `shell` as configured (a name like "powershell", "bash", "cmd", or a path);
   * null: none, it runs where `Hooks.shell` says. Always null for a project file's hook: a project
   * can't choose a hook's shell (the core drops it with a warning).
   */
  shell?: string | null
  /** The shell it runs in: core `Core::hook_shell(shell)` (its own, else the config's, else the default). */
  runsIn?: string
}

/**
 * `matcher` null or empty applies to everything; `timeout` null keeps the default; `shell` null or
 * absent writes no `shell` field (the hook runs where hooks without one do).
 */
export type HookDraft = {
  event: HookEvent
  matcher: string | null
  command: string
  timeout: number | null
  shell?: string | null
}

/**
 * A project file that defines hooks. They run only once the user allowed this content: `digest`
 * names it, and `decision` is null until the user answered for this digest.
 */
export type HookFile = {
  path: string
  digest: string
  decision: "allowed" | "denied" | null
  hooks: Hook[]
  /**
   * The hooks allowed before a change (or before they were removed) are running in place of the
   * listed ones: until the change is allowed, or, once it was refused, until the app restarts.
   */
  standing?: boolean
  /**
   * Allowed before 1.6.0, asked again only because it has hooks for moments 1.6.0 added (an
   * earlier approval never covered them). None of its hooks run until it is allowed again.
   */
  reasked?: boolean
}

/**
 * What a project file or folder can change on this computer, each approved on its own: commands it
 * runs (`hooks`, `mcp`, `formatter`), what it lets the agent do without asking (`permissions`: its
 * allow rules; rules that ask or deny apply without approval), what it makes the app download
 * (`remote`: instruction URLs and git repositories), and installed language servers (`lsp`).
 */
export type TrustKind = "hooks" | "mcp" | "formatter" | "permissions" | "remote" | "lsp"

/** One thing a project file would run. `command`: a command line or a URL; `detail`: the rest worth knowing. */
export type TrustEntry = { label: string; command: string; detail: string | null }

/**
 * A project file's hooks, MCP servers or formatters as they are now (`digest`). They are applied
 * only once the user allowed this content; `decision` is null until the user answered for it.
 * `standing` (hooks only): the version allowed before a change is running in its place.
 */
export type TrustItem = {
  path: string
  kind: TrustKind
  digest: string
  decision: "allowed" | "denied" | null
  standing: boolean
  /** Hooks only: as `HookFile.reasked`. */
  reasked?: boolean
  entries: TrustEntry[]
}

export type Hooks = {
  /** Every hook configured for the folder, in the order they run. */
  hooks: Hook[]
  /** The project's files that define hooks. */
  files: HookFile[]
  /** The user turned hooks off (managed hooks still run). */
  disabled: boolean
  /** Where hooks without a `shell` of their own run ("Hooks run in …"): core `Core::hook_shell(None)`. */
  shell?: string
}

/** A saved "always allow" answer; they are kept per project. */
export type SavedPermission = { projectId: string; folder: string | null; action: string; resource: string }

/**
 * A rule a config file or an agent file sets. `agent`: the one agent it applies to (null: every
 * agent). `project`: it comes from the folder's project. `applied` false: a project's allow rule
 * waiting for that file's `permissions` to be allowed.
 */
export type PermissionRule = {
  action: string
  resource: string
  effect: "allow" | "ask" | "deny"
  agent: string | null
  source: string
  project: boolean
  applied: boolean
}

export type DataInfo = {
  /** `~/.anysola` */
  folder: string
  database: { path: string; bytes: number }
  sessions: number
  /** What can be cleared without losing sessions or settings. */
  caches: { id: "tool-output" | "cache" | "shell"; path: string; bytes: number }[]
  log: string
  /** The one-time import from an older OpenCode-based install, if it happened. */
  imported: { sessions: number; at: number } | null
}

export type About = { version: string; os: string; arch: string }

export type DiscordStatus = { state: "off" | "connected" | "waiting"; detail: string | null }

/** What the window is showing, for the Discord status; the Rust side applies the preferences. */
export type Presence = { directory: string | null; title: string | null; model: string | null; working: boolean }

export type Question = {
  question: string
  header: string
  options: { label: string; description: string }[]
  multiple?: boolean
}

export type Attachment = { mime: string; data: string; name: string }

export type Mode = "ask" | "edits" | "auto"

export type UpdateCheck =
  | { available: false }
  | { available: true; version: string; notes?: string | null; manual?: string }

export type SendRequest = {
  sessionId: string | null
  text: string
  mode: Mode
  model: string | null
  variant: string | null
  agent: string | null
  files: Attachment[]
  skills: string[]
  /**
   * New sessions in a git folder with a commit only: its own worktree (the default) or the main
   * folder. Ignored otherwise. With a worktree, `prompt` returns at once with `queued: true`;
   * the message runs once the worktree is ready (`worktree` events say when).
   */
  place?: Place
}

/** A staged rewind: the turns it hides and the files it put back. */
export type Revert = {
  turns: number
  /** As `revert`'s result: the rewind goes back past an apply or a "Bring main in" (kept across restarts). */
  crossesApply: boolean
  files: { file: string; status: "added" | "modified" | "deleted"; additions: number; deletions: number }[]
}

export type Todo = { content: string; status: "pending" | "in_progress" | "completed" | "cancelled" | string }

/** A message sent while a turn ran: `queue` waits for the turn to end, `steer` joins it at its next step. */
export type Queued = { id: string; text: string; images: number; delivery: "queue" | "steer" }

/** How full the model's context is: the latest step's tokens over the model's window. */
export type Context = { tokens: number; limit: number }

/** A command the agent left running in the background (`sessionId` may be a subagent's). */
export type BackgroundCommand = { id: string; sessionId: string; command: string; started: number }

/**
 * `agent` wrote the last shown reply. `queued`: what waits in the session's inbox, oldest first.
 * `context`: null until a step reported its tokens. `background`: the session's and its
 * subagents' running background commands.
 */
export type View = {
  items: Item[]
  revert: Revert | null
  todos: Todo[]
  agent: string | null
  queued: Queued[]
  context: Context | null
  background: BackgroundCommand[]
}

/** `pending`: what waits for an answer in this session or its subagents. */
export type Opened = { session: SessionRow; view: View; info: Info; busy: boolean; pending: PendingRequest[] }

/**
 * One input of an MCP server's form (elicitation). `external` is the URL kind: show the message
 * and a button that opens `url`; the server ends the request itself.
 */
export type FormField = {
  key: string
  title: string
  description?: string | null
  required?: boolean
  type: "boolean" | "number" | "integer" | "string" | "multiselect" | "external"
  default?: unknown
  minimum?: number
  maximum?: number
  format?: "email" | "uri" | "date" | "date-time" | string
  minLength?: number
  maxLength?: number
  /** A closed list (`custom` false) of values to choose from. */
  options?: { value: string; label: string }[]
  custom?: boolean
  minItems?: number
  maxItems?: number
  url?: string
}

/** `accept` carries typed values by field key: boolean, number, string, or string[] for a multiselect. */
export type FormAnswer = { action: "accept"; values: Record<string, unknown> } | { action: "decline" } | { action: "cancel" }

export type PermissionRequest = {
  type: "permission"
  sessionId: string
  /** The session the user has open above a subagent's; absent when unknown. */
  rootId?: string
  directory: string
  requestId: string
  action: string
  resources: string[]
  canAlways: boolean
  files: FileDiff[] | null
}

export type QuestionRequest = {
  type: "question"
  sessionId: string
  rootId?: string
  directory: string
  requestId: string
  questions: Question[]
}

/** A form belongs to a folder's MCP server, not to a session: it shows in every open session of `directory`. */
export type FormRequest = {
  type: "form"
  directory: string
  formId: string
  server: string
  title: string
  message: string
  fields: FormField[]
}

export type PendingRequest = PermissionRequest | QuestionRequest | FormRequest

/** An editor found on this computer; `id` is what `Prefs.general.editor` stores. */
export type Editor = { id: string; name: string }

export type Palette = {
  commands: { name: string; description: string | null }[]
  skills: { name: string; description: string | null }[]
}

export type CommandRequest = {
  sessionId: string | null
  name: string
  arguments: string
  mode: Mode
  model: string | null
  variant: string | null
  agent: string | null
  /** As `SendRequest.place`: a new session's own worktree (default in a git folder) or the main folder. */
  place?: Place
}

/**
 * A tool call's input while the model writes it, for its `streaming` row (same `callId` as the
 * row's id): each one replaces the last. `target` is the row's one line so far (a file once its
 * path is whole, a command as typed); `lines` counts the text a write, an edit or a script is
 * writing, and `tail` holds its last few lines (write and edit only). All empty again when the
 * app lost pieces of it.
 */
export type ToolInput = {
  type: "toolInput"
  sessionId: string
  messageId: string
  callId: string
  name: string
  target: string
  lines: number | null
  tail: string | null
}

/**
 * A foreground command's output while it runs (the shell polls its output file a few times a
 * second): `lines` counted so far, `tail` its last few lines as a terminal shows them. Only sent
 * when it grew; the call's stored output replaces it once the call settles.
 */
export type ToolOutput = {
  type: "toolOutput"
  sessionId: string
  callId: string
  lines: number
  tail: string
}

export type CoreEvent =
  | { type: "started"; sessionId: string }
  | { type: "text" | "reasoning"; sessionId: string; messageId: string; ordinal: number; text: string }
  | { type: "changed"; sessionId: string; error?: string }
  | ToolInput
  | ToolOutput
  | { type: "retrying"; sessionId: string; attempt: number; delayMs: number; error: string }
  | PermissionRequest
  | { type: "permissionReplied"; sessionId: string; requestId: string }
  | QuestionRequest
  /** A question was answered or dropped (its turn ended). */
  | { type: "questionReplied"; sessionId: string; requestId: string }
  | FormRequest
  /** A form was answered, or its server ended it. */
  | { type: "formReplied"; directory: string; formId: string }
  | { type: "title"; sessionId: string; title: string }
  | { type: "done"; sessionId: string }
  /**
   * `unsent`: the turn never started and the message was not kept in the session, so the window
   * puts its text back in the composer.
   */
  | { type: "failed"; sessionId: string; error: string; kind?: ErrorKind; unsent?: boolean }
  /**
   * What the user asked for didn't run; no turn failed. `hook`: the user's hook of that moment
   * refused it (`PreCompact`: a `/compact`), and `error` is its reason.
   */
  | { type: "refused"; sessionId: string; error: string; kind?: ErrorKind; hook?: HookEvent }
  /** A command or compaction ended with no turn left running. */
  | { type: "settled"; sessionId: string }
  | { type: "interrupted"; sessionId: string; reason: "user" | "budget" | "declined" }
  /** After each model call, at a turn's end, and when the budget is set or cleared. */
  | ({ type: "usage" } & SessionUsage)
  | { type: "created"; sessionId: string; title: string; directory: string }
  | { type: "lagged"; directory: string }
  /** A folder's MCP server changed state (connected, failed, signed in). */
  | { type: "mcp"; directory: string }
  /**
   * A hook did something outside a tool call (a tool's hooks arrive on its item): the window
   * shows `message`. A blocked message arrives as `failed` with `unsent`.
   */
  | ({ type: "hook"; sessionId: string } & HookRun)
  /** The folder's hooks changed, or a project file's hooks now wait for the user's answer. */
  | { type: "hooks"; directory: string }
  /** What the folder's project files want to run changed, or an answer about it was recorded. */
  | { type: "trust"; directory: string }
  | { type: "notice"; message: string }
  /** The user closed the window while `busy` turns run; the window asks, then calls `quit`. */
  | { type: "closeRequested"; busy: number }
  | { type: "terminalData"; id: string; data: string }
  | { type: "terminalExit"; id: string; code: number | null }
  /** An update is downloading; `total` null when the size isn't known. */
  | { type: "update"; downloaded: number; total: number | null }
  /** A session's worktree changed (state, base, apply, restore report). `directory`: the main folder. */
  | { type: "worktree"; directory: string; sessionId: string; worktree: SessionWorktree }
  /** A line of a worktree's preparation or setup command. */
  | { type: "setup"; directory: string; sessionId: string; stream: "info" | "out" | "err"; text: string }
  | { type: "setupEnd"; directory: string; sessionId: string; run: SetupRun }

export type GitStatus =
  /** `git: "missing"`: Git isn't installed (or isn't found), which is why nothing shows. */
  | { repo: false; git?: "missing" }
  | {
      repo: true
      /** The repository's top folder; `files` and `unmerged` are relative to it. */
      root?: string | null
      branch: string
      detached: boolean
      upstream: string | null
      ahead: number
      behind: number
      files: { file: string; status: "added" | "modified" | "deleted" }[]
      /** Files with merge conflicts; committing is refused while any exist. */
      unmerged: string[]
      /** `user.name` and `user.email` are set, so a commit can be made. */
      identity: boolean
      remote: string | null
      /** The GitHub CLI is installed, so a pull request opens from here. */
      gh: boolean
    }

export type PermissionReply = "once" | "always" | "reject"

/**
 * Why a side question has no answer (core `AsideKind`): the session's budget is spent, too many
 * requests at once (after retries), the account's usage window is full, the conversation is too
 * long for the model, it was cancelled (a newer one replaced it, or `asideCancel`), the model gave
 * no text, or anything else. `message` is in plain words.
 */
export type AsideErrorKind = "budget" | "rate_limit" | "usage_limit" | "too_long" | "cancelled" | "empty" | "other"

/** A side question's answer (trimmed markdown), or why there is none. */
export type AsideResult = { text: string } | { error: { kind: AsideErrorKind; message: string } }

/** Where `/memory` notes go: `AGENTS.md` at the root of the checkout the session works in (its own worktree for a worktree session), and `~/.anysola/AGENTS.md`. Absolute paths; either may not exist yet. */
export type MemoryFiles = { project: string; personal: string }

export type MemoryScope = "project" | "personal"

export type Bridge = {
  /** `fault`: the app's own data can't be opened (a broken or locked database); signing in won't help. */
  authStatus(): Promise<{ signedIn: boolean; fault?: { message: string; folder: string } | null }>
  loginStart(): Promise<{ url: string; code: string }>
  loginWait(): Promise<{ signedIn: boolean }>
  loginCancel(): Promise<void>
  sessions(): Promise<SessionRow[]>
  /** One root session by id, listed or not (`sessions` holds the latest 200); null when it is
   * gone or a subagent's. */
  sessionRow(sessionId: string): Promise<SessionRow | null>
  openSession(directory: string, sessionId: string): Promise<Opened>
  transcript(directory: string, sessionId: string): Promise<View>
  account(): Promise<Account>
  updateCheck(): Promise<UpdateCheck>
  updateInstall(): Promise<void>
  logout(): Promise<{ signedIn: boolean }>
  folderInfo(directory: string): Promise<Info>
  selectModel(directory: string, sessionId: string, model: string, variant: string | null): Promise<Info>
  prompt(directory: string, request: SendRequest): Promise<{ sessionId: string; queued: boolean }>
  setMode(directory: string, sessionId: string, mode: Mode): Promise<void>
  interrupt(directory: string, sessionId: string): Promise<void>
  replyPermission(directory: string, requestId: string, reply: PermissionReply): Promise<boolean>
  replyQuestion(directory: string, requestId: string, answers: string[][] | null): Promise<boolean>
  /** False when the form is no longer waiting. */
  replyForm(directory: string, formId: string, answer: FormAnswer): Promise<boolean>
  /** What waits for an answer in the session, its subagents, and the folder's MCP servers. */
  pending(directory: string, sessionId: string): Promise<PendingRequest[]>
  /** Takes a waiting message out of the queue (its text is in `View.queued` before the call). */
  queueCancel(directory: string, sessionId: string, id: string): Promise<View>
  /** Hands a waiting message to the running turn at its next step instead of after it. */
  queueSteer(directory: string, sessionId: string, id: string): Promise<View>
  /** Runs what waits in the queue when no turn is running (after a stop or a failure). */
  queueSend(directory: string, sessionId: string, mode: Mode): Promise<void>
  /** Stops a background command; false when it already ended. */
  backgroundStop(directory: string, id: string): Promise<boolean>
  /** A sent message's images as data URLs, in order. */
  messageImages(directory: string, sessionId: string, messageId: string): Promise<string[]>
  renameSession(sessionId: string, title: string): Promise<void>
  /** Also removes the session's worktree and its branch (its changes are gone). */
  deleteSession(directory: string, sessionId: string): Promise<boolean>
  /**
   * Copies the session into a new one: the whole conversation, or only what came before
   * `before`, the id of a message the user sent (a `user` item of the transcript, as `revert`
   * and `resend` take; that message and everything after it are left out). The new session is a
   * root session, listed like any other, with `forkOf` set to the source; its title is
   * the source's as it is (tell a fork by `forkOf`, never by its title). Use the returned row;
   * the `created` event that also comes only refreshes the list. With a rewind staged it copies what the window shows: up to the rewind's message, as
   * if it were final (`before` must come before it). Refused for a subagent's session, while a
   * turn runs, for a `before` that is not a shown user message of the session, for the first
   * message (nothing comes before it), and with a rewind staged somewhere other than a user
   * message.
   *
   * In a git folder with a commit the fork gets its own worktree (`worktree` on the row, state
   * `preparing` until env files and setup are done), with the files put back as they were at the
   * fork point; `worktree.restored` says how exactly (show a note only when not `exact`).
   * Elsewhere (no git, the scratch folder) a fork works in the same folder as its source and
   * nothing rewinds files: the conversation is as it was then, the files stay as they are now.
   */
  forkSession(directory: string, sessionId: string, before: string | null): Promise<SessionRow>
  /**
   * Sets the session's budget, a whole percent of the 5-hour window from 1 to `MAX_BUDGET`, or
   * clears it (`null`); anything else is refused (a fraction, out of range). NaN would arrive
   * as `null` and clear it: check the number before sending. A `usage` event carries the change too.
   * Every model call of the session stops at the budget: turns (also the ones a background
   * command or a schedule wakes), `/compact`, the goal reviewer, titles, its subagents' calls.
   * Once `total` reaches it the next call does not start; a turn or `/compact` ends with
   * `interrupted`, reason `budget` (a prompt sent then ends the same way at once). The transcript
   * notes the first stop at a budget only. The check runs before each call, so `total` can end
   * a little over. Raising it above `total` or clearing it lets the next prompt run; nothing is
   * resent by itself. A session without a budget never stops for one; none starts with one.
   */
  setBudget(directory: string, sessionId: string, budget: number | null): Promise<SessionUsage>
  /** What the session has used of the 5-hour window, and its budget. */
  sessionUsage(directory: string, sessionId: string): Promise<SessionUsage>
  /**
   * `filesKept`: the folder keeps no snapshots (not git), so the turns' edits stay.
   * `crossesApply`: a worktree session's rewind goes back past an apply or a "Bring main in", so
   * its files go back to before changes already on the main folder (Review then shows them as
   * undoing those changes). Also on `View.revert`, so it holds after a restart.
   */
  revert(
    directory: string,
    sessionId: string,
    messageId: string
  ): Promise<{ text: string; view: View; filesKept: boolean; crossesApply: boolean }>
  unrevert(directory: string, sessionId: string): Promise<View>
  resend(directory: string, sessionId: string, messageId: string, text: string | null, mode: Mode): Promise<{ sessionId: string }>
  palette(directory: string): Promise<Palette>
  runCommand(directory: string, request: CommandRequest): Promise<{ sessionId: string }>
  compact(directory: string, sessionId: string): Promise<void>
  selectAgent(directory: string, sessionId: string, agent: string): Promise<Info>
  projectFiles(directory: string): Promise<string[]>
  scratchFolder(): Promise<string>
  /** Main-folder sessions: each file the session changed, from before its first change. Worktree sessions: the worktree's files vs `worktree.baseCommit` (commands' changes included). */
  changes(directory: string, sessionId: string): Promise<FileDiff[] | null>
  /** Puts one file back: as before the session changed it, or for a worktree session as on `worktree.baseCommit`. */
  restoreFile(directory: string, sessionId: string, file: string): Promise<void>
  gitStatus(directory: string): Promise<GitStatus>
  gitBranch(directory: string, name: string): Promise<void>
  /** `files`: the paths to commit; null commits every change. */
  gitCommit(directory: string, message: string, files: string[] | null): Promise<{ commit: string }>
  gitPush(directory: string): Promise<void>
  /** Fast-forward only: brings the branch up to its upstream, or fails saying why. */
  gitPull(directory: string): Promise<void>
  /** Sets the name and email commits are made with, for every repository of this user. */
  gitIdentity(name: string, email: string): Promise<void>
  gitPr(directory: string, title: string, body: string): Promise<{ url: string; created: boolean }>
  suggestCommitMessage(directory: string, sessionId: string): Promise<string>
  reveal(directory: string): Promise<void>
  /** Shows a file or folder in the system's file manager. */
  revealPath(path: string): Promise<void>
  /** The editors found on this computer, the preferred one first; empty when none. */
  editors(): Promise<Editor[]>
  /**
   * Opens the folder (`path` null) or a file of it (`path` relative to `directory` or absolute,
   * at `line` when given) in the preferred editor. With no editor found, a file is shown in its folder
   * and a folder opens in the file manager: a file is never handed to the system to "open".
   */
  openInEditor(directory: string, path: string | null, line: number | null): Promise<void>

  /** Whether new sessions of the folder can get their own worktree. */
  worktreeInfo(directory: string): Promise<WorktreeInfo>
  worktreeApplyCheck(directory: string, sessionId: string): Promise<ApplyCheck>
  /**
   * One commit with `message` on the main folder's branch `onto` (must equal the check's; the
   * user's hooks and identity apply), main folder untouched until the check passes. `stash`:
   * set main-folder edits of touched files aside and put them back after; `stashLeft` names the
   * stash when putting back conflicted ("Your edits to X are kept in the stash"). `markers`: apply
   * even though files have conflict markers (only after the user confirmed the list). After it the
   * session goes on: its worktree and branch move to `commit`, which becomes `baseCommit`. Any
   * background commands of the session stopped for it are named in a `notice` event.
   */
  worktreeApply(
    directory: string,
    sessionId: string,
    message: string,
    onto: string,
    stash: boolean,
    markers?: boolean
  ): Promise<{ commit: string; stashLeft: string | null }>
  /**
   * Undoes the last apply. A commit a remote branch already has is undone only with `pushed`
   * true (the user confirmed): until then the answer names those branches and nothing moves.
   */
  worktreeUndoApply(directory: string, sessionId: string, pushed?: boolean): Promise<{ commit: string } | { pushed: string[] }>
  /** Deletes the worktree and its branch (irreversible: confirm first). The session stays readable. */
  worktreeDiscard(directory: string, sessionId: string): Promise<void>
  /** Merges the base branch into the session's branch; conflicts stay in the worktree as markers. */
  worktreeBringBase(directory: string, sessionId: string): Promise<{ conflicts: string[] }>
  /** For `missing` or `failed`: makes the worktree again from its branch; uncommitted work comes back when it can; `preparing` again. */
  worktreeRecreate(directory: string, sessionId: string): Promise<SessionWorktree>
  /** Only while preparing or failed, before anything ran there: the session moves to the main folder (a waiting message then runs there). */
  worktreeToFolder(directory: string, sessionId: string): Promise<SessionRow>
  /**
   * Files the session changed that aren't applied to the main folder; 0 for none, for a
   * main-folder session, or a released (discarded) worktree; null when Git couldn't tell (treat
   * as maybe work: still confirm). Asked before a delete.
   */
  worktreeUnapplied(directory: string, sessionId: string): Promise<number | null>
  /** Every root session whose worktree has changes not applied to the main folder. */
  worktreesWithWork(): Promise<WorktreeWithWork[]>
  setupOutput(directory: string, sessionId: string): Promise<SetupRun | null>
  setupRun(directory: string, sessionId: string): Promise<void>
  /** False when nothing ran. */
  setupStop(directory: string, sessionId: string): Promise<boolean>

  prefsGet(): Promise<Prefs>
  prefsSet(prefs: Prefs): Promise<Prefs>

  devices(): Promise<Device[]>
  deviceRevoke(id: string): Promise<Device[]>
  usage(days: number): Promise<Usage>
  /** Spends the banked reset that expires first. */
  limitsRedeem(): Promise<Account>

  /** `directory`: the open session's folder, or the app's own when none is open. */
  configGet(directory: string): Promise<AgentConfig>
  configSet(directory: string, patch: AgentConfigPatch): Promise<AgentConfig>

  mcpList(directory: string): Promise<McpServer[]>
  mcpAdd(directory: string, server: McpDraft): Promise<McpServer[]>
  mcpRemove(directory: string, name: string): Promise<McpServer[]>
  /** Kept in the file for a server defined in `~/.anysola/config.json`; otherwise until the app closes. */
  mcpEnable(directory: string, name: string, enabled: boolean): Promise<McpServer[]>
  mcpRetry(directory: string, name: string): Promise<McpServer[]>
  /** Starts the OAuth sign-in and opens its page; `mcpSignInWait` ends when the browser is done. */
  mcpSignIn(directory: string, name: string): Promise<{ url: string; instructions: string | null }>
  mcpSignInWait(directory: string, name: string): Promise<McpServer[]>
  /** Ends a sign-in that is waiting for the browser; `mcpSignInWait` then returns. */
  mcpSignInCancel(directory: string, name: string): Promise<void>
  mcpSignOut(directory: string, name: string): Promise<McpServer[]>

  sources(directory: string): Promise<Sources>
  /** Tries the failed downloads again (instruction and skill URLs, a repository never downloaded), then lists the sources. */
  sourcesRetry(directory: string): Promise<Sources>
  /** `~/.anysola/AGENTS.md`: what the agent is told in every folder. */
  personalInstructions(): Promise<{ path: string; text: string }>
  savePersonalInstructions(text: string): Promise<void>

  hooks(directory: string): Promise<Hooks>
  /** Adds a hook to `~/.anysola/config.json`. */
  hookAdd(directory: string, hook: HookDraft): Promise<Hooks>
  hookUpdate(directory: string, id: string, hook: HookDraft): Promise<Hooks>
  hookRemove(directory: string, id: string): Promise<Hooks>
  /** Records the user's answer for a project file's hooks as they are now (`digest`). */
  hooksTrust(directory: string, path: string, digest: string, allow: boolean): Promise<Hooks>
  /** Turns the user's and the project's hooks off or back on. */
  hooksDisable(directory: string, disabled: boolean): Promise<Hooks>

  /** Everything the folder's project files want to run, with the user's answer for each. */
  trust(directory: string): Promise<TrustItem[]>
  /** Records one answer for each item as it is now (`digest`); a stale one fails the call. */
  trustAnswer(
    directory: string,
    items: { path: string; kind: TrustKind; digest: string }[],
    allow: boolean
  ): Promise<TrustItem[]>

  permissions(directory: string): Promise<{ saved: SavedPermission[]; rules: PermissionRule[] }>
  permissionForget(projectId: string, action: string, resource: string): Promise<SavedPermission[]>
  /** Every saved answer of one project. */
  permissionForgetAll(projectId: string): Promise<SavedPermission[]>

  dataInfo(): Promise<DataInfo>
  dataClear(id: DataInfo["caches"][number]["id"]): Promise<DataInfo>
  /**
   * Sessions with a turn running are kept, and so are those whose changes are being applied:
   * `applying` of the `kept` (the rest were kept because a turn runs).
   */
  deleteAllSessions(): Promise<{ deleted: number; kept: number; applying: number }>
  /** Asks where to save the session as JSON; null when cancelled. */
  exportSession(directory: string, sessionId: string): Promise<string | null>
  /**
   * Asks for a session export (a JSON file, at most 64 MiB) and imports it as a new root session
   * of `directory` (a folder on this computer), with fresh ids and nothing that grants anything
   * (no rules, folders, worktree, budget). Works whether or not the folder's core runs; a
   * `created` event may follow but only refreshes the list. Null when cancelled: the picker, or
   * the import itself (a newer import, or the app stopping its cores, cancels one in flight;
   * no notice). Rejects with a
   * finished sentence when the file isn't an export it can take; nothing is written then.
   */
  importSession(directory: string): Promise<SessionRow | null>

  /**
   * `/btw`: a one-shot answer to `question` from the session's conversation so far (finished
   * steps only), also while a turn of it runs. Nothing is stored in the session; its use counts
   * toward the session's budget. One per session: a new one cancels the one in flight, which then
   * resolves with `cancelled`. Resolves (never rejects) with the answer or why there is none.
   */
  aside(directory: string, sessionId: string, question: string): Promise<AsideResult>
  /** Cancels the session's side question in flight; false when none was. */
  asideCancel(directory: string, sessionId: string): Promise<boolean>
  /** Where `/memory` notes go for the session (null: the folder's project root). Starts the folder's core. */
  memoryFiles(directory: string, sessionId: string | null): Promise<MemoryFiles>
  /**
   * Adds `text` as a line (`- text`, whitespace collapsed) at the end of that `AGENTS.md`, making
   * the file if it is missing; no permission ask (the user asked). Refuses a file over 4 MiB, one
   * that isn't text, a project file linking outside its checkout, and a worktree that is gone.
   * Returns the file written. Sessions read it at their next step.
   */
  addMemory(directory: string, sessionId: string | null, scope: MemoryScope, text: string): Promise<{ path: string }>

  about(): Promise<About>
  notifyTest(): Promise<void>
  discordStatus(): Promise<DiscordStatus>
  presence(presence: Presence): Promise<void>

  terminalOpen(directory: string, cols: number, rows: number): Promise<{ id: string; shell: string }>
  terminalWrite(id: string, data: string): Promise<void>
  terminalResize(id: string, cols: number, rows: number): Promise<void>
  /** xterm processed `bytes` of the terminal's output: the shell sends more once the window keeps up. */
  terminalAck(id: string, bytes: number): Promise<void>
  terminalClose(id: string): Promise<void>
  /** Every terminal of this window ends: called as the window loads, so a reload leaves no shell running. */
  terminalCloseAll(): Promise<void>

  /** Closes the app even while turns run (they are stopped). */
  quit(): Promise<void>
  /** The close question was answered "keep working": closing again asks again. */
  keepOpen(): Promise<void>

  pickFolder(defaultPath?: string | null): Promise<string | null>
  openUrl(url: string): Promise<void>
  onEvent(handler: (e: CoreEvent) => void): () => void
  window: { minimize(): void; toggleMaximize(): void; close(): void }
}

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window

function tauriBridge(): Bridge {
  return {
    authStatus: () => invoke("auth_status"),
    loginStart: () => invoke("login_start"),
    loginWait: () => invoke("login_wait"),
    loginCancel: () => invoke("login_cancel"),
    sessions: () => invoke("sessions"),
    openSession: (directory, sessionId) => invoke("open_session", { directory, sessionId }),
    transcript: (directory, sessionId) => invoke("transcript", { directory, sessionId }),
    account: () => invoke("account"),
    updateCheck: () => invoke("update_check"),
    updateInstall: () => invoke("update_install"),
    logout: () => invoke("logout"),
    folderInfo: (directory) => invoke("folder_info", { directory }),
    selectModel: (directory, sessionId, model, variant) =>
      invoke("select_model", { directory, sessionId, model, variant }),
    prompt: (directory, request) => invoke("prompt", { directory, request }),
    replyQuestion: (directory, requestId, answers) => invoke("reply_question", { directory, requestId, answers }),
    replyForm: (directory, formId, answer) => invoke("reply_form", { directory, formId, answer }),
    pending: (directory, sessionId) => invoke("pending", { directory, sessionId }),
    queueCancel: (directory, sessionId, id) => invoke("queue_cancel", { directory, sessionId, id }),
    queueSteer: (directory, sessionId, id) => invoke("queue_steer", { directory, sessionId, id }),
    queueSend: (directory, sessionId, mode) => invoke("queue_send", { directory, sessionId, mode }),
    backgroundStop: (directory, id) => invoke("background_stop", { directory, id }),
    messageImages: (directory, sessionId, messageId) => invoke("message_images", { directory, sessionId, messageId }),
    renameSession: (sessionId, title) => invoke("rename_session", { sessionId, title }),
    deleteSession: (directory, sessionId) => invoke("delete_session", { directory, sessionId }),
    sessionRow: (sessionId) => invoke("session_row", { sessionId }),
    forkSession: (directory, sessionId, before) => invoke("fork_session", { directory, sessionId, before }),
    setBudget: (directory, sessionId, budget) => invoke("set_budget", { directory, sessionId, budget }),
    sessionUsage: (directory, sessionId) => invoke("session_usage", { directory, sessionId }),
    setMode: (directory, sessionId, mode) => invoke("set_mode", { directory, sessionId, mode }),
    interrupt: (directory, sessionId) => invoke("interrupt", { directory, sessionId }),
    replyPermission: (directory, requestId, reply) =>
      invoke("reply_permission", { directory, requestId, reply }),
    revert: (directory, sessionId, messageId) => invoke("revert", { directory, sessionId, messageId }),
    unrevert: (directory, sessionId) => invoke("unrevert", { directory, sessionId }),
    resend: (directory, sessionId, messageId, text, mode) =>
      invoke("resend", { directory, sessionId, messageId, text, mode }),
    palette: (directory) => invoke("palette", { directory }),
    runCommand: (directory, request) => invoke("run_command", { directory, request }),
    compact: (directory, sessionId) => invoke("compact", { directory, sessionId }),
    selectAgent: (directory, sessionId, agent) => invoke("select_agent", { directory, sessionId, agent }),
    projectFiles: (directory) => invoke("project_files", { directory }),
    scratchFolder: () => invoke("scratch_folder"),
    changes: (directory, sessionId) => invoke("changes", { directory, sessionId }),
    restoreFile: (directory, sessionId, file) => invoke("restore_file", { directory, sessionId, file }),
    worktreeInfo: (directory) => invoke("worktree_info", { directory }),
    worktreeApplyCheck: (directory, sessionId) => invoke("worktree_apply_check", { directory, sessionId }),
    worktreeApply: (directory, sessionId, message, onto, stash, markers) =>
      invoke("worktree_apply", { directory, sessionId, message, onto, stash, markers: markers ?? false }),
    worktreeUndoApply: (directory, sessionId, pushed) =>
      invoke("worktree_undo_apply", { directory, sessionId, pushed: pushed ?? false }),
    worktreeDiscard: (directory, sessionId) => invoke("worktree_discard", { directory, sessionId }),
    worktreeBringBase: (directory, sessionId) => invoke("worktree_bring_base", { directory, sessionId }),
    worktreeRecreate: (directory, sessionId) => invoke("worktree_recreate", { directory, sessionId }),
    worktreeToFolder: (directory, sessionId) => invoke("worktree_to_folder", { directory, sessionId }),
    worktreeUnapplied: (directory, sessionId) => invoke("worktree_unapplied", { directory, sessionId }),
    worktreesWithWork: () => invoke("worktrees_with_work"),
    setupOutput: (directory, sessionId) => invoke("setup_output", { directory, sessionId }),
    setupRun: (directory, sessionId) => invoke("setup_run", { directory, sessionId }),
    setupStop: (directory, sessionId) => invoke("setup_stop", { directory, sessionId }),
    gitStatus: (directory) => invoke("git_status", { directory }),
    gitBranch: (directory, name) => invoke("git_branch", { directory, name }),
    gitCommit: (directory, message, files) => invoke("git_commit", { directory, message, files }),
    gitPush: (directory) => invoke("git_push", { directory }),
    gitPull: (directory) => invoke("git_pull", { directory }),
    gitIdentity: (name, email) => invoke("git_identity", { name, email }),
    gitPr: (directory, title, body) => invoke("git_pr", { directory, title, body }),
    suggestCommitMessage: (directory, sessionId) => invoke("suggest_commit_message", { directory, sessionId }),
    reveal: (directory) => invoke("reveal", { directory }),
    revealPath: (path) => invoke("reveal_path", { path }),
    editors: () => invoke("editors"),
    openInEditor: (directory, path, line) => invoke("open_in_editor", { directory, path, line }),
    prefsGet: () => invoke("prefs_get"),
    prefsSet: (prefs) => invoke("prefs_set", { prefs }),
    devices: () => invoke("devices"),
    deviceRevoke: (id) => invoke("device_revoke", { id }),
    usage: (days) => invoke("usage", { days }),
    limitsRedeem: () => invoke("limits_redeem"),
    configGet: (directory) => invoke("config_get", { directory }),
    configSet: (directory, patch) => invoke("config_set", { directory, patch }),
    mcpList: (directory) => invoke("mcp_list", { directory }),
    mcpAdd: (directory, server) => invoke("mcp_add", { directory, server }),
    mcpRemove: (directory, name) => invoke("mcp_remove", { directory, name }),
    mcpEnable: (directory, name, enabled) => invoke("mcp_enable", { directory, name, enabled }),
    mcpRetry: (directory, name) => invoke("mcp_retry", { directory, name }),
    mcpSignIn: (directory, name) => invoke("mcp_sign_in", { directory, name }),
    mcpSignInWait: (directory, name) => invoke("mcp_sign_in_wait", { directory, name }),
    mcpSignInCancel: (directory, name) => invoke("mcp_sign_in_cancel", { directory, name }),
    mcpSignOut: (directory, name) => invoke("mcp_sign_out", { directory, name }),
    hooks: (directory) => invoke("hooks", { directory }),
    hookAdd: (directory, hook) => invoke("hook_add", { directory, hook }),
    hookUpdate: (directory, id, hook) => invoke("hook_update", { directory, id, hook }),
    hookRemove: (directory, id) => invoke("hook_remove", { directory, id }),
    hooksTrust: (directory, path, digest, allow) => invoke("hooks_trust", { directory, path, digest, allow }),
    hooksDisable: (directory, disabled) => invoke("hooks_disable", { directory, disabled }),
    trust: (directory) => invoke("trust", { directory }),
    trustAnswer: (directory, items, allow) => invoke("trust_answer", { directory, items, allow }),
    sources: (directory) => invoke("sources", { directory }),
    sourcesRetry: (directory) => invoke("sources_retry", { directory }),
    personalInstructions: () => invoke("personal_instructions"),
    savePersonalInstructions: (text) => invoke("save_personal_instructions", { text }),
    permissions: (directory) => invoke("permissions", { directory }),
    permissionForget: (projectId, action, resource) => invoke("permission_forget", { projectId, action, resource }),
    permissionForgetAll: (projectId) => invoke("permission_forget_all", { projectId }),
    dataInfo: () => invoke("data_info"),
    dataClear: (id) => invoke("data_clear", { id }),
    deleteAllSessions: () => invoke("delete_all_sessions"),
    exportSession: (directory, sessionId) => invoke("export_session", { directory, sessionId }),
    importSession: (directory) => invoke("import_session", { directory }),
    aside: (directory, sessionId, question) => invoke("aside", { directory, sessionId, question }),
    asideCancel: (directory, sessionId) => invoke("aside_cancel", { directory, sessionId }),
    memoryFiles: (directory, sessionId) => invoke("memory_files", { directory, sessionId }),
    addMemory: (directory, sessionId, scope, text) => invoke("add_memory", { directory, sessionId, scope, text }),
    about: () => invoke("about"),
    notifyTest: () => invoke("notify_test"),
    discordStatus: () => invoke("discord_status"),
    presence: (presence) => invoke("presence", { presence }),
    terminalOpen: (directory, cols, rows) => invoke("terminal_open", { directory, cols, rows }),
    terminalWrite: (id, data) => invoke("terminal_write", { id, data }),
    terminalResize: (id, cols, rows) => invoke("terminal_resize", { id, cols, rows }),
    terminalAck: (id, bytes) => invoke("terminal_ack", { id, bytes }),
    terminalClose: (id) => invoke("terminal_close", { id }),
    terminalCloseAll: () => invoke("terminal_close_all"),
    quit: () => invoke("quit"),
    keepOpen: () => invoke("keep_open"),
    async pickFolder(defaultPath) {
      const { open } = await import("@tauri-apps/plugin-dialog")
      const picked = await open({ directory: true, multiple: false, defaultPath: defaultPath ?? undefined })
      return typeof picked === "string" ? picked : null
    },
    async openUrl(url) {
      const { openUrl } = await import("@tauri-apps/plugin-opener")
      await openUrl(url)
    },
    onEvent(handler) {
      let stop: (() => void) | null = null
      let stopped = false
      void listen<CoreEvent>("core", (e) => handler(e.payload)).then((unlisten) => {
        if (stopped) unlisten()
        else stop = unlisten
      })
      return () => {
        stopped = true
        stop?.()
      }
    },
    window: {
      minimize: () => void import("@tauri-apps/api/window").then((w) => w.getCurrentWindow().minimize()),
      toggleMaximize: () =>
        void import("@tauri-apps/api/window").then((w) => w.getCurrentWindow().toggleMaximize()),
      close: () => void import("@tauri-apps/api/window").then((w) => w.getCurrentWindow().close()),
    },
  }
}

let bridge: Bridge | null = null

export async function getBridge(): Promise<Bridge> {
  if (bridge) return bridge
  bridge = inTauri ? tauriBridge() : (await import("./mock")).mockBridge()
  return bridge
}
