export * as AnysolaRpc from "./anysola.js"

import { Schema } from "effect"
import { Rpc } from "./rpc.js"

/** A proxied Anysola API reply: `body` is the JSON the endpoint returned, error envelopes included. */
export const Reply = Schema.Struct({
  ok: Schema.Boolean,
  status: Schema.Number,
  body: Schema.Unknown,
})
export type Reply = typeof Reply.Type

/** Credits a Session has spent, from the gateway's `x-credits-*` response headers. */
export const SessionCredits = Schema.Struct({
  sessionID: Schema.String,
  /** Balance after the latest request; null until the gateway reported one. */
  remaining: Schema.NullOr(Schema.Number),
  /** Credits of the turn in progress, or of the last finished turn while idle. */
  turn: Schema.Number,
  /** Credits of the whole Session in this server process. */
  total: Schema.Number,
  /** Finished turns that spent credits. */
  turns: Schema.Number,
})
export type SessionCredits = typeof SessionCredits.Type

const standard = Schema.toStandardSchemaV1

/**
 * The Anysola account RPC served by the built-in Anysola plugin. Every call uses the signed-in token on the
 * server; clients never see it. `credits` fires after every charged request and when a turn ends.
 */
export const Definition = Rpc.define({
  id: "anysola.account",
  methods: {
    session: {
      input: standard(Schema.Struct({ sessionID: Schema.String })),
      output: standard(Schema.NullOr(SessionCredits)),
    },
    me: { input: standard(Schema.Struct({})), output: standard(Reply) },
    usage: { input: standard(Schema.Struct({ days: Schema.Number })), output: standard(Reply) },
    tokens: { input: standard(Schema.Struct({})), output: standard(Reply) },
    revoke: { input: standard(Schema.Struct({ id: Schema.String })), output: standard(Reply) },
    /** `credits_per_turn_estimate` per model ID, from the remote config; empty when the server sends none. */
    estimates: {
      input: standard(Schema.Struct({})),
      output: standard(Schema.Record(Schema.String, Schema.Number)),
    },
  },
  events: {
    credits: { schema: standard(SessionCredits) },
  },
})
