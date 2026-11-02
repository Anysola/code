export * as Anysola from "./anysola.js"

import { Config } from "@opencode/schema/config"
import { ConfigAgent } from "@opencode/schema/config/agent"
import { ConfigProvider } from "@opencode/schema/config/provider"

declare const ANYSOLA_VERSION: string | undefined

/**
 * Anysola Code is Anysola-only: the one model provider is `anysola`, served by the Anysola Code gateway.
 * Everything that decides "which providers exist" funnels through these helpers so the restriction lives in one place.
 */
export const providerID = "anysola"
export const integrationID = "anysola"
export const tokenEnv = "ANYSOLA_TOKEN"
export const version = typeof ANYSOLA_VERSION === "string" ? ANYSOLA_VERSION : "0.0.0-dev"

/** API origin. `ANYSOLA_API_URL` overrides it for tests and staging. */
export function apiURL() {
  return (process.env.ANYSOLA_API_URL || "https://api.anysola.com").replace(/\/+$/, "")
}

export function isAllowedProvider(id: string | undefined) {
  return id === providerID
}

/** Integrations the user may see or connect: Anysola itself and MCP servers (which are tools, not model providers). */
export function isAllowedIntegration(id: string, metadata: Readonly<Record<string, unknown>> | undefined) {
  return id === integrationID || metadata?.source === "mcp"
}

/**
 * Drops everything in a local config document that would add or select another provider: `providers.<other>`,
 * a default `model` or an agent `model` on another provider. Local overrides of `providers.anysola` are kept
 * except the endpoint and credentials, which only the Anysola remote config sets.
 */
export function sanitize(info: Config.Info): Config.Info {
  const providers = info.providers
  const agents = info.agents
  const foreignModel = info.model !== undefined && !isAllowedProvider(info.model.providerID)
  const foreignProviders = providers !== undefined && Object.keys(providers).some((id) => id !== providerID)
  const ownProvider = providers?.[providerID]
  const lockedSettings =
    ownProvider?.settings !== undefined &&
    ["baseURL", "apiKey", "authToken", "accessToken"].some((key) => key in (ownProvider.settings ?? {}))
  const foreignAgents =
    agents !== undefined &&
    Object.values(agents).some((agent) => agent.model !== undefined && !isAllowedProvider(agent.model.providerID))
  if (!foreignModel && !foreignProviders && !foreignAgents && !lockedSettings && ownProvider?.package === undefined && ownProvider?.canonical === undefined)
    return info
  return new Config.Info({
    ...info,
    model: foreignModel ? undefined : info.model,
    providers: ownProvider
      ? {
          [providerID]: new ConfigProvider.Info({
            ...ownProvider,
            canonical: undefined,
            package: undefined,
            env: undefined,
            settings: ownProvider.settings && withoutEndpoint(ownProvider.settings),
          }),
        }
      : undefined,
    agents:
      agents &&
      Object.fromEntries(
        Object.entries(agents).map(([name, agent]) => [
          name,
          agent.model !== undefined && !isAllowedProvider(agent.model.providerID)
            ? new ConfigAgent.Info({ ...agent, model: undefined })
            : agent,
        ]),
      ),
  })
}

function withoutEndpoint<Value>(settings: Readonly<Record<string, Value>>) {
  return Object.fromEntries(
    Object.entries(settings).filter(([key]) => !["baseURL", "apiKey", "authToken", "accessToken"].includes(key)),
  )
}
