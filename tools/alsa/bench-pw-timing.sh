#!/bin/bash
# bench-pw-timing.sh — objective (silent) timing bench for the RME Babyface.
#
# Plays silence and records at the same time through PipeWire for a given
# duration, then reports the nodes' xrun counters (pw-top ERR) and the
# ALSA period/buffer the card really uses (hw_params).  Nothing to listen
# to; exit status is 0 only if every point was clean.
#
# Variants (temporary rule edits, restored byte-exact at exit):
#   current  the installed WirePlumber rule (IRQ; period left to PipeWire)
#   tsched   set api.alsa.disable-tsched=false (timer-based scheduling)
#
# A pinned-period variant is deliberately absent: forcing a quantum below
# the pinned period made the capture start fail intermittently on this
# card, so its numbers were not trustworthy.
#
# Examples:
#   tools/alsa/bench-pw-timing.sh --quantum 256,64,32
#   tools/alsa/bench-pw-timing.sh --quantum 32 --duration 600
#   tools/alsa/bench-pw-timing.sh --variant tsched --quantum 64 --duration 20
#
# Requires: pipewire, wireplumber, python3, pactl (pipewire-pulse).
set -u

RULE="${WIREPLUMBER_RME_RULE:-$HOME/.config/wireplumber/wireplumber.conf.d/51-rme.conf}"
VARIANT=current
QUANTA=
DUR=60
CSV=
RATE=48000
CH=12
LOOP_THRESHOLD=30
LOOP_TOTAL=100

usage() {
	cat <<'EOF'
Usage: bench-pw-timing.sh [options]

  --variant current|tsched         rule under test (default: current)
  --quantum Q[,Q...]               graph quanta to force (default: clock.quantum)
  --duration S                     seconds per point (default: 60)
  --csv FILE                       append one CSV row per point
  -h, --help                       this help

The rule file is backed up and restored byte-exact at exit;
clock.force-quantum is reset to its previous value.
EOF
}

while [ $# -gt 0 ]; do
	case "$1" in
		--variant) VARIANT=${2:-}; shift 2 ;;
		--quantum) QUANTA=${2:-}; shift 2 ;;
		--duration) DUR=${2:-}; shift 2 ;;
		--csv) CSV=${2:-}; shift 2 ;;
		-h|--help) usage; exit 0 ;;
		*) echo "error: unknown option '$1'" >&2; usage >&2; exit 2 ;;
	esac
done

case "$VARIANT" in
	current|tsched) ;;
	*) echo "error: --variant must be current or tsched" >&2; exit 2 ;;
esac
case "$DUR" in
	''|*[!0-9]*) echo "error: --duration must be an integer" >&2; exit 2 ;;
esac
[ "$DUR" -ge 2 ] || { echo "error: --duration must be >= 2" >&2; exit 2; }

if [ -z "$QUANTA" ]; then
	QUANTA=$(pw-metadata -n settings 0 | sed -n "s/.*key:'clock.quantum' value:'\([0-9]*\)'.*/\1/p")
fi
[ -n "$QUANTA" ] || QUANTA=64
QUANTA=$(printf '%s' "$QUANTA" | tr ',' ' ')
for q in $QUANTA; do
	case "$q" in
		''|*[!0-9]*) echo "error: bad quantum '$q'" >&2; exit 2 ;;
	esac
	[ "$q" -ge 2 ] || { echo "error: quantum $q is too small" >&2; exit 2; }
done

