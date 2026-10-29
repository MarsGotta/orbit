# shellcheck shell=bash
# Step runner of scripts/check.sh. Sourced, never executed.
#
#   check_init LOG_DIR
#   check_run ID LABEL [--needs ID]... [--after ID]... [--cwd DIR] [--report KIND:PATH]... -- COMMAND...
#   check_finish            prints the table; returns 0 only if every step passed
#
# A step writes LOG_DIR/ID.log; output.py streams it until the command exits,
# even if an orphaned helper retains its output. The result is the tool's own
# exit code, never the streamer's. `--report` names what the step ran (cargo, junit:X,
# unittest:X) so that skips.py can fail it for a skip outside the allowed list
# and name its failed tests, reading the text with its colours removed. With
# CHECK_PLAN set, check_run only prints `id<TAB>label<TAB>dir<TAB>command`.

CHECK_RUNNER_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK_ALLOWED_SKIPS="$CHECK_RUNNER_DIR/allowed-skips.toml"

check_init() {
  CHECK_LOG_DIR="$1"
  CHECK_IDS=()
  CHECK_CURRENT=""
  CHECK_OUTPUT_PID=""
  declare -gA CHECK_RESULT_OF=()
  declare -gA CHECK_LABEL_OF=()
  CHECK_STARTED_US="$(check_now_us)"
  if [[ -z "${CHECK_PLAN:-}" ]]; then
    mkdir -p "$CHECK_LOG_DIR"
    : >"$CHECK_LOG_DIR/results.tsv"
  fi
}

check_now_us() {
  local now="${EPOCHREALTIME/[.,]/}"
  printf '%s' "$now"
}

check_seconds_since() {
  local elapsed=$(($(check_now_us) - $1))
  printf '%d.%03d' $((elapsed / 1000000)) $(((elapsed % 1000000) / 1000))
}

check_render() {
  local rendered
  printf -v rendered '%q ' "$@"
  printf '%s' "${rendered% }"
}

check_record() {
  local id="$1" label="$2" result="$3" seconds="$4" cause="${5:-}"
  cause="${cause//$'\t'/ }"
  cause="${cause//$'\n'/ }"
  CHECK_IDS+=("$id")
  CHECK_RESULT_OF["$id"]="$result"
  CHECK_LABEL_OF["$id"]="$label"
  printf '%s\t%s\t%s\t%s\t%s\n' "$id" "$result" "$seconds" "$label" "$cause" \
    >>"$CHECK_LOG_DIR/results.tsv"
}

