import os from "node:os"
import { Clock, Duration, Effect, FiberHandle, Option, Schema, Scope, Semaphore, Stream } from "effect"
import type { IntegrationOAuthMethodRegistration } from "@opencode/plugin/effect/integration"
import type { ProviderEditor } from "@opencode/plugin/effect/provider"
import { define } from "@opencode/plugin/effect/plugin"
import { Config as ConfigSchema } from "@opencode/schema/config"
import { Anysola } from "../../anysola.js"
import { Bus } from "../../bus.js"
import { Config } from "../../config.js"
import { ConfigIntegration } from "../../config/integration.js"
import { ConfigNormalize } from "../../config/normalize.js"
import { ConfigProviderPlugin } from "../../config/plugin/provider.js"
import { Credential } from "../../credential.js"
import { Integration } from "../../integration.js"
import { IntegrationConnection } from "../../integration/connection.js"
import { KV } from "../../kv.js"
import { creditsEventHeader } from "./anysola-credits-stream.js"

// Anysola Code's built-in login and provider. Login is the Anysola device flow (account.anysola.com/device); the
// resulting token is stored in the regular credential store as the `anysola` integration's OAuth credential, so
// the model resolver sends it as `Authorization: Bearer`. Models, the default model and the title model come from
// `GET /v1/code/config`, merged as this integration's config document.

const methodID = Integration.MethodID.make("device")
const integrationID = Integration.ID.make(Anysola.integrationID)
const requestTimeout = 15_000
export const cacheKey = "anysola:config"
const signedOut = "Run `anysola login` to sign in to Anysola"

type Json = Record<string, unknown>
type Reply = { status: number; body: Json }

const str = (value: unknown) => (typeof value === "string" && value ? value : undefined)
const num = (value: unknown) => (typeof value === "number" && Number.isFinite(value) ? value : undefined)

export function call(method: "GET" | "POST", route: string, options: { body?: unknown; token?: string } = {}) {
  return Effect.tryPromise({
    try: async (): Promise<Reply> => {
      const headers: Record<string, string> = { accept: "application/json", "x-anysola-version": Anysola.version }
      if (options.body !== undefined) headers["content-type"] = "application/json"
      if (options.token) headers.authorization = `Bearer ${options.token}`
      const response = await fetch(Anysola.apiURL() + route, {
        method,
        headers,
        body: options.body === undefined ? undefined : JSON.stringify(options.body),
        signal: AbortSignal.timeout(requestTimeout),
      })
      const text = await response.text()
      let body: Json = {}
      try {
        const parsed: unknown = text ? JSON.parse(text) : {}
        if (parsed && typeof parsed === "object") body = parsed as Json
      } catch {
        // A proxy error page; the status is enough.
      }
      return { status: response.status, body }
    },
    catch: (cause) => new Error(`Cannot reach ${Anysola.apiURL()}`, { cause }),
  })
}

/** `{error, error_description}` from device endpoints or the Anthropic-style envelope from the gateway. */
export function errorOf(reply: Reply) {
  const error = reply.body.error
  const code = typeof error === "string" ? error : str((error as Json | undefined)?.type)
  const message =
    str(reply.body.error_description) ?? str((error as Json | undefined)?.message) ?? str(reply.body.message)
  return { code, message: message ?? code ?? `HTTP ${reply.status}` }
}

function poll(deviceCode: string, interval: number, deadline: number): Effect.Effect<Credential.OAuth, Error> {
  return Effect.gen(function* () {
    let wait = interval
    let failures = 0
    while (true) {
      yield* Effect.sleep(Duration.seconds(wait))
      if ((yield* Clock.currentTimeMillis) >= deadline)
        return yield* Effect.fail(new Error("The login code expired. Run `anysola login` again."))
      const reply = yield* call("POST", "/v1/code/device/token", { body: { device_code: deviceCode } }).pipe(
        Effect.option,
      )
      if (Option.isSome(reply) && reply.value.status >= 200 && reply.value.status < 300) {
        const token = str(reply.value.body.access_token)
        if (!token || !/^asc_[0-9A-Za-z]+$/.test(token))
          return yield* Effect.fail(new Error("Login failed: the server returned no usable token."))
        const expiresAt = Date.parse(str(reply.value.body.expires_at) ?? "")
        return Credential.OAuth.make({
          type: "oauth",
          methodID,
          access: token,
          refresh: "",
          // The token has no refresh flow; a revoked or expired one surfaces as a 401 and asks for a new login.
          expires: Number.isFinite(expiresAt) ? expiresAt : Date.now() + 365 * 24 * 60 * 60 * 1000,
          metadata: { apiURL: Anysola.apiURL(), device: os.hostname() },
        })
      }
      if (Option.isSome(reply) && reply.value.status < 500) {
        failures = 0
        const error = errorOf(reply.value)
        if (error.code === "authorization_pending") continue
        if (error.code === "slow_down") {
          wait += 5
          continue
        }
        if (error.code === "expired_token")
          return yield* Effect.fail(new Error("The code expired before it was approved. Run `anysola login` again."))
        if (error.code === "access_denied") return yield* Effect.fail(new Error("Login was denied."))
        return yield* Effect.fail(new Error(`Login failed: ${error.message}`))
      }
      // Network errors and 5xx: keep polling unless they repeat.
      failures += 1
      if (failures >= 3)
        return yield* Effect.fail(
          new Error(Option.isSome(reply) ? `Login failed: ${errorOf(reply.value).message}` : "Login failed: network error"),
        )
    }
  })
}

