import { beforeEach, describe, expect } from "bun:test"
import { Effect } from "effect"
import { Anysola } from "@opencode/core/anysola"
import { Integration } from "@opencode/core/integration"
import { WebSearch } from "@opencode/core/websearch"
import { WebSearchAnysola } from "@opencode/core/plugin/websearch/anysola"
import { BILLING_URL } from "@opencode/core/plugin/provider/anysola-account"
import { host, integrationHost, webSearchHost } from "./host"
import { requests, resetWebSearchFixture, webSearchIntegrationTest } from "./websearch-fixture"

const it = webSearchIntegrationTest
const integrationID = Integration.ID.make(Anysola.integrationID)

beforeEach(() => resetWebSearchFixture(JSON.stringify({ results: [], credits: 1 })))

/** Loads the plugin, optionally signed in with an Anysola token, and returns the WebSearch service. */
const setup = (token?: string) =>
  Effect.gen(function* () {
    const integrations = yield* Integration.Service
    const websearch = yield* WebSearch.Service
    const integration = integrationHost(integrations)
    yield* integration.transform((editor) => {
      editor.update(Anysola.integrationID, (item) => (item.name = "Anysola"))
      editor.method.update({ integrationID: Anysola.integrationID, method: { type: "key" } })
    })
    if (token) yield* integrations.connection.key({ integrationID, key: token })
    yield* WebSearchAnysola.Plugin.effect(host({ integration, websearch: webSearchHost(websearch) }))
    return websearch
  })

/** The failure the provider returned, as the websearch tool receives it. */
const failure = (websearch: WebSearch.Interface) =>
  websearch.query({ query: "effect" }).pipe(
    Effect.flip,
    Effect.map((error) => (error instanceof WebSearch.RequestError ? error.cause : error)),
  )

describe("Anysola web search", () => {
  it.effect("is the only provider and the default selection", () =>
    Effect.gen(function* () {
      const websearch = yield* setup("asc_test")
      expect(yield* websearch.providers()).toEqual([{ id: WebSearch.ID.make("anysola"), name: "Anysola" }])
      expect(yield* websearch.default()).toEqual({ id: WebSearch.ID.make("anysola"), name: "Anysola" })
    }),
  )

  it.effect("posts the query with the account token and maps the results", () =>
    Effect.gen(function* () {
      resetWebSearchFixture(
        JSON.stringify({
          results: [
            { url: "https://effect.website", title: "Effect", content: "Effect documentation", published: 1785024000000 },
            { url: "https://example.com/bare" },
            { url: "https://example.com/nulls", title: null, content: null, published: null },
          ],
          credits: 2,
        }),
      )
      const websearch = yield* setup("asc_test")
      expect(yield* websearch.query({ query: "effect typescript" })).toEqual(
        new WebSearch.Response({
          providerID: WebSearch.ID.make("anysola"),
          results: [
            {
              url: "https://effect.website",
              title: "Effect",
              content: "Effect documentation",
              time: { published: 1785024000000 },
            },
            { url: "https://example.com/bare", time: {} },
            { url: "https://example.com/nulls", time: {} },
          ],
        }),
      )
      expect(requests).toEqual([
        {
          url: `${Anysola.apiURL()}/v1/code/search`,
          headers: expect.objectContaining({ authorization: "Bearer asc_test", "x-anysola-client": "opencode" }),
          body: { query: "effect typescript" },
        },
      ])
    }),
  )

  it.effect("asks to sign in without calling the gateway when there is no token", () =>
    Effect.gen(function* () {
      const websearch = yield* setup()
      expect(yield* failure(websearch)).toEqual(
        new WebSearchAnysola.Failure({ message: "Web search needs an Anysola sign-in. Run `anysola login`." }),
      )
      expect(requests).toEqual([])
    }),
  )

  it.effect("turns gateway errors into readable messages", () =>
    Effect.gen(function* () {
      const websearch = yield* setup("asc_test")
      const envelope = (type: string, message: string) => JSON.stringify({ type: "error", error: { type, message } })
      const cases = [
        { status: 401, body: envelope("authentication_error", "invalid token"), message: "Your Anysola sign-in has expired or was revoked. Run `anysola login`." },
        { status: 402, body: envelope("billing_error", "no credits"), message: `Out of Anysola credits for web search. Add credits at ${BILLING_URL}` },
        { status: 429, body: envelope("rate_limit_error", "slow down"), message: "Anysola web search is rate limited: slow down" },
        { status: 502, body: envelope("api_error", "search backend unavailable"), message: "Anysola web search failed (HTTP 502): search backend unavailable" },
        { status: 503, body: "<html>Bad gateway</html>", message: "Anysola web search failed (HTTP 503): HTTP 503" },
      ]
      for (const item of cases) {
        resetWebSearchFixture(item.body, item.status)
        expect(yield* failure(websearch)).toEqual(new WebSearchAnysola.Failure({ status: item.status, message: item.message }))
      }
    }),
  )

  it.effect("rejects a malformed success body", () =>
    Effect.gen(function* () {
      resetWebSearchFixture(JSON.stringify({ items: [] }))
      const websearch = yield* setup("asc_test")
      expect(yield* failure(websearch)).toEqual(
        new WebSearchAnysola.Failure({ message: "Anysola web search returned an unexpected response" }),
      )
    }),
  )
})
