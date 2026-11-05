#!/bin/sh
# Prepares the volumes shared with the Caddymium panel, then runs Caddy from
# the Caddyfile the panel maintains.
#
# The panel's plugin manager can drop a custom Caddy build into the shared
# bin volume (caddy-custom). When it exists and runs, it is used instead of
# the image's binary; the panel asks Caddy to exit after installing one, and
# the loop below restarts with it. A custom build that dies right after
# starting is set aside so the container never gets stuck.
set -u

CADDYFILE="${CADDYFILE:-/etc/caddyhub/Caddyfile}"
ADMIN="${CADDY_ADMIN_LISTEN:-caddy-admin:2019}"
PANEL_UID="${PANEL_UID:-1000}"
BIN_DIR=/caddy-bin
CUSTOM="$BIN_DIR/caddy-custom"

mkdir -p "$(dirname "$CADDYFILE")" /run/caddyhub/secrets /var/log/caddy
# The panel (non-root) writes the Caddyfile, secret files and custom builds.
chown "$PANEL_UID" "$(dirname "$CADDYFILE")" /run/caddyhub/secrets
chmod 0700 /run/caddyhub/secrets
chmod 0755 /var/log/caddy
if [ -d "$BIN_DIR" ]; then
	chown "$PANEL_UID" "$BIN_DIR"
	chmod 0755 "$BIN_DIR"
fi

# Before Caddy starts (and overwrites autosave.json), keep one copy of the
# config a previous Caddy left behind so Caddymium can offer to import it.
CFG_DIR="${XDG_CONFIG_HOME:-/config}/caddy"
mkdir -p "$CFG_DIR"
chmod 0755 "$CFG_DIR"
BACKUP="$CFG_DIR/autosave.pre-caddymium.json"
# Installs from before the rename keep their copy under the old name.
[ -e "$CFG_DIR/autosave.pre-caddyhub.json" ] && BACKUP="$CFG_DIR/autosave.pre-caddyhub.json"
if [ -s "$CFG_DIR/autosave.json" ] && [ ! -e "$BACKUP" ] \
	&& ! grep -q "$ADMIN" "$CFG_DIR/autosave.json"; then
	cp "$CFG_DIR/autosave.json" "$BACKUP"
	chmod 0644 "$BACKUP"
fi

# First start: a minimal config that only enables the admin API on the
# internal network, so the panel can take over.
if [ ! -s "$CADDYFILE" ]; then
	cat > "$CADDYFILE" <<CFG
{
	admin $ADMIN {
		origins $ADMIN
	}
}
CFG
	chown "$PANEL_UID" "$CADDYFILE"
fi

# Picks the binary to run: a working custom build, else the image's.
choose() {
	if [ -x "$CUSTOM" ] && "$CUSTOM" version >/dev/null 2>&1; then
		echo "$CUSTOM"
	else
		echo /usr/bin/caddy
	fi
}

# The panel kills a Caddy that stopped answering by creating caddy.kill in
# the shared bin volume; caddy.gen tells it when a new process started.
GEN_FILE="$BIN_DIR/caddy.gen"
KILL_FILE="$BIN_DIR/caddy.kill"
KILLED="/tmp/caddymium-killed"
gen=0
rm -f "$KILL_FILE" "$KILLED"
if [ -d "$BIN_DIR" ]; then
	(
		while :; do
			if [ -e "$KILL_FILE" ]; then
				rm -f "$KILL_FILE"
				pid=$(cat /tmp/caddymium-caddy.pid 2>/dev/null)
				if [ -n "$pid" ]; then
					echo "caddymium-entrypoint: the panel asked to stop an unresponsive Caddy; killing it" >&2
					touch "$KILLED"
					kill -KILL "$pid" 2>/dev/null
				fi
			fi
			sleep 1
		done
	) &
fi

child=""
term() {
	if [ -n "$child" ]; then
		kill -TERM "$child" 2>/dev/null
		wait "$child"
	fi
	exit 0
}
trap term TERM INT

while :; do
	BIN=$(choose)
	# Share the exact running binary so the panel validates with the same
	# build (and the same plugins) that serves traffic.
	if [ -d "$BIN_DIR" ]; then
		cp "$BIN" "$BIN_DIR/caddy.tmp" && mv -f "$BIN_DIR/caddy.tmp" "$BIN_DIR/caddy" && chmod 0755 "$BIN_DIR/caddy"
	fi
	echo "caddymium-entrypoint: starting $BIN ($("$BIN" version 2>/dev/null | cut -d' ' -f1))" >&2
	start=$(date +%s)
	"$BIN" run --config "$CADDYFILE" --adapter caddyfile &
	child=$!
	echo "$child" > /tmp/caddymium-caddy.pid
	gen=$((gen + 1))
	if [ -d "$BIN_DIR" ]; then
		echo "$gen" > "$GEN_FILE.tmp" && mv -f "$GEN_FILE.tmp" "$GEN_FILE"
	fi
	wait "$child"
	rc=$?
	child=""
	now=$(date +%s)
	killed=false
	if [ -e "$KILLED" ]; then
		killed=true
		rm -f "$KILLED"
	fi
	# A process killed on the panel's request is handled by the panel (it
	# picks the build to fall back to); only crashes are set aside here.
	if [ "$killed" = false ] && [ "$BIN" = "$CUSTOM" ] && [ "$rc" -ne 0 ] && [ $((now - start)) -lt 20 ]; then
		echo "caddymium-entrypoint: custom build exited with $rc within 20s; falling back to the image's Caddy" >&2
		mv -f "$CUSTOM" "$CUSTOM.broken"
	fi
	if [ "$rc" -ne 0 ]; then
		sleep 2
	fi
	echo "caddymium-entrypoint: caddy exited ($rc); restarting" >&2
done
