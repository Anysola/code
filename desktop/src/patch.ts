/** A unified diff (the core's `createTwoFilesPatch` output) as lines the diff view draws. */
export type DiffLine = {
  kind: "ctx" | "add" | "del" | "hunk"
  text: string
  old?: number
  new?: number
}

const HUNK = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/

export function parsePatch(patch: string): DiffLine[] {
  const out: DiffLine[] = []
  let oldLine = 0
  let newLine = 0
  let inHunk = false
  for (const raw of patch.split("\n")) {
    const hunk = HUNK.exec(raw)
    if (hunk) {
      oldLine = Number(hunk[1])
      newLine = Number(hunk[2])
      inHunk = true
      out.push({ kind: "hunk", text: raw })
      continue
    }
    // The header (Index:, ===, ---, +++) comes before the first hunk.
    if (!inHunk || raw.startsWith("\\")) continue
    const body = raw.slice(1)
    if (raw.startsWith("+")) out.push({ kind: "add", text: body, new: newLine++ })
    else if (raw.startsWith("-")) out.push({ kind: "del", text: body, old: oldLine++ })
    else if (raw.startsWith(" ")) out.push({ kind: "ctx", text: body, old: oldLine++, new: newLine++ })
  }
  return out
}