const device: IntegrationOAuthMethodRegistration = {
  integrationID,
  method: { id: methodID, type: "oauth", label: "Anysola account" },
  authorize: () =>
    Effect.gen(function* () {
      const reply = yield* call("POST", "/v1/code/device/code", {
        body: { client: `anysola-code/${Anysola.version}`, deviceName: os.hostname() },
      })
      if (reply.status === 429) return yield* Effect.fail(new Error("Too many login attempts. Try again later."))
      if (reply.status < 200 || reply.status >= 300)
        return yield* Effect.fail(new Error(`Could not start login: ${errorOf(reply).message}`))
      const deviceCode = str(reply.body.device_code)
      const userCode = str(reply.body.user_code)
      // The server never pre-fills the code into the URL (phishing); the user types it.
      const url = str(reply.body.verification_uri)
      if (!deviceCode || !userCode || !url)
        return yield* Effect.fail(new Error("Could not start login: unexpected response from the server."))
      const interval = Math.max(1, num(reply.body.interval) ?? 5)
      const lifetime = Math.min(num(reply.body.expires_in) ?? 600, 900)
      const deadline = (yield* Clock.currentTimeMillis) + lifetime * 1000
      return {
        mode: "auto" as const,
        url,
        instructions: `Open ${url} and enter ${userCode}`,
        callback: poll(deviceCode, interval, deadline),
      }
    }),
  label: () => "Anysola account",
}

/** Decodes `/v1/code/config` into this integration's config document: its providers, default and title models. */
function document(body: unknown, connection: { readonly type: string; readonly id?: string; readonly name?: string }) {
  const normalized = ConfigNormalize.normalize(body)
  if (normalized.type === "rejected") return undefined
  const full = Option.getOrUndefined(
    Schema.decodeUnknownOption(ConfigSchema.Info, { errors: "all", onExcessProperty: "ignore" })(normalized.encoded),
  )
  if (!full) return undefined
  const providers = Object.fromEntries(
    Object.entries(full.providers ?? {}).filter(([id]) => Anysola.isAllowedProvider(id)),
  )
  const made = ConfigIntegration.make({
    integrationID,
    connection: (connection.type === "credential"
      ? { ...connection, id: Credential.ID.make(connection.id ?? "") }
      : connection) as Parameters<typeof ConfigIntegration.make>[0]["connection"],
    info: { providers },
  })
  return new ConfigIntegration.Document({
    type: "document",
    integrationID,
    connection: made.connection,
    info: new ConfigSchema.Info({
      providers: made.info.providers,
      model: full.model && Anysola.isAllowedProvider(full.model.providerID) ? full.model : undefined,
      agents: full.agents,
    }),
  })
}

const endpointKeys = ["baseURL", "apiKey", "authToken", "accessToken"]

type Endpoint = { package?: string; canonical?: string; settings: Record<string, unknown> }
const endpoint = (value: { package?: string; canonical?: string; settings?: Readonly<Record<string, unknown>> }) => ({
  package: value.package,
  canonical: value.canonical,
  settings: Object.fromEntries(Object.entries(value.settings ?? {}).filter(([key]) => endpointKeys.includes(key))),
})
/** Anysola's provider and model endpoints as the built-in plugin and the Anysola remote config left them. */
let trusted: { provider: Endpoint; models: Map<string, Endpoint> } | undefined

/** Sets the Anysola provider's endpoint, package, sign-in and gateway headers. */
function pin(providers: ProviderEditor) {
  providers.update(Anysola.providerID, (provider) => {
    provider.name = "Anysola"
    provider.integrationID = integrationID
    provider.package = "@opencode/ai/providers/anthropic-compatible"
    provider.settings = { ...provider.settings, baseURL: `${Anysola.apiURL()}/v1/code` }
    // x-anysola-version: Admin's versions panel follows updates, not the version a device signed in with.
    provider.headers = {
      ...provider.headers,
      "x-anysola-client": "opencode",
      "x-anysola-version": Anysola.version,
      [creditsEventHeader]: "1",
    }
  })
}

function restore<Value extends { package?: string; canonical?: string; settings?: Record<string, unknown> }>(
  value: Value,
  saved: Endpoint | undefined,
) {
  value.package = saved?.package
  value.canonical = saved?.canonical
  if (!value.settings && !saved) return
  value.settings = {
    ...Object.fromEntries(Object.entries(value.settings ?? {}).filter(([key]) => !endpointKeys.includes(key))),
    ...saved?.settings,
  }
}

/**
 * Runs last, after package plugins and config: drops every provider but Anysola and puts back the endpoints the
 * built-in plugin and the Anysola remote config set, so a plugin cannot add a model provider or point Anysola's
 * models at another server or package.
 */
