#!/usr/bin/env node
// Optional speed-up after `npm install`: the `anysola` launcher works without it (newer npm skips
// install scripts unless allowed). It finds the platform binary once and, on Linux and macOS, puts
// it in place of the Node launcher so starts skip Node. It never fails the install.

import childProcess from "node:child_process"
import fs from "node:fs"
import path from "node:path"
import { createRequire } from "node:module"

const require = createRequire(import.meta.url)
const resolve = require("./resolve.cjs")

// Windows on ARM64 with x64 Node gets only the x64 packages from npm; fetch the native one beside.
function installNative(name) {
  const npm = process.env.npm_execpath?.endsWith("npm-cli.js") ? [process.execPath, process.env.npm_execpath] : ["npm"]
  const result = childProcess.spawnSync(
    npm[0],
    [
      ...npm.slice(1),
      "install",
      "--ignore-scripts",
      "--no-save",
      "--loglevel=error",
      // npm rejects a package built for another CPU, such as arm64 under emulated x64 Node, unless forced.
      ...(name.includes(`-${resolve.platform}-${resolve.nodeArch}`) ? [] : ["--force"]),
      "--prefix",
      resolve.nativeDir,
      `${name}@${resolve.dependencies[name]}`,
    ],
    { stdio: "inherit", windowsHide: true, shell: process.platform === "win32" && npm[0] === "npm" },
  )
  return result.status === 0
}

function main() {
  let binary = resolve.find()
  if (!binary || (resolve.platform === "windows" && resolve.arch !== resolve.nodeArch)) {
    const native = resolve.packageNames()[0]
    if (installNative(native)) binary = resolve.find() ?? binary
  }
  if (!binary) return
  resolve.remember(binary)
  if (resolve.platform === "windows") return
  // npm links the command to this file; the binary in its place runs without Node.
  const launcher = path.join(resolve.directory, "bin", "anysola")
  const staged = `${launcher}.native`
  try {
    fs.rmSync(staged, { force: true })
    try {
      fs.linkSync(binary, staged)
    } catch {
      fs.copyFileSync(binary, staged)
    }
    fs.chmodSync(staged, 0o755)
    if (!resolve.works(staged)) throw new Error("copied binary does not run")
    fs.renameSync(staged, launcher)
  } catch {
    fs.rmSync(staged, { force: true })
  }
}

try {
  main()
} catch (error) {
  console.warn(`Anysola Code: ${error instanceof Error ? error.message : String(error)}`)
}
