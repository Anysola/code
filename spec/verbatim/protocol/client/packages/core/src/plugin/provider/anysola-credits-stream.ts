// The gateway closes a streamed answer with `event: anysola_credits` (what the request was charged) when the
// request carries `x-anysola-credits-event: 1`. This stream reports that event and removes it, so the Anthropic
// parser only ever sees the Messages events.

export const creditsEventHeader = "x-anysola-credits-event"

const EVENT = "event: anysola_credits"

export function creditsStream(onCharged: (charged: number) => void) {
  const decoder = new TextDecoder()
  const encoder = new TextEncoder()
  let buffer = ""
  const read = (block: string) => {
    const data = block
      .split("\n")
      .find((line) => line.startsWith("data:"))
      ?.slice(5)
      .trim()
    try {
      const charged = (JSON.parse(data ?? "") as { charged?: unknown }).charged
      if (typeof charged === "number" && Number.isFinite(charged) && charged >= 0) onCharged(charged)
    } catch {}
  }
  // Complete events are passed on as they arrive; only the credits event is held back and dropped.
  const drain = (controller: TransformStreamDefaultController<Uint8Array>, final: boolean) => {
    let out = ""
    while (true) {
      const end = buffer.indexOf("\n\n")
      if (end === -1) break
      const block = buffer.slice(0, end + 2)
      buffer = buffer.slice(end + 2)
      if (block.replace(/\r/g, "").startsWith(EVENT)) read(block.replace(/\r/g, ""))
      else out += block
    }
    if (final && buffer) {
      if (!buffer.replace(/\r/g, "").startsWith(EVENT)) out += buffer
      buffer = ""
    }
    if (out) controller.enqueue(encoder.encode(out))
  }
  return new TransformStream<Uint8Array, Uint8Array>({
    transform(chunk, controller) {
      buffer += decoder.decode(chunk, { stream: true })
      drain(controller, false)
    },
    flush(controller) {
      buffer += decoder.decode()
      drain(controller, true)
    },
  })
}
