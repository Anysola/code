/**
 * The window's only way to the agent: the Rust commands in src-tauri/src/main.rs and their
 * `core` events. Outside Tauri (a plain browser, screenshots) the same calls go to `mock.ts`.
 */
import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"

export type SessionRow = { id: string; title: string; directory: string; updated: number }

export type FileDiff = {
  file: string
  patch: string
  status: "added" | "modified" | "deleted"
  additions: number
  deletions: number
}

export type Item =
  /** `command`: the `/name args` line a command's text was rendered from. */
  | { kind: "user"; id: string; text: string; images: number; command?: string | null }
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
    }
  | { kind: "error"; id: string; message: string }
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
}

/** The agent's settings in `~/.anysola/config.json`, which the terminal app reads too. */
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
  /** Settings another file overrides for the open folder: the key and that file. */
  overridden: { key: string; file: string }[]
  /** What the config loader complained about. */
  warnings: string[]
}

export type AgentConfigPatch = Partial<
  Pick<AgentConfig, "model" | "websearch" | "todo" | "subagentDepth" | "autoCompact" | "snapshots" | "shell">
>

export type McpServer = {
  name: string
  kind: "remote" | "local"
  /** The address, or the command line. */
  target: string
  status: "connected" | "pending" | "disabled" | "failed" | "needs_auth"
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
  /** Instruction files in the order the agent reads them. */
  instructions: { path: string; bytes: number; origin: Origin }[]
  /** Every place that is looked in, found or not: the answer to "where from". */
  places: { path: string; holds: "skills" | "commands" | "agents" | "instructions" | "config"; origin: Origin; exists: boolean }[]
}

/** A saved "always allow" answer; they are kept per project. */
export type SavedPermission = { projectId: string; folder: string | null; action: string; resource: string }

export type PermissionRule = { action: string; resource: string; effect: "allow" | "ask" | "deny"; source: string }

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
}

/** A staged rewind: the turns it hides and the files it put back. */
export type Revert = {
  turns: number
  files: { file: string; status: "added" | "modified" | "deleted"; additions: number; deletions: number }[]
}

export type Todo = { content: string; status: "pending" | "in_progress" | "completed" | "cancelled" | string }

/** `agent` wrote the last shown reply. */
export type View = { items: Item[]; revert: Revert | null; todos: Todo[]; agent: string | null }

export type Opened = { session: SessionRow; view: View; info: Info; busy: boolean }

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
}

