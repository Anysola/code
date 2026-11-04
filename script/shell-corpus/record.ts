// Bun test preload: records every input the fork's shell scanner tests pass to the scanner.
// RECORD_SCAN = a copy of the fork's shell/scan.ts without its self re-export line, RECORD_OUT = output file.
import { mock } from "bun:test"
import { appendFileSync } from "fs"

const original = await import(process.env.RECORD_SCAN!)
const fork = process.env.RECORD_FORK!
const record = (input: unknown, dialect: string) => {
  if (typeof input === "string") appendFileSync(process.env.RECORD_OUT!, JSON.stringify({ input, dialect }) + "\n")
}
const scan = (input: string, dialect = "posix") => {
  record(input, dialect)
  return original.scan(input, dialect)
}
const scanPowerShell = (input: string) => {
  record(input, "powershell")
  return original.scanPowerShell(input)
}
mock.module(`${fork}/packages/core/src/shell/scan.ts`, () => ({ ShellScan: { scan, scanPowerShell }, scan, scanPowerShell }))