CARD=
for c in /proc/asound/card*; do
	[ -e "$c/id" ] || continue
	if [ "$(cat "$c/id" 2>/dev/null)" = BabyfacePro ]; then
		CARD=${c##*card}
		break
	fi
done
[ -n "$CARD" ] || { echo "error: BabyfacePro card not found in /proc/asound" >&2; exit 1; }
HWPB=/proc/asound/card${CARD}/pcm0p/sub0/hw_params
HWCAP=/proc/asound/card${CARD}/pcm0c/sub0/hw_params

SINK=
SRC=
detect_nodes() {
	read -r SINK SRC <<<"$(pw-dump 2>/dev/null | python3 -c '
import json, sys
sink = src = ""
for o in json.load(sys.stdin):
    if o.get("type") != "PipeWire:Interface:Node":
        continue
    n = o.get("info", {}).get("props", {}).get("node.name", "")
    if not sink and n.startswith("alsa_output.usb-RME"):
        sink = n
    if not src and n.startswith("alsa_input.usb-RME"):
        src = n
print(sink, src)
')"
}

state_of() { # $1 = node.name -> state, empty if absent
	pw-dump 2>/dev/null | python3 -c '
import json, sys
name = sys.argv[1]
states = []
for o in json.load(sys.stdin):
    if o.get("type") != "PipeWire:Interface:Node":
        continue
    if o.get("info", {}).get("props", {}).get("node.name") == name:
        states.append(o["info"].get("state", ""))
if "running" in states:
    print("running")
elif states:
    print(states[-1])
' "$1"
}

prop_of() { # $1 node name, $2 prop -> value or NA
	pw-dump 2>/dev/null | python3 -c '
import json, sys
name, key = sys.argv[1], sys.argv[2]
for o in json.load(sys.stdin):
    if o.get("type") != "PipeWire:Interface:Node":
        continue
    p = o.get("info", {}).get("props", {})
    if p.get("node.name") == name:
        print(p.get(key, "NA"))
        break
' "$1" "$2"
}

stats() { # one pw-top pass -> "err_sink err_src"
	timeout 6 pw-top -b -n 2 2>/dev/null | awk -v s="$SINK" -v c="$SRC" '
		$NF == s { es = $9 }
		$NF == c { ec = $9 }
		END { printf "%s %s\n", (es == "" ? "NA" : es), (ec == "" ? "NA" : ec) }
	'
}

card_params() { # one read per stream, so the fields come from one snapshot
	local pb cap pp bp pc bc
	pb=$(cat "$HWPB" 2>/dev/null)
	cap=$(cat "$HWCAP" 2>/dev/null)
	pp=$(printf '%s\n' "$pb" | awk '/period_size/ {print $2}')
	bp=$(printf '%s\n' "$pb" | awk '/buffer_size/ {print $2}')
	pc=$(printf '%s\n' "$cap" | awk '/period_size/ {print $2}')
	bc=$(printf '%s\n' "$cap" | awk '/buffer_size/ {print $2}')
	echo "${pp:-NA} ${bp:-NA} ${pc:-NA} ${bc:-NA}"
}

delta() { # $1 current, $2 baseline
	if [ "$1" = NA ] || [ "$2" = NA ]; then echo NA; else echo $(( $1 - $2 )); fi
}

ensure_nodes() {
	for _ in 1 2 3 4 5; do
		detect_nodes
		[ -n "$SINK" ] && [ -n "$SRC" ] && return 0
		sleep 2
	done
	echo "-- nodes missing, restarting WirePlumber" >&2
	systemctl --user restart wireplumber
	for _ in 1 2 3 4 5 6 7 8; do
		detect_nodes
		[ -n "$SINK" ] && [ -n "$SRC" ] && return 0
		sleep 2
	done
	return 1
}

ensure_ready() { # nodes present and settled before a full-duplex point
	ensure_nodes || return 1
	sleep 10
	ensure_nodes || return 1
}

make_variant() { # $1 variant, $2 output
	python3 - "$RULE" "$1" "$2" <<'PY'
import sys
src_path, variant, out_path = sys.argv[1], sys.argv[2], sys.argv[3]
out = []
for line in open(src_path).read().splitlines():
    stripped = line.lstrip()
    if stripped.startswith("#"):
        out.append(line)
        continue
    if variant == "tsched":
        if "api.alsa.period-size" in line or "api.alsa.period-num" in line:
            continue
        if "api.alsa.disable-tsched" in line:
            line = line.replace("true", "false")
    out.append(line)
open(out_path, "w").write("\n".join(out) + "\n")
PY
}

ERRDIR=$(mktemp -d)
BACKUP=$(mktemp)
TMP_RULE=$(mktemp)
RULE_CHANGED=no
cp -p "$RULE" "$BACKUP"
SHA_BEFORE=$(sha256sum < "$RULE" | cut -d' ' -f1)
FQ_BEFORE=$(pw-metadata -n settings 0 | sed -n "s/.*key:'clock.force-quantum' value:'\([0-9]*\)'.*/\1/p")

cleanup() {
	trap - EXIT
	pkill -f 'pw-cat -p --raw' 2>/dev/null
	pkill -f 'pw-record --raw' 2>/dev/null
	wait 2>/dev/null
	if [ "$RULE_CHANGED" = yes ]; then
		cp -p "$BACKUP" "$RULE"
		systemctl --user restart wireplumber
		sleep 4
	fi
	pw-metadata -n settings 0 clock.force-quantum "${FQ_BEFORE:-0}" >/dev/null 2>&1
	if [ "$RULE_CHANGED" = yes ]; then
		if [ "$(sha256sum < "$RULE" | cut -d' ' -f1)" = "$SHA_BEFORE" ]; then
			echo "== rule restored byte-exact" >&2
		else
			echo "== WARNING: rule differs from the original" >&2
		fi
	fi
	rm -rf "$ERRDIR" "$BACKUP" "$TMP_RULE"
}
trap cleanup EXIT
trap 'exit 143' TERM INT HUP

if ! ensure_ready; then
	echo "error: RME nodes not usable (sink='${SINK}' source='${SRC}')" >&2
	exit 1
fi

echo "== rule: $RULE"
echo "== card=$CARD sink=$SINK"
echo "== source=$SRC variant=$VARIANT duration=${DUR}s"

if [ "$VARIANT" != current ]; then
	make_variant "$VARIANT" "$TMP_RULE"
	cp "$TMP_RULE" "$RULE"
	RULE_CHANGED=yes
	systemctl --user restart wireplumber
	sleep 5
	if ! ensure_ready; then
		echo "error: RME nodes not usable after the $VARIANT rule change" >&2
		exit 1
	fi
	echo "== live props: disable-tsched=$(prop_of "$SINK" api.alsa.disable-tsched) period-size=$(prop_of "$SINK" api.alsa.period-size) period-num=$(prop_of "$SINK" api.alsa.period-num)"
	if [ "$VARIANT" = tsched ] && [ "$(prop_of "$SINK" api.alsa.disable-tsched)" != "False" ]; then
		echo "error: the node did not switch to tsched; refusing to report a misleading run" >&2
		exit 1
	fi
fi

[ -n "$CSV" ] && [ ! -f "$CSV" ] && printf 'variant,quantum,duration,period_pb,buffer_pb,period_cap,buffer_cap,xruns_sink,xruns_src,verdict\n' > "$CSV"

run_point() { # $1 quantum, $2 retry
	local q=$1 retry=${2:-yes}
	local e0s e0c ees eec es ec ds dc prev_s prev_c
	local per_pb buf_pb per_c buf_c ss sc
	local warm rem step loop=no reset=no xr_s xr_c v
	local verdict="" fpp fbp fpc fbc

	echo "-- quantum=$q duration=${DUR}s" >&2
	pw-metadata -n settings 0 clock.force-quantum "$q" >/dev/null
	sleep 1
	read -r e0s e0c <<<"$(stats)"

	timeout $((DUR + 20)) pw-cat -p --raw --format s32 --rate "$RATE" --channels "$CH" --target "$SINK" /dev/zero >/dev/null 2>"$ERRDIR/pb_$q.err" &
	timeout $((DUR + 20)) pw-record --raw --format s32 --rate "$RATE" --channels "$CH" --target "$SRC" /dev/null >/dev/null 2>"$ERRDIR/rec_$q.err" &

	warm=8
	[ "$DUR" -lt 8 ] && warm=$DUR
	sleep "$warm"
	read -r ees eec <<<"$(stats)"
	read -r per_pb buf_pb per_c buf_c <<<"$(card_params)"
	ss=$(state_of "$SINK")
	sc=$(state_of "$SRC")

	if [ "$ss" != running ] || [ "$sc" != running ]; then
		echo "-- nodes not running (sink=${ss:-absent} src=${sc:-absent})" >&2
		pkill -f 'pw-cat -p --raw' 2>/dev/null
		pkill -f 'pw-record --raw' 2>/dev/null
		wait 2>/dev/null
		if [ "$retry" = yes ]; then
			echo "-- retrying once after a WirePlumber restart" >&2
			systemctl --user restart wireplumber
			sleep 5
			ensure_ready
			run_point "$q" no
			return $?
		fi
		xr_s=NA
		xr_c=NA
		verdict=NODE-NOT-RUNNING
	else
		ds=$(delta "$ees" "$e0s")
		dc=$(delta "$eec" "$e0c")
		if { [ "$ds" != NA ] && [ "$ds" -gt "$LOOP_THRESHOLD" ]; } || \
		   { [ "$dc" != NA ] && [ "$dc" -gt "$LOOP_THRESHOLD" ]; }; then
			loop=yes
		fi
		if [ "$loop" = yes ]; then
			echo "-- STOP/START LOOP after ~${warm}s, aborting this point" >&2
			xr_s=$ds
			xr_c=$dc
			verdict="STOP/START LOOP"
			sleep 1
		else
			es=$ees
			ec=$eec
			prev_s=$ees
			prev_c=$eec
			rem=$(( DUR - warm ))
			while [ "$rem" -gt 0 ]; do
				step=$(( rem > 60 ? 60 : rem ))
				sleep "$step"
				read -r es ec <<<"$(stats)"
				rem=$(( rem - step ))
				if [ "$DUR" -ge 120 ]; then
					echo "   t=$(( DUR - rem ))s +$(delta "$es" "$prev_s")/+$(delta "$ec" "$prev_c") total $(delta "$es" "$e0s")/$(delta "$ec" "$e0c")" >&2
				fi
				if [ "$es" = NA ] || [ "$ec" = NA ]; then
					verdict=NODE-NOT-RUNNING
					break
				fi
				prev_s=$es
				prev_c=$ec
			done
			if [ "$verdict" != NODE-NOT-RUNNING ]; then
				xr_s=$(delta "$es" "$e0s")
				xr_c=$(delta "$ec" "$e0c")
				for v in "$xr_s" "$xr_c"; do
					if [ "$v" != NA ] && [ "$v" -lt 0 ]; then reset=yes; fi
				done
				if [ "$xr_s" = NA ] || [ "$xr_c" = NA ]; then
					verdict=NO-DATA
				elif [ "$reset" = yes ]; then
					verdict=NODE-RESET
				elif { [ "$xr_s" != NA ] && [ "$xr_s" -gt "$LOOP_TOTAL" ]; } || \
				     { [ "$xr_c" != NA ] && [ "$xr_c" -gt "$LOOP_TOTAL" ]; }; then
					verdict="STOP/START LOOP"
				elif { [ "$xr_s" != NA ] && [ "$xr_s" -gt 0 ]; } || \
				     { [ "$xr_c" != NA ] && [ "$xr_c" -gt 0 ]; }; then
					verdict=XRUNS
				else
					verdict=CLEAN
				fi
			fi
			# the streams have been up for DUR now: prefer these
			# values, the capture may have opened late
			read -r fpp fbp fpc fbc <<<"$(card_params)"
			[ "$fpp" != NA ] && per_pb=$fpp
			[ "$fbp" != NA ] && buf_pb=$fbp
			[ "$fpc" != NA ] && per_c=$fpc
			[ "$fbc" != NA ] && buf_c=$fbc
		fi
	fi

	pkill -f 'pw-cat -p --raw' 2>/dev/null
	pkill -f 'pw-record --raw' 2>/dev/null
	wait 2>/dev/null
	sleep 1

	printf 'variant=%s quantum=%s period=%s/%s buffer=%s/%s xruns=%s/%s verdict=%s\n' \
		"$VARIANT" "$q" "$per_pb" "$per_c" "$buf_pb" "$buf_c" "$xr_s" "$xr_c" "$verdict"
	if [ -n "$CSV" ]; then
		printf '%s,%s,%s,%s,%s,%s,%s,%s,%s,%s\n' \
			"$VARIANT" "$q" "$DUR" "$per_pb" "$buf_pb" "$per_c" "$buf_c" "$xr_s" "$xr_c" "$verdict" >> "$CSV"
	fi
	[ "$verdict" = CLEAN ]
}

RC=0
for q in $QUANTA; do
	run_point "$q" || RC=1
done
exit "$RC"
