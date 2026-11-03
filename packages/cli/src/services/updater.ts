import { Global } from "@opencode/util/global"
import { AppProcess } from "@opencode/util/process"
import { ANYSOLA_VERSION } from "../version"
import { spawn } from "node:child_process"
import { AnysolaVersion } from "./anysola-version"
import { Context, Duration, Effect, FileSystem, Layer, Option, Schema } from "effect"
import { ChildProcess } from "effect/process"
import { parse, type ParseError } from "jsonc-parser"
import path from "node:path"
import { stripVTControlCharacters } from "node:util"
import { RetainedImage } from "./retained-image"
import type { Policy } from "./updater-action"

export const methods = ["curl", "npm", "pnpm", "bun", "yarn", "vp", "brew", "winget"] as const

export type Method = (typeof methods)[number]
export type RunResult = { readonly type: "available" | "installed"; readonly version: string }
export type CheckResult = RunResult | { readonly type: "unavailable"; readonly message: string }

export class UpgradeError extends Error {
  readonly title: string
  readonly detail: string
  readonly command?: string
  readonly retry: string

  constructor(
    input: {
      readonly title: string
      readonly detail: string
      readonly command?: string
      readonly retry: string
    },
    options?: ErrorOptions,
  ) {
    super(input.detail, options)
    this.name = "UpgradeError"
    this.title = input.title
    this.detail = input.detail
    this.command = input.command
    this.retry = input.retry
  }
}

const decodeVpPackages = Schema.decodeUnknownOption(
  Schema.fromJsonString(Schema.Array(Schema.Struct({ name: Schema.String }))),
)

const wingetPackage = "Anomaly.OpenCode"
const wingetArgs = [
  "--id",
  wingetPackage,
  "--exact",
  "--source",
  "winget",
  "--accept-source-agreements",
  "--disable-interactivity",
]

function conciseDetail(input: string) {
  const lines = stripVTControlCharacters(input)
    .trim()
    .replaceAll("\r", "\n")
    .split("\n")
    .map((line) => line.trimEnd())
    .filter((line) => line.trim().length > 0)
  const tail = lines.slice(-12).join("\n")
  const clipped = tail.length > 2_000
  const detail = clipped ? `…${tail.slice(-1_999)}` : tail
  if (!detail) return
  if (lines.length <= 12 && !clipped) return detail
  return `${detail}\n\nOutput shortened to the last 12 lines.`
}

function resultDetail(result: { code: number; stdout: string; stderr: string }) {
  return (
    conciseDetail(result.stderr) ??
    conciseDetail(result.stdout) ??
    `The command exited with code ${result.code} without any error output.`
  )
}

export interface Interface {
  readonly run: (onInstall?: (version: string) => void) => Effect.Effect<RunResult | undefined>
  readonly check: () => Effect.Effect<CheckResult | undefined, Error>
  readonly apply: (version: string) => Effect.Effect<void, Error>
  readonly method: () => Effect.Effect<Method | undefined>
  readonly latest: (method?: Method) => Effect.Effect<string, Error>
  readonly upgrade: (method: Method, version: string) => Effect.Effect<void, Error>
  readonly removal: (
    method: Method,
  ) => { readonly command: ReadonlyArray<string>; readonly run: Effect.Effect<void, Error> } | undefined
}

export class Service extends Context.Service<Service, Interface>()("@opencode/cli/Updater") {}

export function decodePolicy(text: string): Policy | undefined {
  // The CLI only projects this host-level preference instead of initializing
  // the location-scoped server configuration graph.
  const errors: ParseError[] = []
  const input: unknown = parse(text, errors, { allowTrailingComma: true })
  if (errors.length || typeof input !== "object" || input === null) return
  if ("update" in input) {
    const value = input.update
    if (value === "disable" || value === "notify" || value === "auto") return value
    return
  }
  if (!("autoupdate" in input)) return
  if (input.autoupdate === false) return "disable"
  if (input.autoupdate === "notify") return "notify"
  if (input.autoupdate === true) return "auto"
}

