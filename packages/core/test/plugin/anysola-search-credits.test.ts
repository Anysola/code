import { describe, expect, test } from "bun:test"
import { AnysolaSearchCredits } from "../../src/plugin/websearch/anysola-credits"
import { results } from "../../src/plugin/websearch/anysola"
import { Effect } from "effect"

describe("AnysolaSearchCredits", () => {
  test("hands each search's credits over once, oldest first", () => {
    AnysolaSearchCredits.note("bun version", 2)
    AnysolaSearchCredits.note("bun version", 3)
    expect(AnysolaSearchCredits.take("bun version")).toBe(2)
    expect(AnysolaSearchCredits.take("bun version")).toBe(3)
    expect(AnysolaSearchCredits.take("bun version")).toBeUndefined()
  })

  test("the provider notes the charge of a successful reply under its query", async () => {
    const text = JSON.stringify({ results: [{ url: "https://bun.com", title: "Bun" }], credits: 2 })
    const mapped = await Effect.runPromise(results(text, "latest bun"))
    expect(mapped).toEqual([{ url: "https://bun.com", title: "Bun", time: {} }])
    expect(AnysolaSearchCredits.take("latest bun")).toBe(2)
  })
})
