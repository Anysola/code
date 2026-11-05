// Finds the platform binary of @anysola/code: shared by the `anysola` launcher and the postinstall.

const childProcess = require("node:child_process")
const fs = require("node:fs")
const os = require("node:os")
const path = require("node:path")

const directory = __dirname
const packageJson = JSON.parse(fs.readFileSync(path.join(directory, "package.json"), "utf8"))
const dependencies = packageJson.optionalDependencies ?? {}
const platform = { darwin: "darwin", linux: "linux", win32: "windows" }[os.platform()] ?? os.platform()
const nodeArch = { x64: "x64", arm64: "arm64", arm: "arm" }[os.arch()] ?? os.arch()
// x64 Node runs emulated on Windows ARM64, but PROCESSOR_IDENTIFIER still names the host CPU.
const arch =
  platform === "windows" && nodeArch === "x64" && process.env.PROCESSOR_IDENTIFIER?.startsWith("ARM")
    ? "arm64"
    : nodeArch
const executable = platform === "windows" ? "anysola.exe" : "anysola"
// The resolved binary, remembered so later starts skip the search.
const cacheFile = path.join(directory, "bin", ".binary")
// Where the postinstall puts a platform package npm did not install (another CPU's build).
const nativeDir = path.join(directory, "native")

function isMusl() {
  if (platform !== "linux") return false
  try {
    if (fs.existsSync("/etc/alpine-release")) return true
    const result = childProcess.spawnSync("ldd", ["--version"], { encoding: "utf8" })
    return `${result.stdout || ""}${result.stderr || ""}`.toLowerCase().includes("musl")
  } catch {
    return false
  }
}

/// Platform package names to try, best first. The AVX2 build comes before the baseline one; a CPU
/// without AVX2 fails the `--version` check and falls through to baseline.
function packageNames() {
  const base = Object.keys(dependencies).find((name) => name.endsWith(`-${platform}-${arch}`))
  if (!base) throw new Error(`Anysola Code does not provide a binary for ${platform}-${arch}`)
  const names =
    platform === "linux" && isMusl()
      ? [`${base}-musl`, `${base}-baseline-musl`, base, `${base}-baseline`]
      : platform === "linux"
        ? [base, `${base}-baseline`, `${base}-musl`, `${base}-baseline-musl`]
        : [base, `${base}-baseline`]
  // Windows 11 on ARM64 can still run the x64 binary under emulation when the native one fails.
  const x64 = base.replace(/-arm64$/, "-x64")
  const emulated = platform === "windows" && arch === "arm64" ? [x64, `${x64}-baseline`] : []
  return [...names, ...emulated].filter((name) => dependencies[name])
}

/// The binary of an installed platform package, when it is the version this launcher expects.
function packageBinary(name) {
  for (const root of [undefined, nativeDir]) {
    let packagePath
    try {
      packagePath = require.resolve(`${name}/package.json`, root ? { paths: [root] } : undefined)
    } catch {
      continue
    }
    // Package managers keep an older platform package when the new version is not on the registry
    // yet, which would silently run the previous release under the new launcher.
    const version = JSON.parse(fs.readFileSync(packagePath, "utf8")).version
    if (version !== dependencies[name]) continue
    const binary = path.join(path.dirname(packagePath), "bin", executable)
    if (fs.existsSync(binary)) return binary
  }
  return undefined
}

function works(binary) {
  return childProcess.spawnSync(binary, ["--version"], { stdio: "ignore", windowsHide: true }).status === 0
}

/// The first installed platform binary that runs here, or undefined.
function find() {
  for (const name of packageNames()) {
    const binary = packageBinary(name)
    if (binary && works(binary)) return binary
  }
  return undefined
}

function remember(binary) {
  try {
    fs.writeFileSync(cacheFile, binary)
  } catch {}
}

function remembered() {
  try {
    const binary = fs.readFileSync(cacheFile, "utf8").trim()
    return binary && fs.existsSync(binary) ? binary : undefined
  } catch {
    return undefined
  }
}

function forget() {
  try {
    fs.unlinkSync(cacheFile)
  } catch {}
}

module.exports = {
  arch,
  dependencies,
  directory,
  executable,
  find,
  forget,
  nativeDir,
  nodeArch,
  packageNames,
  platform,
  remember,
  remembered,
  works,
}