export type CoreEvent =
  | { type: "started"; sessionId: string }
  | { type: "text" | "reasoning"; sessionId: string; messageId: string; ordinal: number; text: string }
  | { type: "changed"; sessionId: string; error?: string }
  | { type: "retrying"; sessionId: string; attempt: number; delayMs: number; error: string }
  | {
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
  | { type: "permissionReplied"; sessionId: string; requestId: string }
  | {
      type: "question"
      sessionId: string
      rootId?: string
      directory: string
      requestId: string
      questions: Question[]
    }
  | { type: "title"; sessionId: string; title: string }
  | { type: "done"; sessionId: string }
  | { type: "failed"; sessionId: string; error: string }
  /** What the user asked for didn't run; no turn failed. */
  | { type: "refused"; sessionId: string; error: string }
  /** A command or compaction ended with no turn left running. */
  | { type: "settled"; sessionId: string }
  | { type: "interrupted"; sessionId: string; reason: "user" | "budget" | "declined" }
  | { type: "usage"; sessionId: string; fiveHour: number | null; weekly: number | null }
  | { type: "created"; sessionId: string; title: string; directory: string }
  | { type: "lagged"; directory: string }
  /** A folder's MCP server changed state (connected, failed, signed in). */
  | { type: "mcp"; directory: string }
  | { type: "notice"; message: string }

export type GitStatus =
  | { repo: false }
  | {
      repo: true
      branch: string
      detached: boolean
      upstream: string | null
      ahead: number
      behind: number
      files: { file: string; status: "added" | "modified" | "deleted" }[]
      remote: string | null
      /** The GitHub CLI is installed, so a pull request opens from here. */
      gh: boolean
    }

export type PermissionReply = "once" | "always" | "reject"

export type Bridge = {
  authStatus(): Promise<{ signedIn: boolean }>
  loginStart(): Promise<{ url: string; code: string }>
  loginWait(): Promise<{ signedIn: boolean }>
  loginCancel(): Promise<void>
  sessions(): Promise<SessionRow[]>
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
  renameSession(sessionId: string, title: string): Promise<void>
  deleteSession(directory: string, sessionId: string): Promise<boolean>
  /** `filesKept`: the folder keeps no snapshots (not git), so the turns' edits stay. */
  revert(
    directory: string,
    sessionId: string,
    messageId: string
  ): Promise<{ text: string; view: View; filesKept: boolean }>
  unrevert(directory: string, sessionId: string): Promise<View>
  resend(directory: string, sessionId: string, messageId: string, text: string | null, mode: Mode): Promise<{ sessionId: string }>
  palette(directory: string): Promise<Palette>
  runCommand(directory: string, request: CommandRequest): Promise<{ sessionId: string }>
  compact(directory: string, sessionId: string): Promise<void>
  selectAgent(directory: string, sessionId: string, agent: string): Promise<Info>
  projectFiles(directory: string): Promise<string[]>
  scratchFolder(): Promise<string>
  changes(directory: string, sessionId: string): Promise<FileDiff[] | null>
  restoreFile(directory: string, sessionId: string, file: string): Promise<void>
  gitStatus(directory: string): Promise<GitStatus>
  gitBranch(directory: string, name: string): Promise<void>
  gitCommit(directory: string, message: string): Promise<{ commit: string }>
  gitPush(directory: string): Promise<void>
  gitPr(directory: string, title: string, body: string): Promise<{ url: string; created: boolean }>
  suggestCommitMessage(directory: string, sessionId: string): Promise<string>
  reveal(directory: string): Promise<void>
  /** Shows a file or folder in the system's file manager. */
  revealPath(path: string): Promise<void>

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
  mcpSignOut(directory: string, name: string): Promise<McpServer[]>

  sources(directory: string): Promise<Sources>
  /** `~/.anysola/AGENTS.md`: what the agent is told in every folder. */
  personalInstructions(): Promise<{ path: string; text: string }>
  savePersonalInstructions(text: string): Promise<void>

  permissions(directory: string): Promise<{ saved: SavedPermission[]; rules: PermissionRule[] }>
  permissionForget(projectId: string, action: string, resource: string): Promise<SavedPermission[]>
  /** Every saved answer of one project. */
  permissionForgetAll(projectId: string): Promise<SavedPermission[]>

  dataInfo(): Promise<DataInfo>
  dataClear(id: DataInfo["caches"][number]["id"]): Promise<DataInfo>
  /** Sessions with a turn running are kept. */
  deleteAllSessions(): Promise<{ deleted: number; kept: number }>
  /** Asks where to save the session as JSON; null when cancelled. */
  exportSession(directory: string, sessionId: string): Promise<string | null>

  about(): Promise<About>
  notifyTest(): Promise<void>
  discordStatus(): Promise<DiscordStatus>
  presence(presence: Presence): Promise<void>

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
    renameSession: (sessionId, title) => invoke("rename_session", { sessionId, title }),
    deleteSession: (directory, sessionId) => invoke("delete_session", { directory, sessionId }),
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
    gitStatus: (directory) => invoke("git_status", { directory }),
    gitBranch: (directory, name) => invoke("git_branch", { directory, name }),
    gitCommit: (directory, message) => invoke("git_commit", { directory, message }),
    gitPush: (directory) => invoke("git_push", { directory }),
    gitPr: (directory, title, body) => invoke("git_pr", { directory, title, body }),
    suggestCommitMessage: (directory, sessionId) => invoke("suggest_commit_message", { directory, sessionId }),
    reveal: (directory) => invoke("reveal", { directory }),
    revealPath: (path) => invoke("reveal_path", { path }),
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
    mcpSignOut: (directory, name) => invoke("mcp_sign_out", { directory, name }),
    sources: (directory) => invoke("sources", { directory }),
    personalInstructions: () => invoke("personal_instructions"),
    savePersonalInstructions: (text) => invoke("save_personal_instructions", { text }),
    permissions: (directory) => invoke("permissions", { directory }),
    permissionForget: (projectId, action, resource) => invoke("permission_forget", { projectId, action, resource }),
    permissionForgetAll: (projectId) => invoke("permission_forget_all", { projectId }),
    dataInfo: () => invoke("data_info"),
    dataClear: (id) => invoke("data_clear", { id }),
    deleteAllSessions: () => invoke("delete_all_sessions"),
    exportSession: (directory, sessionId) => invoke("export_session", { directory, sessionId }),
    about: () => invoke("about"),
    notifyTest: () => invoke("notify_test"),
    discordStatus: () => invoke("discord_status"),
    presence: (presence) => invoke("presence", { presence }),
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
