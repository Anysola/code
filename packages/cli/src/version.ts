import { Option, Schema } from "effect"

declare const OPENCODE_VERSION: string
declare const OPENCODE_CHANNEL: string
declare const OPENCODE_ARTIFACT: string
declare const OPENCODE_RELEASE_HIGHLIGHTS: string
declare const ANYSOLA_VERSION: string

const version = typeof OPENCODE_VERSION === "string" ? OPENCODE_VERSION : "local"
const channel = typeof OPENCODE_CHANNEL === "string" ? OPENCODE_CHANNEL : "local"
const artifact = typeof OPENCODE_ARTIFACT === "string" ? OPENCODE_ARTIFACT : "cli"
// Stable release builds embed the release notes' Highlights; every other build has none.
const highlights =
  typeof OPENCODE_RELEASE_HIGHLIGHTS === "string"
    ? Option.getOrElse(
        Schema.decodeUnknownOption(
          Schema.fromJsonString(Schema.Array(Schema.Struct({ title: Schema.String, body: Schema.String }))),
        )(OPENCODE_RELEASE_HIGHLIGHTS),
        () => [],
      )
    : []

export {
  version as OPENCODE_VERSION,
  channel as OPENCODE_CHANNEL,
  artifact as OPENCODE_ARTIFACT,
  highlights as OPENCODE_HIGHLIGHTS,
}
export const OPENCODE_LOCAL = channel === "local"

// Anysola Code's own release version; OPENCODE_VERSION stays the upstream version it is built from.
const anysolaVersion = typeof ANYSOLA_VERSION === "string" ? ANYSOLA_VERSION : "local"
export { anysolaVersion as ANYSOLA_VERSION }
export const ANYSOLA_VERSION_LINE = `Anysola Code ${anysolaVersion} (OpenCode ${version})`

/**
 * Version the background service registers and reports. It names both releases so updating Anysola Code on the same
 * upstream base still replaces a running older service (its plugins and RPCs differ).
 */
export const SERVICE_VERSION = `${version}+anysola.${anysolaVersion}`
