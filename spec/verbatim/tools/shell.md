# shell

Source: packages/core/src/tool/plugin/shell.ts, packages/core/src/shell.ts, packages/core/src/shell/{select,parse,scan,result}.ts, packages/core/src/config/plugin/shell.ts, packages/core/src/job.ts, packages/core/src/session.ts (background), packages/util/src/cross-spawn-spawner.ts, packages/cli/src/run/noninteractive.ts

Captured from the 0.5.1 binary's `/v1/code/messages` request (build agent, anysola/claude-sonnet-5-5, Linux). Ground truth: `_captured-request-build-sonnet.json`.

## Description (verbatim, as sent)

````text
Execute a shell command and return its output. Commands run on Linux using bash. Quote file paths containing spaces or special characters. Prefer dedicated tools over shell commands when possible. When output is large, the full result is saved to a file and a truncated preview is returned. Rely on automatic truncation unless filtering the output is more useful. Commands accept an optional timeout, background commands have no timeout by default. Background commands return immediately, and you will be notified when they complete.
````

## input_schema (verbatim, as sent)

```json
{
  "type": "object",
  "properties": {
    "command": {
      "type": "string",
      "description": "Shell command string to execute"
    },
    "workdir": {
      "type": "string",
      "description": "Working directory to execute the command in. Defaults to the current working directory. When possible, avoid changing directories in the command and set the working directory here instead."
    },
    "timeout": {
      "type": "integer",
      "minimum": 0,
      "description": "Timeout in milliseconds. Set to 0 to disable the timeout. Defaults to 120000 for foreground commands. Background commands have no timeout by default."
    },
    "background": {
      "type": "boolean",
      "description": "Run the command in the background and return immediately (useful for dev servers and long-running builds). You do not need to use '&' at the end of the command when using this parameter. You will be notified when it completes. DO NOT poll for completion."
    }
  },
  "required": [
    "command"
  ],
  "additionalProperties": false
}
```

## Description variants

Built by `description(shell?)` (tool/plugin/shell.ts:34-44): the sentences below joined with a single space. At registration the tool has NO shell clause (shell.ts:190 `description()`); a session `context` / `compaction` / `generate` hook (shell.ts:277-285) replaces it on every request with `description(ShellSelect.name(<compat shell>))`, so the model always sees the clause.

```text
Execute a shell command and return its output. Commands run on ${OS} using ${shell}. Quote file paths containing spaces or special characters. Prefer dedicated tools over shell commands when possible. When output is large, the full result is saved to a file and a truncated preview is returned. Rely on automatic truncation unless filtering the output is more useful. Commands accept an optional timeout, background commands have no timeout by default. Background commands return immediately, and you will be notified when they complete.
```

`${OS}` (shell.ts:26-33): `process.platform` darwin -> `macOS`, win32 -> `Windows`, linux -> `Linux`, anything else -> the raw platform string (e.g. `freebsd`).

`${shell}` = `ShellSelect.name(path)` (select.ts:141-144): lower-cased basename on Unix (`bash`, `zsh`, `sh`, `dash`, `ksh`); on Windows the lower-cased file stem without extension (`pwsh`, `powershell`, `bash`, `cmd`).

Shell chosen with priority "compat" (select.ts:179-189, 116-139, 95-108):
1. Config `shell` (string, config key `shell`, latest config entry wins; config/plugin/shell.ts:16) if set and not `fish`/`nu` (META deny) and it resolves to an existing executable (absolute path must be a file; bare name via PATH lookup).
2. Else, if `OPENCODE_GIT_BASH_PATH` was given (Windows option `gitbash`, cli/src/server-process.ts:137 -> server/src/routes.ts:143): `$SHELL` via the same rule.
3. Else `$SHELL` if set, not fish/nu, and executable (cached per process).
4. Fallback: Windows -> first existing of `pwsh` (PATH), `powershell` (PATH), Git Bash (`OPENCODE_GIT_BASH_PATH`, else `<dir of git.exe>/../../bin/bash.exe` if non-empty), `%COMSPEC%` or `cmd.exe`. macOS -> `/bin/zsh`. Other Unix -> `bash` from PATH, else `/bin/sh`.
Windows path fixups (select.ts:63-72): a POSIX-style `/usr/bin/bash` style path named bash is replaced by Git Bash; `FSUtil.windowsPath` maps `/c/...`, `/c:/...`, `/cygdrive/c/...`, `/mnt/c/...` to `C:/...`.

Captured variant (Linux, SHELL=/bin/bash): `Commands run on Linux using bash.` Other reachable texts: `Commands run on macOS using zsh.`, `Commands run on Linux using zsh.` (SHELL=zsh), `Commands run on Linux using sh.`, `Commands run on Windows using pwsh.`, `Commands run on Windows using powershell.`, `Commands run on Windows using bash.`, `Commands run on Windows using cmd.`.