export const AnysolaProviderLockPlugin = define({
  id: "anysola.provider-lock",
  effect: Effect.fn(function* (ctx) {
    yield* ctx.provider.transform((providers) => {
      for (const record of providers.list())
        if (!Anysola.isAllowedProvider(record.provider.id)) providers.remove(record.provider.id)
      const record = providers.get(Anysola.providerID)
      if (!record || !trusted) return
      const saved = trusted
      providers.update(Anysola.providerID, (provider) => {
        provider.integrationID = integrationID
        restore(provider as Parameters<typeof restore>[0], saved.provider)
      })
      pin(providers)
      for (const model of record.models.values())
        providers.models.update(Anysola.providerID, model.id, (item) =>
          restore(item as Parameters<typeof restore>[0], saved.models.get(model.id)),
        )
    })
  }),
})

export const AnysolaPlugin = define<Bus.Service | Config.Service | KV.Service | Scope.Scope>({
  id: "anysola.provider",
  effect: Effect.fn(function* (ctx) {
    const bus = yield* Bus.Service
    const config = yield* Config.Service
    const kv = yield* KV.Service
    const loading = Semaphore.makeUnsafe(1)
    let integrated: ConfigIntegration.Document | undefined
    type Active = Effect.Success<ReturnType<typeof ctx.integration.connection.active>>
    let current: { connection: Active; body: unknown } = {
      connection: undefined,
      body: undefined,
    }

    yield* ctx.integration.transform((editor) => {
      editor.update(Anysola.integrationID, (integration) => {
        integration.name = "Anysola"
      })
      editor.method.update(device)
      // CI and tests can supply a token without the device flow.
      editor.method.update({ integrationID, method: { type: "env", names: [Anysola.tokenEnv] } })
    })

    // The provider is always wired in; it becomes available once the Anysola integration has a connection.
    yield* ctx.provider.transform((providers) => {
      pin(providers)
      if (integrated) ConfigProviderPlugin.integrate(providers, integrated)
      const record = providers.get(Anysola.providerID)
      trusted = record && {
        provider: endpoint(record.provider),
        models: new Map([...record.models.values()].map((model) => [model.id, endpoint(model)])),
      }
    })

    const load = Effect.fn("AnysolaPlugin.load")(function* (): Effect.fn.Return<typeof current> {
      const connection = yield* ctx.integration.connection.active(Anysola.integrationID)
      if (!connection) return { connection, body: undefined }
      const status = (next: IntegrationConnection.Status | undefined) =>
        ctx.integration.connection.status({ integrationID: Anysola.integrationID, connection, status: next })
      const credential = yield* ctx.integration.connection.resolve(connection).pipe(Effect.orElseSucceed(() => undefined))
      const token = credential?.type === "oauth" ? credential.access : credential?.type === "key" ? credential.key : undefined
      const reply = token ? yield* call("GET", "/v1/code/config", { token }).pipe(Effect.option) : Option.none()
      if (Option.isSome(reply) && reply.value.status >= 200 && reply.value.status < 300) {
        yield* status(undefined)
        yield* kv.set(cacheKey, reply.value.body as never).pipe(Effect.ignore)
        return { connection, body: reply.value.body as unknown }
      }
      if (Option.isSome(reply) && reply.value.status === 401)
        yield* status({ status: "needs_auth", message: signedOut })
      yield* Effect.logWarning("failed to load Anysola config", {
        status: Option.isSome(reply) ? reply.value.status : "network",
      })
      // Offline or a transient error: keep the last config so the model list survives restarts.
      return { connection, body: (yield* kv.get(cacheKey).pipe(Effect.orElseSucceed(() => undefined))) as unknown }
    })

    const apply = Effect.fn("AnysolaPlugin.apply")(function* (next: typeof current) {
      current = next
      const doc = next.connection && next.body !== undefined ? document(next.body, next.connection) : undefined
      yield* config.integration?.(integrationID, doc) ?? Effect.void
      integrated = (yield* config.entries()).find(
        (entry): entry is ConfigIntegration.Document =>
          entry instanceof ConfigIntegration.Document && entry.integrationID === integrationID,
      )
      yield* ctx.provider.reload()
    })
    const refresh = () =>
      loading
        .withPermit(load().pipe(Effect.flatMap(apply)))
        .pipe(Effect.catchCause((cause) => Effect.logWarning("failed to refresh Anysola config", { cause })))

    yield* refresh()

    // Model lists and prices change server-side; re-fetch every 10 minutes while signed in.
    const polling = yield* FiberHandle.make<void, never>()
    const schedule = Effect.fnUntraced(function* () {
      if (!current.connection) return yield* FiberHandle.clear(polling)
      yield* FiberHandle.run(
        polling,
        Effect.sleep(Duration.minutes(10)).pipe(
          Effect.andThen(refresh()),
          Effect.forever,
        ),
        { onlyIfMissing: true },
      )
    })
    yield* bus.subscribe([Credential.Event.Switched, Credential.Event.Updated]).pipe(
      Stream.runForEach(() => refresh().pipe(Effect.andThen(schedule()))),
      Effect.forkScoped({ startImmediately: true }),
    )
    yield* schedule()
  }),
})
