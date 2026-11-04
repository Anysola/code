# Isolated paths for testing the Anysola Code binary. Usage: source env.sh [home-dir]
H=${1:-/home/ubuntu/anysola-code/build/home}
mkdir -p "$H"
export HOME="$H" XDG_CONFIG_HOME="$H/.config" XDG_DATA_HOME="$H/.local/share" XDG_CACHE_HOME="$H/.cache" XDG_STATE_HOME="$H/.local/state"
export OPENCODE_DISABLE_AUTOUPDATE=1
