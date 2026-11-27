#!/bin/sh
# Installs the hourly sweep of leftover *.rcgu.o files on this Mac (LIO-676).
#
# Usage: scripts/install-sweep-rcgu.sh [MAIN_CHECKOUT]   (default: this checkout's main worktree)
#
# Copies scripts/sweep-rcgu.py to ~/.local/bin/sweep-rcgu, so the job doesn't
# depend on what branch a checkout has out, and loads a launchd job that runs
# it with --delete over every checkout of the repository: once at load, then
# every hour, at background priority. Each run appends a dated report to
# ~/Library/Logs/sweep-rcgu.log. Running it again updates the copy and reloads
# the job. To remove it:
#   launchctl bootout gui/$(id -u)/local.sweep-rcgu
#   rm ~/Library/LaunchAgents/local.sweep-rcgu.plist ~/.local/bin/sweep-rcgu
set -eu

here=$(cd "$(dirname "$0")" && pwd)
main=${1:-$(git -C "$here" worktree list --porcelain | sed -n '1s/^worktree //p')}
[ -d "$main/.git" ] || { echo "not a main checkout: $main" >&2; exit 1; }

label=local.sweep-rcgu
bin="$HOME/.local/bin/sweep-rcgu"
plist="$HOME/Library/LaunchAgents/$label.plist"
log="$HOME/Library/Logs/sweep-rcgu.log"

mkdir -p "$HOME/.local/bin" "$HOME/Library/LaunchAgents" "$HOME/Library/Logs"
install -m 755 "$here/sweep-rcgu.py" "$bin"

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
    <string>date; exec /usr/bin/python3 -I "\$0" --delete --worktrees "\$1"</string>
    <string>$bin</string>
    <string>$main</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict><key>PATH</key><string>/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin</string></dict>
  <key>RunAtLoad</key><true/>
  <key>StartInterval</key><integer>3600</integer>
  <key>ProcessType</key><string>Background</string>
  <key>LowPriorityIO</key><true/>
  <key>Nice</key><integer>10</integer>
  <key>StandardOutPath</key><string>$log</string>
  <key>StandardErrorPath</key><string>$log</string>
</dict>
</plist>
EOF
plutil -lint "$plist" >/dev/null

launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
launchctl bootstrap "gui/$(id -u)" "$plist"
echo "installed $label: $bin hourly over the checkouts of $main; log $log"
