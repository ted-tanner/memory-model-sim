#!/usr/bin/env bash
set -e

cd "$(dirname "$0")"

TRIALS="${TRIALS:-1000000}"
RESULTS_DIR="${RESULTS_DIR:-results}"
JOBS="${JOBS:-8}"
POLL_INTERVAL="${POLL_INTERVAL:-1}"
BIN="target/release/memory-model-sim"
PIDS=()

running_jobs() {
	jobs -pr | wc -l | tr -d ' '
}

queue_exp() {
	name="$1"
	attack="$2"
	mode="$3"
	model="$4"
	domains="$5"
	seed="$6"
	control="${7:-none}"

	echo "starting $name"
	"$BIN" experiment \
		--attack="$attack" \
		--mode="$mode" \
		--memory-model="$model" \
		--domains="$domains" \
		--control="$control" \
		--trials="$TRIALS" \
		--seed="$seed" \
		--out="$RESULTS_DIR/$name" &
	PIDS+=("$!")

	while [ "$(running_jobs)" -ge "$JOBS" ]; do
		sleep "$POLL_INTERVAL"
	done
}

wait_all() {
	for pid in "${PIDS[@]}"; do
		wait "$pid"
	done
	PIDS=()
}

./build-riscv-programs.sh
cargo build --release

while read -r name attack mode model domains seed control; do
	case "$name" in
		""|\#*) continue ;;
	esac
	queue_exp "$name" "$attack" "$mode" "$model" "$domains" "$seed" "${control:-none}"
done <<'EXPERIMENTS'
default-binary-timesliced binary-pp time-sliced default different 10 none
backcache-binary-timesliced binary-pp time-sliced backcache different 10 none
newcache-binary-timesliced binary-pp time-sliced newcache different 10 none
smtcache-binary-smt-different binary-pp smt smtcache different 10 none
smtcache-binary-smt-same binary-pp smt smtcache same 10 none
default-pp-timesliced prime-probe time-sliced default different 11 none
backcache-pp-timesliced prime-probe time-sliced backcache different 11 none
newcache-pp-timesliced prime-probe time-sliced newcache different 11 none
smtcache-pp-smt-different prime-probe smt smtcache different 11 none
smtcache-pp-smt-same prime-probe smt smtcache same 11 none
control-no-victim binary-pp time-sliced default different 20 no-victim
control-forced-eviction binary-pp time-sliced default different 21 forced-eviction
EXPERIMENTS

wait_all

for f in "$RESULTS_DIR"/*/summary.csv; do
	echo "$f"
	tail -n 1 "$f"
done