## Model-visible output templates

Content is a list of text blocks; on the Anthropic wire a single block is sent as a plain string `content`, several as a list (observed).

Normal completion (shell.ts:77-80, shell/result.ts:30-34): block 1 = captured output (or `(no output)` when empty, shell.ts:259); block 2 only when there is a notice:
- `Timed out before completion`
- `Killed by ${signal}` (e.g. `Killed by SIGTERM`)
- `Exited with code ${exit}` (non-zero only; exit 0 has no notice)

Captured:
```
'hi\nerr\n'                                  # echo hi; echo err 1>&2  (stdout+stderr merged, in arrival order)
[ "ls: cannot access '/nonexistent_zz': No such file or directory\n", "Exited with code 3" ]
'(no output)'                                # true
```

Timeout (shell.ts:223-225): output gets `\n\nCommand exceeded timeout of ${finalTimeout} ms. Retry with a larger timeout if the command is expected to take longer.` appended, plus notice block. Captured (timeout 500, `echo start; sleep 3; echo end`):
```
[ "start\n\n\nCommand exceeded timeout of 500 ms. Retry with a larger timeout if the command is expected to take longer.", "Timed out before completion" ]
```

Truncation (shell.ts:244-259; tail is kept, NOT head): limits = config `tool_output.max_lines` (default 2000) / `tool_output.max_bytes` (default 51200). The last `max_bytes` bytes of the output file are read (may start mid-line / mid-UTF-8 char); if that page has more than `max_lines` lines only the last `max_lines` are kept (trailing newline dropped by the join). Notice appended to the text:
```
\n\n[showing lines ${total - shown + 1}-${total} of ${total}; full output saved to ${file}]
```
`total` = newline count of the whole output (+1 if it does not end with `\n`), `shown` = min(lines in page, max_lines). Captured (`seq 1 100000`): text ends `...99999\n100000\n\n[showing lines 98001-100000 of 100000; full output saved to /…/home/.local/share/opencode/shell/global/sh_1a6167ba20011tHVqwHNcM9lyV.out]`. Captured byte-limit case (120000 `a` + `\ntail\n`): 51374 chars, ending `aaaa\ntail\n\n\n[showing lines 1-2 of 2; full output saved to …/shell/global/sh_….out]` (line numbers are wrong when the byte cut is inside the first kept line; reproduce as is). Shell results always carry `metadata.truncated`, so the generic tool-output truncation (tool-output.ts:70) never re-truncates them.
Full output file: `<XDG_DATA_HOME>/opencode/shell/<projectID>/<shellID>.out` (`projectID` is `global` outside a git repo with commits); shellID `sh_` + ascending id; files deleted after 7 days (hourly sweep, shell.ts:33,91-119); in-memory record of only the last 25 exited commands (shell.ts:32).

Background start (shell.ts:94-99, 24-25): two blocks:
```
Command moved to the background (shell ID: ${shellID}).
Output is streaming to: ${file}
```
```
You will be notified automatically when the command finishes. The notification will include the command's output. Unless the user explicitly asks otherwise, DO NOT poll for completion, even if you need the final result to continue. Repeatedly sleeping and reading or searching the output file is polling, not useful work. You may read the current output if it lets you do useful work now, but do not repeatedly check it while waiting for the command to finish. Keep working on anything that does not depend on the result. If you have nothing else to do, end your response; you will be resumed automatically when the command finishes.
```

Background completion notification (shell.ts:151-183, shell/result.ts:45-63): a synthetic user message (wakes the session), text:
```
<shell id="${shellID}" state="${state}" command="${command}">
${text}
</shell>
```
`state` = `completed` | `cancelled` | `error`; `text` = output blocks joined by `\n\n` (completed), the job error or `Command failed` (error), `Cancelled` (cancelled). The command attribute is not escaped. Captured:
```
<shell id="sh_1a616ce160017IYc3YZFxACWiB" state="completed" command="echo started; sleep 1; echo finished; exit 4">
started
finished


Exited with code 4
</shell>
```

User pressing ctrl+b (TUI `session.background`) on a running foreground command: the tool returns the same background-start result, the command's timeout is cleared (shell.ts:257-261), and this synthetic message is added (session.ts:511-523):
```
User requested that active blocking work be moved to the background.

Backgrounded work:
- shell: ${command}

The backgrounded work is still unfinished. Move on to other work if you can. If there is nothing else useful to do, finish your response. Do not wait, sleep, poll, or report the backgrounded work as complete until a later completion notification is added to the conversation.
```

