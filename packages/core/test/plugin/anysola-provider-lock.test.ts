import { describe, expect } from "bun:test"
import { Effect } from "effect"
import { Anysola } from "@opencode/core/anysola"
import { Model } from "@opencode/core/model"
import { Plugin } from "@opencode/core/plugin"
import { PluginHost } from "@opencode/core/plugin/host"
import { AnysolaPlugin, AnysolaProviderLockPlugin } from "@opencode/core/plugin/provider/anysola"
import { Provider } from "@opencode/core/provider"
import { testEffect } from "../lib/effect"
import { PluginTestLayer } from "./fixture"

const it = testEffect(PluginTestLayer)
const anysola = Provider.ID.make(Anysola.providerID)

describe("AnysolaProviderLockPlugin", () => {
  it.effect("drops other providers and puts back the endpoints a plugin changed", () =>
    Effect.gen(function* () {
      const host = yield* PluginHost.make(yield* Plugin.Service)
      const providers = yield* Provider.Service
      // Stands in for a model from the Anysola remote config, which the built-in plugin applies before the capture.
      yield* host.provider.transform((editor) =>
        editor.models.update(Anysola.providerID, "routed", (model) => {
          model.package = "@opencode/ai/providers/openai-compatible"
          model.settings = { baseURL: `${Anysola.apiURL()}/v1/code/openai` }
        }),
      )
      yield* AnysolaPlugin.effect(host)
      // A package plugin, which runs between the built-in plugins and the lock.
      yield* host.provider.transform((editor) => {
        editor.update("other", (provider) => {
          provider.package = "@opencode/ai/providers/openai"
        })
        editor.models.update("other", "model", () => {})
        editor.update(Anysola.providerID, (provider) => {
          provider.package = "some-npm-provider"
          provider.integrationID = "other"
          provider.settings = { ...provider.settings, baseURL: "https://example.com/v1", timeout: 5 }
        })
        editor.models.update(Anysola.providerID, "routed", (model) => {
          model.settings = { ...model.settings, baseURL: "https://example.com/routed" }
        })
        editor.models.update(Anysola.providerID, "added", (model) => {
          model.package = "some-npm-provider"
          model.settings = { apiKey: "secret", temperature: 1 }
        })
      })
      yield* AnysolaProviderLockPlugin.effect(host)

      expect((yield* providers.all()).map((provider) => provider.id)).toEqual([anysola])
      const provider = yield* providers.get(anysola)
      expect(provider).toMatchObject({
        integrationID: Anysola.integrationID,
        package: "@opencode/ai/providers/anthropic-compatible",
        settings: { baseURL: `${Anysola.apiURL()}/v1/code`, timeout: 5 },
        headers: { "x-anysola-client": "opencode" },
      })
      const record = (yield* providers.snapshot()).records.get(anysola)
      const routed = record?.models.get(Model.ID.make("routed"))
      expect(routed?.package).toBe("@opencode/ai/providers/openai-compatible")
      expect(routed?.settings).toEqual({ baseURL: `${Anysola.apiURL()}/v1/code/openai` })
      const added = record?.models.get(Model.ID.make("added"))
      expect(added).toBeDefined()
      expect(added?.package).toBeUndefined()
      expect(added?.settings).toEqual({ temperature: 1 })
    }),
  )
})
