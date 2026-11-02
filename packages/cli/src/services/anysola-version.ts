import { ANYSOLA_VERSION } from "../version"

// Anysola Code updates: `GET /v1/code/version` names the latest and the minimum supported release; npm installs it.

export const NPM_PACKAGE = "@anysola/code"
export const UPDATE_COMMAND = ["npm", "i", "-g", `${NPM_PACKAGE}@latest`] as const
export const UPDATE_HINT = `Update Anysola Code with: ${UPDATE_COMMAND.join(" ")}`

export type Info = { readonly latest: string; readonly minimum?: string; readonly notes?: string }

export function apiURL() {
  return (process.env.ANYSOLA_API_URL || "https://api.anysola.com").replace(/\/+$/, "")
}

/** Development builds (0.0.0-*, "local") never ask to update. */
export const released = () => /^\d+\.\d+\.\d+/.test(ANYSOLA_VERSION) && !ANYSOLA_VERSION.startsWith("0.0.0")

export async function fetchInfo(timeout = 5_000): Promise<Info | undefined> {
  try {
    const response = await fetch(`${apiURL()}/v1/code/version`, {
      headers: { accept: "application/json" },
      signal: AbortSignal.timeout(timeout),
    })
    if (!response.ok) return undefined
    const body = (await response.json()) as Record<string, unknown>
    if (typeof body.latest !== "string") return undefined
    return {
      latest: body.latest.replace(/^v/, ""),
      minimum: typeof body.minimum === "string" ? body.minimum.replace(/^v/, "") : undefined,
      notes: typeof body.notes === "string" ? body.notes : undefined,
    }
  } catch {
    return undefined
  }
}

/** Semver order; a prerelease sorts before its release. */
export function compare(a: string, b: string) {
  const parse = (value: string) => {
    const [core = "", pre] = value.split("-", 2)
    return { parts: core.split(".").map((part) => Number.parseInt(part, 10) || 0), pre }
  }
  const left = parse(a)
  const right = parse(b)
  for (let index = 0; index < 3; index++) {
    const diff = (left.parts[index] ?? 0) - (right.parts[index] ?? 0)
    if (diff !== 0) return Math.sign(diff)
  }
  if (left.pre === right.pre) return 0
  if (left.pre === undefined) return 1
  if (right.pre === undefined) return -1
  return left.pre < right.pre ? -1 : 1
}

/** Stops a release older than the server's minimum before it talks to the gateway. Offline: allowed. */
export async function enforceMinimum() {
  if (!released()) return
  const info = await fetchInfo(3_000)
  if (!info?.minimum || compare(ANYSOLA_VERSION, info.minimum) >= 0) return
  process.stderr.write(
    `Anysola Code ${ANYSOLA_VERSION} is no longer supported (minimum ${info.minimum}).\n${UPDATE_HINT}\n`,
  )
  process.exit(1)
}

export * as AnysolaVersion from "./anysola-version"