User-run shell (`!cmd` in the TUI, session area) is reported as (shell/result.ts:65-81): `The following shell command was executed by the user:\n<shell id="…" state="completed|cancelled" command="…">\n<output>[\n\n<notice or Cancelled or "Exited with code unknown">]\n</shell>`.

Failures (is_error true). The session serializes a failed tool as JSON (observed; serialization is session-area): `{"error":{"type":"<type>","message":"<message>"},"content":[]}`. Shell failure messages:
- `Working directory does not exist: ${absolute}` (shell.ts:143)
- `Working directory is not a directory: ${absolute}` (shell.ts:147)
- `Portable shell scanner cannot analyze command: ${reason}` (shell/parse.ts:172), reason one of `command-substitution`, `compound-command`, `dynamic-command-name`, `dynamic-execution`, `heredoc`, `invalid-redirect`, `invalid-structure`, `unterminated-escape`, `unterminated-quote` (shell/scan.ts:3-12). The command is NOT run.
- `Command cancelled` (shell.ts:264), job error text / `Command failed` (shell.ts:263)
- ToolFailure wrapper message `Unable to execute command: ${command}` (shell.ts:270) — not what the model sees; the inner error message is (observed `"type":"unknown"`).
- Permission rejected in `anysola run` without `--auto`: `{"error":{"type":"permission.rejected","message":"This non-interactive run cannot ask the user for permission, so the request was rejected. Continue without this action."},"content":[]}` (cli/src/run/noninteractive.ts:73-74).
- Input validation (tool/runtime.ts:99): `Invalid arguments for tool "shell":\n- <path>: <issue>\n\nArguments provided:\n<json>\n\nUpdate the arguments and call the tool again.`

## Permission resources

Order of checks in `prepare` (shell.ts:114-149), before spawning:
1. `workdir` (or session directory) resolved with `~` expansion, relative to the session directory; `cwd` target plus every directory named by a static `cd`-like command go through `external_directory` (one batched ask). Resource = `<dir>/*` (forward slashes), save = `<project root containing dir, else dir>/*` (file-access.ts:107-121). Paths inside the session directory or its project worktree need nothing.
2. Action `shell`, resources = every simple command's source text, save = arity prefix + ` *` (one entry per command, same order). Skipped entirely when the command list is empty.
3. Working directory existence check (after approval).
`run` stderr line when asking without `--auto` (noninteractive.ts:175): `! permission requested: ${action} (${resources.join(", ")}); auto-rejecting`; with `--auto` the reply is `once` and nothing is printed. Captured: `! permission requested: shell (git status --short); auto-rejecting`, `! permission requested: external_directory (/tmp/*); auto-rejecting`.

Scanner: hand-written (no tree-sitter), shell/scan.ts. Dialect: `bash` / `zsh` by shell name, PowerShell scanner for pwsh/powershell, everything else (sh, dash, ksh, cmd!) `posix`. Input over 64 KiB (65536 chars) -> opaque `invalid-structure`; substitution depth max 32.
Per command (shell/parse.ts:175-248):
- declarations (`declare typeset export readonly local unset unsetenv` with assignments) are not resources; their nested substitutions still are.
- command/process substitutions, pipelines, lists, subshells, `if`/`for`/`while`/function bodies are flattened into their simple commands (control words like `true` in `if true` count).
- leading `VAR=value` assignments stay in the resource but not in the save prefix (`FOO=1 bun test` -> save `bun test *`).
- redirects stay in the resource; a commandless redirect (`> f.txt`) saves `> *`.
- `cd chdir pushd popd push-location set-location` (also behind `builtin`/`command`): static operands become directories for the external check and the cd is not a resource; if any operand is dynamic (`$HOME`, `-`), the cd stays a resource (`cd $HOME` -> save `cd *`). `~`, `~/x`, `~+`, `~user` expanded; PowerShell `$env:X`, `$HOME`, `$PWD`, `$PSHOME` expanded.
Probed (bun against the fork source, shell `/bin/bash`, cwd `/proj`):
```
echo hi                         -> [{"resource":"echo hi","save":"echo *"}]
git status --short              -> [{"resource":"git status --short","save":"git status *"}]
npm run build -- --x            -> [{"resource":"npm run build -- --x","save":"npm run build *"}]
ls -la /tmp && cd /tmp && rm -rf foo -> ls -la /tmp | ls * ; rm -rf foo | rm * ; directories ["/tmp"]
for i in 1 2; do echo $i; done  -> [{"resource":"echo $i","save":"echo *"}]
cat <<EOF\nx\nEOF               -> [{"resource":"cat <<EOF\nx\nEOF","save":"cat *"}]
echo $(date)                    -> echo $(date) | echo * ; date | date *
x=$(pwd)                        -> [{"resource":"pwd","save":"pwd *"}]
export A=1                      -> [] (no shell ask)
cd ~/x                          -> [] ; directories ["<home>/x"]
cd $HOME                        -> [{"resource":"cd $HOME","save":"cd *"}]
(cd /tmp; ls)                   -> ls | ls * ; directories ["/tmp"]
ls | grep a > out.txt           -> ls | ls * ; grep a > out.txt | grep *
> f.txt                         -> [{"resource":"> f.txt","save":"> *"}]
if true; then echo a; fi        -> true | true * ; echo a | echo *
bash -c "rm -rf /"              -> save "bash *"
sudo rm -rf /x                  -> save "sudo *"
git -C /tmp status              -> save "git -C *"
docker compose up -d            -> save "docker compose up *"
f() { ls; }                     -> ls | ls *
$CMD arg                        -> save "$CMD *"
echo "unterminated              -> opaque unterminated-quote (tool fails)
pwsh: Get-ChildItem -Path C:\x  -> save "Get-ChildItem *"
pwsh: cd ..; git status         -> git status | git status * ; directories [".."]
pwsh: Set-Location -LiteralPath /tmp -> [] ; directories ["/tmp"]
```

