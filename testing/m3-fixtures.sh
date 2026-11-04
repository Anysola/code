#!/usr/bin/env bash
# Builds the 0.5.x home behind crates/migrate/tests/fixtures/rich with the 0.5.1 binary, then captures the
# requests 0.5.1 sends when it continues those sessions. Usage: testing/m3-fixtures.sh <fixture dir>
# Needs testing/bin/anysola-0.5.1 and the two fake gateways under testing/runs/ of the main checkout.
set -euo pipefail
OUT=$(realpath "$1")
R=/home/ubuntu/anysola-code/rust
W=$R/testing/runs/m3-fix
BIN=$R/testing/bin/anysola-0.5.1
RICH=18841
LOG=18842
rm -rf "$W" && mkdir -p "$W/logs"
cleanup() { kill $(jobs -p) 2>/dev/null || true; pkill -f "$W/home.*serve --service" 2>/dev/null || true; }
trap cleanup EXIT
PORT=$RICH node $R/testing/runs/storage/rich-gateway.mjs >/dev/null &
PORT=$LOG LOGDIR=$W/logs node $R/testing/runs/session/logger.mjs >/dev/null &
sleep 1
source $R/testing/env.sh "$W/home"
P=$HOME/proj
mkdir -p "$P" && cd "$P"
git init -q && git remote add origin https://github.com/anysola/m3-fixture.git
printf '# Fixture\n' >README.md && printf 'attached note\n' >note.txt
cp $R/testing/runs/storage/rich/home/proj/pixel.png .
git add -A && git -c user.name=t -c user.email=t@t commit -qm init

oc() { timeout 120 "$BIN" "$@" </dev/null >>"$W/out.txt" 2>&1; }
ANYSOLA_API_URL=http://127.0.0.1:$RICH oc login --standalone
export ANYSOLA_TOKEN=asc_FIXTURE
ANYSOLA_API_URL=http://127.0.0.1:$RICH oc run --standalone --auto "RICHTEST please do the scripted steps"
A=$(sqlite3 "$XDG_DATA_HOME/opencode/opencode.db" "select id from session_v2 where parent_id is null")
ANYSOLA_API_URL=http://127.0.0.1:$RICH oc run --standalone -s "$A" --fork -m anysola/claude-sonnet-5-5#high \
  -f pixel.png -f note.txt "third turn with files"
ANYSOLA_API_URL=http://127.0.0.1:$LOG oc run --standalone "OVERFLOW please"
ANYSOLA_API_URL=http://127.0.0.1:$LOG oc run --standalone -c "after the checkpoint"
rm -f "$W"/logs/*.json

# What only the TUI writes, in the shapes 0.5.1 writes them.
DB=$XDG_DATA_HOME/opencode/opencode.db
PID=$(sqlite3 "$DB" "select id from project")
sqlite3 "$DB" "insert into kv values ('anysola:budget:default', '25', 1793787700000, 1793787700000);
  insert into kv values ('anysola:session:ses_orphan', '{\"total\":3,\"turns\":1,\"budget\":null}', 1793787700000, 1793787700000);
  insert into permission values ('psv_fixture', '$PID', 'shell', 'git status *', 1793787700000, 1793787700000);"
cat >"$XDG_CONFIG_HOME/opencode/cli.json" <<'EOF'
{
  "$schema": "https://opencode.ai/v2/cli.json",
  // edited by hand
  "theme": { "name": "tokyonight", "mode": "dark" },
  "keybinds": { "session.sidebar.toggle": "<leader>s" },
  "session": { "thinking": "show" }
}
EOF
S=$XDG_STATE_HOME/opencode
printf '{"text":"RICHTEST please do the scripted steps","files":[],"agents":[],"skills":[],"pasted":[],"mode":"normal"}\n' >"$S/prompt-history.jsonl"
printf '{"prompt":{"text":"stashed draft","files":[],"agents":[],"skills":[],"pasted":[],"mode":"normal"},"timestamp":1793787700000}\n' >"$S/prompt-stash.jsonl"
printf '{"recent":[{"providerID":"anysola","modelID":"claude-sonnet-5-5"}],"favorite":[],"variant":{"anysola/claude-sonnet-5-5":"high"}}\n' >"$S/model.json"
mkdir -p "$S/latest/tui"
printf '{"global":{"tabs":[],"unread":{}},"cwd":{"%s":{"tabs":[{"sessionID":"%s","title":"Rich storage test"}],"unread":{}}}}\n' "$P" "$A" >"$S/latest/tui/tabs.json"

cp -a "$HOME" "$W/pristine"
restore() { rm -rf "$HOME" && cp -a "$W/pristine" "$HOME"; }
# The main (tool-carrying) request of one continuation, without the host header.
capture() {
  local name=$1; shift
  (cd "$P" && ANYSOLA_API_URL=http://127.0.0.1:$LOG oc run --standalone "$@")
  node -e '
    const fs = require("fs"), dir = process.argv[1]
    const main = fs.readdirSync(dir).filter((f) => f.endsWith("POST_v1_code_messages.json")).sort()
      .map((f) => JSON.parse(fs.readFileSync(`${dir}/${f}`))).find((r) => r.body.tools?.length)
    delete main.headers.host
    process.stdout.write(JSON.stringify(main, null, 2) + "\n")' "$W/logs" >"$OUT/capture-$name.json"
  rm -f "$W"/logs/*.json
}
F=$(sqlite3 "$DB" "select id from session_v2 where fork_session_id is not null")
restore && capture continue-id -s "$A" "continue imported"
restore && capture continue-last -c "after compaction"
restore && sqlite3 "$DB" <"$OUT/synthetic.sql" && capture continue-synthetic -s "$F" "continue synthetic"

# The fixture: the db as SQL without the models.dev cache, home as @HOME@, the shadow repo by project id.
restore
sqlite3 "$DB" "delete from kv where key like 'models-dev:%'"
sqlite3 "$DB" .dump | sed "s#$HOME#@HOME@#g" >"$OUT/opencode.sql"
rm -rf "$OUT/home" && mkdir -p "$OUT/home/snapshot"
cp "$XDG_CONFIG_HOME/opencode/cli.json" "$OUT/home/cli.json"
cp -a "$S" "$OUT/home/state" && rm -rf "$OUT/home/state/locks" "$OUT/home/state/latest/locks"
sed -i "s#$HOME#@HOME@#g" "$OUT/home/state/latest/tui/tabs.json"
cp -a "$XDG_DATA_HOME/opencode/snapshot/$PID"/* "$OUT/home/snapshot/$PID"
cp "$P/pixel.png" "$OUT/home/pixel.png"
rm -rf "$OUT/home/snapshot/$PID"/{index,hooks,description,branches,logs}
sed -i "s#$HOME#@HOME@#g" "$OUT/home/snapshot/$PID/config" "$OUT/home/snapshot/$PID/objects/info/alternates"
echo "fixture: $OUT"
