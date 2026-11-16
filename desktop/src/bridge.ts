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
  | { kind: "user"; id: string; text: string; images: number }
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
    }
  | { kind: "error"; id: string; message: string }

export type Info = { model: string | null; variant: string | null; fiveHour: number | null }

export type Opened = { session: SessionRow; items: Item[]; info: Info; busy: boolean }

export type CoreEvent =
  | { type: "started"; sessionId: string }
  | { type: "text" | "reasoning"; sessionId: string; messageId: string; ordinal: number; text: string }
  | { type: "changed"; sessionId: string; error?: string }
  | { type: "retrying"; sessionId: string; attempt: number; delayMs: number; error: string }
  | {
      type: "permission"
      sessionId: string
      requestId: string
      action: string
      resources: string[]
      canAlways: boolean
      files: FileDiff[] | null
    }
  | { type: "permissionReplied"; sessionId: string; requestId: string }
  | { type: "title"; sessionId: string; title: string }
  | { type: "done"; sessionId: string }
  | { type: "failed"; sessionId: string; error: string }
  | { type: "interrupted"; sessionId: string; reason: "user" | "budget" | "declined" }
  | { type: "usage"; sessionId: string; fiveHour: number | null; weekly: number | null }
  | { type: "created"; sessionId: string; title: string; directory: string }
  | { type: "lagged"; directory: string }

export type PermissionReply = "once" | "always" | "reject"

export type Bridge = {
  authStatus(): Promise<{ signedIn: boolean }>
  loginStart(): Promise<{ url: string; code: string }>
  loginWait(): Promise<{ signedIn: boolean }>
  sessions(): Promise<SessionRow[]>
  openSession(directory: string, sessionId: string): Promise<Opened>
  transcript(directory: string, sessionId: string): Promise<Item[]>
  folderInfo(directory: string): Promise<Info>
  prompt(
    directory: string,
    sessionId: string | null,
    text: string,
    mode: "ask" | "edits" | "auto"
  ): Promise<{ sessionId: string; queued: boolean }>
  setMode(directory: string, sessionId: string, mode: "ask" | "edits" | "auto"): Promise<void>
  interrupt(directory: string, sessionId: string): Promise<void>
  replyPermission(directory: string, requestId: string, reply: PermissionReply): Promise<boolean>
  pickFolder(): Promise<string | null>
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
    sessions: () => invoke("sessions"),
    openSession: (directory, sessionId) => invoke("open_session", { directory, sessionId }),
    transcript: (directory, sessionId) => invoke("transcript", { directory, sessionId }),
    folderInfo: (directory) => invoke("folder_info", { directory }),
    prompt: (directory, sessionId, text, mode) => invoke("prompt", { directory, sessionId, text, mode }),
    setMode: (directory, sessionId, mode) => invoke("set_mode", { directory, sessionId, mode }),
    interrupt: (directory, sessionId) => invoke("interrupt", { directory, sessionId }),
    replyPermission: (directory, requestId, reply) =>
      invoke("reply_permission", { directory, requestId, reply }),
    async pickFolder() {
      const { open } = await import("@tauri-apps/plugin-dialog")
      const picked = await open({ directory: true, multiple: false })
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