Save prefix = `prefix(words[0..3])` (parse.ts:605-611): longest key of ARITY matching the first 1..3 words, take that many words; no match -> first word only. ARITY table verbatim (parse.ts:23-161):
```json
{"cat":1,"cd":1,"chmod":1,"chown":1,"cp":1,"echo":1,"env":1,"export":1,"grep":1,"kill":1,"killall":1,"ln":1,"ls":1,"mkdir":1,"mv":1,"ps":1,"pwd":1,"rm":1,"rmdir":1,"sleep":1,"source":1,"tail":1,"touch":1,"unset":1,"which":1,
"aws":3,"az":3,"bazel":2,"brew":2,"bun":2,"bun run":3,"bun x":3,"cargo":2,"cargo add":3,"cargo run":3,"cdk":2,"cf":2,"cmake":2,"composer":2,"consul":2,"consul kv":3,"crictl":2,"deno":2,"deno task":3,"doctl":3,"docker":2,"docker builder":3,"docker compose":3,"docker container":3,"docker image":3,"docker network":3,"docker volume":3,"eksctl":2,"eksctl create":3,"firebase":2,"flyctl":2,"gcloud":3,"gh":3,"git":2,"git config":3,"git remote":3,"git stash":3,"go":2,"gradle":2,"helm":2,"heroku":2,"hugo":2,"ip":2,"ip addr":3,"ip link":3,"ip netns":3,"ip route":3,"kind":2,"kind create":3,"kubectl":2,"kubectl kustomize":3,"kubectl rollout":3,"kustomize":2,"make":2,"mc":2,"mc admin":3,"minikube":2,"mongosh":2,"mysql":2,"mvn":2,"ng":2,"npm":2,"npm exec":3,"npm init":3,"npm run":3,"npm view":3,"npx":2,"nvm":2,"nx":2,"openssl":2,"openssl req":3,"openssl x509":3,"pip":2,"pipenv":2,"pnpm":2,"pnpm dlx":3,"pnpm exec":3,"pnpm run":3,"poetry":2,"podman":2,"podman container":3,"podman image":3,"psql":2,"pulumi":2,"pulumi stack":3,"python":2,"pyenv":2,"rake":2,"rbenv":2,"redis-cli":2,"rustup":2,"serverless":2,"sfdx":3,"skaffold":2,"sls":2,"sst":2,"swift":2,"systemctl":2,"terraform":2,"terraform workspace":3,"tmux":2,"turbo":2,"ufw":2,"vault":2,"vault auth":3,"vault kv":3,"vercel":2,"volta":2,"wp":2,"yarn":2,"yarn dlx":3,"yarn run":3}
```
Other verbatim scanner lists (shell/scan.ts:31-87): redirects `&>> &> <<< <<- << <> <& >& >| >> > <`; bash list operators `&& || |&`; zsh adds `&! &|`; binding builtins `declare typeset local export readonly private unset read getln vared getopts mapfile readarray compgen zparseopts zregexparse zformat zstyle zstat stat`; precommands bash `builtin command time coproc !`, zsh `noglob nocorrect exec -`; CWD commands (parse.ts:12) `cd chdir popd pushd push-location set-location`. PowerShell save keeps the original source spacing up to the prefix end (parse.ts:231-243). Matching of saved/configured rules against resources uses the wildcard matcher (`*` glob over the whole resource text; util/wildcard.ts).
