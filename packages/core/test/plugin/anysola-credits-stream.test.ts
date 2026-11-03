import { describe, expect, test } from "bun:test"
import { creditsStream } from "../../src/plugin/provider/anysola-credits-stream"

const stop = 'event: message_stop\ndata: {"type":"message_stop"}\n\n'
const credits = 'event: anysola_credits\ndata: {"type":"anysola_credits","charged":7}\n\n'

async function pipe(chunks: string[]) {
  const seen: number[] = []
  const encoder = new TextEncoder()
  const source = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encoder.encode(chunk))
      controller.close()
    },
  })
  const text = await new Response(source.pipeThrough(creditsStream((value) => seen.push(value)))).text()
  return { text, seen }
}

describe("creditsStream", () => {
  test("reports the closing credits event and removes it", async () => {
    const start = 'event: message_start\ndata: {"type":"message_start"}\n\n'
    expect(await pipe([start, stop, credits])).toEqual({ text: start + stop, seen: [7] })
  })

  test("handles events split across chunks, multi-byte text included", async () => {
    const delta = 'event: content_block_delta\ndata: {"text":"çğü 🚀"}\n\n'
    const all = new TextEncoder().encode(delta + stop + credits)
    const seen: number[] = []
    const source = new ReadableStream<Uint8Array>({
      start(controller) {
        for (let i = 0; i < all.length; i += 5) controller.enqueue(all.slice(i, i + 5))
        controller.close()
      },
    })
    const text = await new Response(source.pipeThrough(creditsStream((value) => seen.push(value)))).text()
    expect(text).toBe(delta + stop)
    expect(seen).toEqual([7])
  })

  test("passes a stream without the event through unchanged", async () => {
    const keepalive = ": keepalive\n\n"
    expect(await pipe([keepalive, stop, "event: ping"])).toEqual({ text: keepalive + stop + "event: ping", seen: [] })
  })
})