check_run() {
  local id="$1" label="$2"
  shift 2
  local cwd="$PWD" needs=() after=() reports=()
  while (($#)); do
    case "$1" in
      --needs) needs+=("$2"); shift 2 ;;
      --after) after+=("$2"); shift 2 ;;
      --cwd) cwd="$2"; shift 2 ;;
      --report) reports+=("$2"); shift 2 ;;
      --) shift; break ;;
      *) printf 'check_run %s: unknown option %s\n' "$id" "$1" >&2; return 2 ;;
    esac
  done
  if (($# == 0)); then
    printf 'check_run %s: the command is missing\n' "$id" >&2
    return 2
  fi
  if [[ -n "${CHECK_PLAN:-}" ]]; then
    printf '%s\t%s\t%s\t%s\n' "$id" "$label" "$cwd" "$(check_render "$@")"
    return 0
  fi

  local need prior cause
  for need in "${needs[@]}"; do
    prior="${CHECK_RESULT_OF[$need]:-}"
    [[ "$prior" != ok ]] || continue
    case "$prior" in
      "") cause="'$need' did not run" ;;
      skipped) cause="'${CHECK_LABEL_OF[$need]}' was skipped" ;;
      *) cause="'${CHECK_LABEL_OF[$need]}' failed" ;;
    esac
    printf '\n── %s: skipped, %s\n' "$label" "$cause"
    check_record "$id" "$label" skipped 0 "$cause"
    return 0
  done
  for need in "${after[@]}"; do
    if [[ "${CHECK_RESULT_OF[$need]:-skipped}" == skipped ]]; then
      cause="'${CHECK_LABEL_OF[$need]:-$need}' did not run"
      printf '\n── %s: skipped, %s\n' "$label" "$cause"
      check_record "$id" "$label" skipped 0 "$cause"
      return 0
    fi
  done

  local log="$CHECK_LOG_DIR/$id.log" started statuses errexit=0
  [[ $- == *e* ]] && errexit=1
  printf '\n━━ %s · %s\n' "$label" "$(check_render "$@")"
  CHECK_CURRENT="$id"
  CHECK_CURRENT_LABEL="$label"
  CHECK_CURRENT_STARTED_US="$(check_now_us)"
  started="$CHECK_CURRENT_STARTED_US"
  set +e
  local completed="$CHECK_LOG_DIR/$id.completed" tool keeper bytes
  rm -f "$completed"
  : >"$log"
  python3 -B "$CHECK_RUNNER_DIR/output.py" "$log" "$completed" &
  CHECK_OUTPUT_PID=$!
  # Keep the tool in the foreground so it still receives terminal interrupts.
  (cd "$cwd" && "$@") </dev/null >"$log" 2>&1
  tool=$?
  # Only this shell creates the marker, after the tool exits. Record its
  # output boundary so descendants writing forever cannot keep us here.
  bytes="$(wc -c <"$log")"
  printf '%s\n' "$bytes" >"$completed.tmp"
  mv "$completed.tmp" "$completed"
  wait "$CHECK_OUTPUT_PID"
  keeper=$?
  CHECK_OUTPUT_PID=""
  ((errexit)) && set -e
  CHECK_CURRENT=""

  local causes=()
  ((tool == 0)) || causes+=("code $tool")
  ((keeper == 0)) || causes+=("its output could not be streamed from $log (output.py exited with $keeper)")
  local report kind path cause_file="$CHECK_LOG_DIR/$id.cause" skip_arguments=()
  for report in "${reports[@]}"; do
    kind="${report%%:*}"
    path="${report#*:}"
    case "$kind" in
      cargo) skip_arguments+=(--cargo-log "$log") ;;
      junit) skip_arguments+=(--junit "$path") ;;
      unittest) skip_arguments+=(--unittest "$path") ;;
      *) causes+=("report of unknown kind $kind") ;;
    esac
  done
  if ((${#skip_arguments[@]})); then
    rm -f "$cause_file"
    set +e
    python3 -B "$CHECK_RUNNER_DIR/skips.py" --suite "$id" --allow "$CHECK_ALLOWED_SKIPS" \
      --cause-file "$cause_file" "${skip_arguments[@]}" 2>&1 | tee -a "$log"
    statuses=("${PIPESTATUS[@]}")
    ((errexit)) && set -e
    if [[ -s "$cause_file" ]]; then
      causes+=("$(<"$cause_file")")
    elif ((statuses[0] != 0)) && ((tool == 0)); then
      causes+=("skips unchecked: skips.py exited with code ${statuses[0]}")
    fi
    ((statuses[1] == 0)) || causes+=("its output could not be saved to $log")
  fi

  local seconds result=ok joined=""
  seconds="$(check_seconds_since "$started")"
  if ((${#causes[@]})); then
    result=failed
    joined="${causes[0]}"
    local more
    for more in "${causes[@]:1}"; do joined+="; $more"; done
    printf '── %s: FAILED (%s) in %s s\n' "$label" "$joined" "$seconds"
  else
    printf '── %s: ok in %s s\n' "$label" "$seconds"
  fi
  check_record "$id" "$label" "$result" "$seconds" "$joined"
}

check_interrupted() {
  local signal="$1"
  if [[ -n "${CHECK_OUTPUT_PID:-}" ]]; then
    kill -TERM "$CHECK_OUTPUT_PID" 2>/dev/null || true
    wait "$CHECK_OUTPUT_PID" 2>/dev/null || true
    CHECK_OUTPUT_PID=""
  fi
  if [[ -n "${CHECK_CURRENT:-}" ]]; then
    check_record "$CHECK_CURRENT" "$CHECK_CURRENT_LABEL" interrupted \
      "$(check_seconds_since "$CHECK_CURRENT_STARTED_US")" "signal $signal"
    CHECK_CURRENT=""
  fi
  check_finish || true
}

check_finish() {
  [[ -z "${CHECK_PLAN:-}" ]] || return 0
  python3 -B "$CHECK_RUNNER_DIR/table.py" "$CHECK_LOG_DIR/results.tsv" \
    "$(check_seconds_since "$CHECK_STARTED_US")"
  printf 'Output of each step: %s\n' "$CHECK_LOG_DIR"
  local id
  for id in "${CHECK_IDS[@]}"; do
    [[ "${CHECK_RESULT_OF[$id]}" == ok ]] || return 1
  done
  ((${#CHECK_IDS[@]} > 0))
}