const make = Effect.gen(function* () {
  const fs = yield* FileSystem.FileSystem
  const global = yield* Global.Service
  const appProcess = yield* AppProcess.Service
  const executable = yield* fs.realPath(process.execPath).pipe(Effect.orElseSucceed(() => process.execPath))
  const installedPackage = yield* Effect.gen(function* () {
    const directory = path.dirname(path.dirname(executable))
    const manifest: { name: string; bin?: Record<string, string> } = yield* fs
      .readFileString(path.join(directory, "package.json"))
      .pipe(Effect.flatMap((text) => Effect.try(() => JSON.parse(text))))
    // Source invocations run inside Bun or Node, which may themselves be npm packages.
    if (!/^@opencode(?:-ai)?\/cli(?:-node)?$/.test(manifest.name)) return
    if (Object.values(manifest.bin ?? {}).some((bin) => path.resolve(directory, bin) === executable))
      return manifest.name
  }).pipe(Effect.orElseSucceed(() => undefined))
  // "opencode" is Homebrew Core's formula; the others are published to anomalyco/tap.
  const installedFormula = ["opencode", "opencode-beta", "opencode-v2"].find((name) =>
    executable.includes(`${path.sep}Cellar${path.sep}${name}${path.sep}`),
  )
  // WinGet extracts portable packages into a directory named after the package identifier.
  const installedWinget = /[\\/]winget[\\/]packages[\\/]anomaly\.opencode_/i.test(executable)

  const exec = Effect.fnUntraced(function* (command: string[], timeout: Duration.Input = "30 seconds") {
    return yield* appProcess
      .run(ChildProcess.make(command[0], command.slice(1)), {
        timeout,
        maxOutputBytes: 100_000,
        maxErrorBytes: 100_000,
      })
      .pipe(
        Effect.map((result) => ({
          code: result.exitCode,
          stdout: result.stdout.toString("utf8"),
          stderr: result.stderr.toString("utf8"),
        })),
      )
  })

  // Anysola Code: the install script's binary (~/.anysola/bin/anysola).
  const curlPath = AnysolaVersion.scriptInstallPath()
  const curlBinary = yield* fs.realPath(curlPath).pipe(Effect.orElseSucceed(() => curlPath))

  const method = Effect.fnUntraced(function* () {
    if (executable === curlBinary) return "curl"
    if (installedFormula) return "brew"
    if (installedWinget) return "winget"
    if (!installedPackage) return

    const parts = executable.toLowerCase().split(path.sep).map((part) => part.replace(/^\./, ""))
    const checks: Array<{ method: Method; command: string[] }> = [
      { method: "npm", command: ["npm", "list", "-g", "--depth=0", installedPackage] },
      { method: "pnpm", command: ["pnpm", "list", "-g", "--depth=0", installedPackage] },
      { method: "bun", command: ["bun", "pm", "ls", "-g"] },
      { method: "yarn", command: ["yarn", "global", "list"] },
      { method: "vp", command: ["vp", "list", "-g", "--json", installedPackage] },
    ]
    checks.sort((a, b) => Number(parts.includes(b.method)) - Number(parts.includes(a.method)))
    const results = yield* Effect.forEach(
      checks,
      (check) =>
        exec(check.command).pipe(
          Effect.orElseSucceed(() => ({ code: 1, stdout: "", stderr: "" })),
          Effect.map((result) => ({ check, result })),
        ),
      { concurrency: "unbounded" },
    )
    return results.find((result) => {
      if (result.check.method !== "vp") return result.result.stdout.includes(installedPackage)
      // Vite+ repeats the filter in its successful no-match message, so substring detection would be a false positive.
      return Option.exists(decodeVpPackages(result.result.stdout), (packages) =>
        packages.some((item) => item.name === installedPackage),
      )
    })?.check.method
  })

  const removalCommand = (method: Method) => {
    if (method === "winget") return ["winget", "uninstall", ...wingetArgs]
    if (method === "curl" || method === "brew" || !installedPackage) return undefined
    const commands = {
      npm: ["npm", "uninstall", "--global", installedPackage],
      pnpm: ["pnpm", "remove", "--global", installedPackage],
      bun: ["bun", "remove", "--global", installedPackage],
      yarn: ["yarn", "global", "remove", installedPackage],
      vp: ["vp", "uninstall", "-g", installedPackage],
    }
    return commands[method]
  }

  const removal = (method: Method) => {
    const command = removalCommand(method)
    if (!command) return undefined
    return {
      command,
      run: retaining(
        method,
        exec(command, "5 minutes").pipe(
          Effect.flatMap((result) => (result.code === 0 ? Effect.void : Effect.fail(new Error(resultDetail(result))))),
        ),
        global.tmp,
      ),
    }
  }

  // On Windows the installer must delete or replace the running binary, which only works
  // while another link to it exists (see RetainedImage). Upgrades keep that link in the
  // cache; uninstall has already removed the cache, so it uses the temporary directory.
  const retaining = <A, E, R>(method: Method, effect: Effect.Effect<A, E, R>, directory = global.cache) => {
    if (process.platform !== "win32" || method === "brew") return effect
    // Only the installed binary is at stake; source checkouts run inside bun or node.
    const owned =
      method === "curl"
        ? executable === curlBinary
        : method === "winget"
          ? installedWinget
          : installedPackage !== undefined
    if (!owned) return effect
    return Effect.scoped(RetainedImage.retain(directory, "upgrade").pipe(Effect.andThen(effect))).pipe(
      Effect.provideService(FileSystem.FileSystem, fs),
    )
  }

  // Anysola Code: releases come from npm (`@anysola/code`) and the install script; GET /v1/code/version says which
  // one is latest. Upstream's update server, installers, Homebrew and WinGet paths are removed; `method` and
  // `removal` stay for uninstall.
  const anysolaCheck = Effect.promise(() => AnysolaVersion.fetchInfo()).pipe(
    Effect.map((info) =>
      info && AnysolaVersion.released() && AnysolaVersion.compare(info.latest, ANYSOLA_VERSION) > 0
        ? { type: "available" as const, version: info.latest }
        : undefined,
    ),
  )
  const npmInstall = (version: string) =>
    Effect.tryPromise({
      try: () =>
        new Promise<void>((resolve, reject) => {
          const [command, ...args] = AnysolaVersion.updateCommand(version)
          const child = spawn(command!, args, {
            stdio: "ignore",
            shell: process.platform === "win32" && command === "npm",
          })
          child.on("error", reject)
          child.on("exit", (code) => (code === 0 ? resolve() : reject(new Error(`${command} exited with ${code}`))))
        }),
      catch: (cause) => new Error(`${AnysolaVersion.UPDATE_HINT} (${cause instanceof Error ? cause.message : cause})`),
    })
  return Service.of({
    run: () => anysolaCheck,
    check: () => anysolaCheck,
    apply: npmInstall,
    method,
    latest: () =>
      Effect.promise(() => AnysolaVersion.fetchInfo()).pipe(
        Effect.flatMap((info) => (info ? Effect.succeed(info.latest) : Effect.fail(new Error(AnysolaVersion.UPDATE_HINT)))),
      ),
    upgrade: (_method, version) => npmInstall(version),
    removal,
  })
})

export const UPDATE_HINT = AnysolaVersion.UPDATE_HINT

export const layer = Layer.effect(Service, make)

export * as Updater from "./updater"
export { action, type Action, type Policy } from "./updater-action"
