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
  /** Instruction files in the order the agent reads them. */
  instructions: { path: string; bytes: number; origin: Origin }[]
  /** Every place that is looked in, found or not: the answer to "where from". */
  places: { path: string; holds: "skills" | "commands" | "agents" | "instructions" | "config"; origin: Origin; exists: boolean }[]
}

/** The moments a hook can run at. */
export type HookEvent = "PreToolUse" | "PostToolUse" | "UserPromptSubmit" | "Stop" | "SessionStart"

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
}

/** `matcher` null or empty applies to everything; `timeout` null keeps the default. */
export type HookDraft = { event: HookEvent; matcher: string | null; command: string; timeout: number | null }

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
}

/** What a project file can make the app run, each approved on its own. */
export type TrustKind = "hooks" | "mcp" | "formatter"

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
  entries: TrustEntry[]
}

export type Hooks = {
  /** Every hook configured for the folder, in the order they run. */
  hooks: Hook[]
  /** The project's files that define hooks. */
  files: HookFile[]
  /** The user turned hooks off (managed hooks still run). */
  disabled: boolean
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
}

export type CoreEvent =
  | { type: "started"; sessionId: string }
  | { type: "text" | "reasoning"; sessionId: string; messageId: string; ordinal: number; text: string }
  | { type: "changed"; sessionId: string; error?: string }
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
  /** What the user asked for didn't run; no turn failed. */
  | { type: "refused"; sessionId: string; error: string; kind?: ErrorKind }
  /** A command or compaction ended with no turn left running. */
  | { type: "settled"; sessionId: string }
  | { type: "interrupted"; sessionId: string; reason: "user" | "budget" | "declined" }
  | { type: "usage"; sessionId: string; fiveHour: number | null; weekly: number | null }
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
  /** An update is downloading; `total` null when the size isn't known. */
  | { type: "update"; downloaded: number; total: number | null }

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

export type Bridge = {
  /** `fault`: the app's own data can't be opened (a broken or locked database); signing in won't help. */
  authStatus(): Promise<{ signedIn: boolean; fault?: { message: string; folder: string } | null }>
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
  /** Sessions with a turn running are kept. */
  deleteAllSessions(): Promise<{ deleted: number; kept: number }>
  /** Asks where to save the session as JSON; null when cancelled. */
  exportSession(directory: string, sessionId: string): Promise<string | null>

  about(): Promise<About>
  notifyTest(): Promise<void>
  discordStatus(): Promise<DiscordStatus>
  presence(presence: Presence): Promise<void>

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
