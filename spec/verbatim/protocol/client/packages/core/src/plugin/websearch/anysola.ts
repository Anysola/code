export * as WebSearchAnysola from "./anysola.js"

import { define } from "@opencode/plugin/effect/plugin"
import { Duration, Effect, Option, Schema, Scope } from "effect"
import { HttpClient, HttpClientRequest } from "effect/http"
import { Anysola } from "../../anysola.js"
import { AnysolaSearchCredits } from "./anysola-credits.js"
import { collectBoundedResponseBody } from "../../tool/http-body.js"
import { BILLING_URL } from "../provider/anysola-account.js"
import { errorOf } from "../provider/anysola.js"

// Anysola Code's only web search provider: `POST /v1/code/search` on the Anysola gateway, signed in with the same
// token as the model calls and billed to the account's credits. It is the default selection, so the websearch tool
// never asks which provider to use; `"websearch": false` in the config still turns it off.

export const id = "anysola"
export const route = "/v1/code/search"
const MAX_RESPONSE_BYTES = 1024 * 1024
const TIMEOUT = Duration.seconds(30)

const SearchResponse = Schema.fromJsonString(
  Schema.Struct({
    results: Schema.Array(
      Schema.Struct({
        url: Schema.String,
        title: Schema.NullOr(Schema.String).pipe(Schema.optional),
        content: Schema.NullOr(Schema.String).pipe(Schema.optional),
        published: Schema.NullOr(Schema.Finite).pipe(Schema.optional),
      }),
    ),
    credits: Schema.NullOr(Schema.Finite).pipe(Schema.optional),
  }),
)

/** A failure whose message is written for the user; the websearch tool shows it as is. */
export class Failure extends Schema.TaggedError<Failure>()("WebSearchAnysola.Failure", {
  message: Schema.String,
  status: Schema.Number.pipe(Schema.optional),
}) {}

/** The gateway's reply as a readable failure: sign-in, credits, rate limit or the server's own message. */
export function failure(status: number, text: string) {
  const body = Option.getOrUndefined(
    Schema.decodeUnknownOption(Schema.fromJsonString(Schema.Record(Schema.String, Schema.Unknown)))(text),
  )
  const detail = errorOf({ status, body: body ?? {} }).message
  if (status === 401)
    return new Failure({ status, message: "Your Anysola sign-in has expired or was revoked. Run `anysola login`." })
  if (status === 402)
    return new Failure({ status, message: `Out of Anysola credits for web search. Add credits at ${BILLING_URL}` })
  if (status === 429) return new Failure({ status, message: `Anysola web search is rate limited: ${detail}` })
  return new Failure({ status, message: `Anysola web search failed (HTTP ${status}): ${detail}` })
}

/** Maps a successful reply to search results; `published` is milliseconds since the epoch. */
export function results(text: string, query?: string) {
  return Schema.decodeUnknownEffect(SearchResponse)(text).pipe(
    // The account plugin adds the charge to the Session that ran the websearch tool.
    Effect.tap((response) =>
      Effect.sync(() => {
        if (query !== undefined && response.credits) AnysolaSearchCredits.note(query, response.credits)
      }),
    ),
    Effect.map((response) =>
      response.results.map((item) => ({
        url: item.url,
        ...(item.title ? { title: item.title } : {}),
        ...(item.content ? { content: item.content } : {}),
        time: item.published == null ? {} : { published: item.published },
      })),
    ),
    Effect.mapError(() => new Failure({ message: "Anysola web search returned an unexpected response" })),
  )
}

export const Plugin = define<HttpClient.HttpClient | Scope.Scope>({
  id: "anysola.websearch",
  effect: Effect.fn("WebSearchAnysola.Plugin")(function* (ctx) {
    const http = yield* HttpClient.HttpClient
    const token = Effect.fn("WebSearchAnysola.token")(function* () {
      const connection = yield* ctx.integration.connection.active(Anysola.integrationID)
      if (!connection) return undefined
      const credential = yield* ctx.integration.connection.resolve(connection).pipe(Effect.orElseSucceed(() => undefined))
      return credential?.type === "oauth" ? credential.access : credential?.type === "key" ? credential.key : undefined
    })
    yield* ctx.websearch.transform((editor) => {
      editor.add({
        id,
        name: "Anysola",
        execute: (input) =>
          Effect.gen(function* () {
            const value = yield* token()
            if (!value) return yield* new Failure({ message: "Web search needs an Anysola sign-in. Run `anysola login`." })
            const request = HttpClientRequest.post(Anysola.apiURL() + route).pipe(
              HttpClientRequest.acceptJson,
              HttpClientRequest.setHeaders({
                authorization: `Bearer ${value}`,
                "x-anysola-client": "opencode",
                "x-anysola-version": Anysola.version,
              }),
              HttpClientRequest.bodyJsonUnsafe({ query: input.query }),
            )
            const reply = yield* HttpClient.withScope(http)
              .execute(request)
              .pipe(
                Effect.flatMap((response) =>
                  collectBoundedResponseBody(
                    response,
                    MAX_RESPONSE_BYTES,
                    () => new Failure({ message: "Anysola web search returned too much data" }),
                  ).pipe(Effect.map((body) => ({ status: response.status, text: body.toString("utf8") }))),
                ),
                Effect.scoped,
                Effect.timeoutOrElse({
                  duration: TIMEOUT,
                  orElse: () => Effect.fail(new Failure({ message: "Anysola web search timed out" })),
                }),
                Effect.catchIf(
                  (error) => !(error instanceof Failure),
                  () => Effect.fail(new Failure({ message: `Cannot reach ${Anysola.apiURL()}` })),
                ),
              )
            if (reply.status < 200 || reply.status >= 300) return yield* failure(reply.status, reply.text)
            return yield* results(reply.text, input.query)
          }),
      })
      editor.default.set(id)
    })
  }),
})
