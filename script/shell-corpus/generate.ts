// Runs the fork's ShellParse.scan over recorded scanner inputs and writes the expected results.
// Run with exactly HOME=/home/tester USER=tester PROJECTS=/srv/projects in the environment (the
// home directory is read at startup), as script/shell-corpus.sh does.
// Usage: bun generate.ts <fork> <recorded.jsonl> <out.jsonl>
const [fork, recorded, out] = process.argv.slice(2)
const { ShellParse } = await import(`${fork}/packages/core/src/shell/parse.ts`)
const { Effect, Exit } = await import(`${fork}/packages/core/node_modules/effect/dist/index.js`)
const shells: Record<string, string> = { bash: "/bin/bash", zsh: "/bin/zsh", posix: "/bin/sh", powershell: "/usr/bin/pwsh" }
const seen = new Set<string>()
const rows: string[] = []
for (const line of (await Bun.file(recorded).text()).split("\n").filter(Boolean)) {
  const { input, dialect } = JSON.parse(line)
  // Every bash-family input is checked in all three of those dialects.
  for (const shell of dialect === "powershell" ? ["powershell"] : ["bash", "zsh", "posix"]) {
    const key = `${shell}\0${input}`
    if (seen.has(key)) continue
    seen.add(key)
    // `~user` resolves through the machine's user database, which differs between hosts.
    if (/~[A-Za-z_]/.test(input)) continue
    const exit = await Effect.runPromiseExit(ShellParse.scan(input, shells[shell], "/work/project"))
    const row: Record<string, unknown> = { shell, command: input }
    if (Exit.isSuccess(exit)) {
      row.commands = exit.value.commands.map((c: { resource: string; save: string }) => [c.resource, c.save])
      row.directories = exit.value.directories
    } else {
      const message = String((exit.cause as any).reasons?.[0]?.error?.message ?? exit.cause)
      row.error = message.replace("Portable shell scanner cannot analyze command: ", "")
    }
    rows.push(JSON.stringify(row))
  }
}
rows.sort()
await Bun.write(out, rows.join("\n") + "\n")
console.log(`${rows.length} rows`)
