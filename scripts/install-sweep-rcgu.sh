#!/bin/sh
# Installs the hourly sweep of leftover *.rcgu.o files on this Mac (LIO-676).
#
# Usage: scripts/install-sweep-rcgu.sh [MAIN_CHECKOUT [WORKTREES_DIR]]
#   (defaults: this checkout's main worktree, ~/.t3/worktrees/<its folder name>)
#
# Copies scripts/sweep-rcgu.py to ~/.local/bin/sweep-rcgu, so the job doesn't
# depend on what branch a checkout has out, and loads a launchd job that runs
# it with --delete over every checkout of the repository: once at load, then
# every hour, at background priority. macOS doesn't let the job read
# ~/Documents, so it also lists WORKTREES_DIR itself, where T3 keeps the
# worktrees, rather than only asking git for them. Each run appends a dated report to
# ~/Library/Logs/sweep-rcgu.log. Running it again updates the copy and reloads
# the job. To remove it:
#   launchctl bootout gui/$(id -u)/local.sweep-rcgu
#   rm ~/Library/LaunchAgents/local.sweep-rcgu.plist ~/.local/bin/sweep-rcgu
set -eu

here=$(cd "$(dirname "$0")" && pwd)
main=${1:-$(git -C "$here" worktree list --porcelain | sed -n '1s/^worktree //p')}
[ -d "$main/.git" ] || { echo "not a main checkout: $main" >&2; exit 1; }
worktrees=${2:-$HOME/.t3/worktrees/$(basename "$main")}

label=local.sweep-rcgu
bin="$HOME/.local/bin/sweep-rcgu"
plist="$HOME/Library/LaunchAgents/$label.plist"
log="$HOME/Library/Logs/sweep-rcgu.log"

mkdir -p "$HOME/.local/bin" "$HOME/Library/LaunchAgents" "$HOME/Library/Logs"
install -m 755 "$here/sweep-rcgu.py" "$bin"

xml() { printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'; }

cat > "$plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$label</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/sh</string>
    <string>-c</string>
    <string>date; exec /usr/bin/python3 -I -u "\$0" --delete --worktrees "\$1" --in "\$2"</string>
    <string>$(xml "$bin")</string>
    <string>$(xml "$main")</string>
    <string>$(xml "$worktrees")</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict><key>PATH</key><string>/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin</string></dict>
  <key>RunAtLoad</key><true/>
  <key>StartInterval</key><integer>3600</integer>
  <key>ProcessType</key><string>Background</string>
  <key>LowPriorityIO</key><true/>
  <key>Nice</key><integer>10</integer>
  <key>StandardOutPath</key><string>$(xml "$log")</string>
  <key>StandardErrorPath</key><string>$(xml "$log")</string>
</dict>
</plist>
EOF
plutil -lint "$plist" >/dev/null

launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" "$plist"
echo "installed $label: $bin hourly over the checkouts of $main and the folders in $worktrees; log $log"
