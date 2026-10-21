# shellcheck shell=bash
# Step runner of scripts/check.sh (REF-02). Sourced, never executed.
#
#   check_init LOG_DIR
#   check_run ID LABEL [--needs ID]... [--after ID]... [--cwd DIR] [--report KIND:PATH]... -- COMMAND...
#   check_finish            prints the table; returns 0 only if every step passed
#
# A step's output goes to the terminal and to LOG_DIR/ID.log through `tee`.
# The result is the exit code of the tool itself, read from PIPESTATUS, never
# the code of `tee` (A8). `--report` names what the step ran (cargo, junit:X,
# unittest:X) so that skips.py can fail it for a skip outside the allowed list
# and name its failed tests, reading the text with its colours removed (A2,
# A8). With CHECK_PLAN set,
# check_run only prints `id<TAB>label<TAB>dir<TAB>command`.

CHECK_RUNNER_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK_ALLOWED_SKIPS="$CHECK_RUNNER_DIR/allowed-skips.toml"

check_init() {
  CHECK_LOG_DIR="$1"
  CHECK_IDS=()
  CHECK_CURRENT=""
  declare -gA CHECK_RESULT_OF=()
  declare -gA CHECK_LABEL_OF=()
  CHECK_STARTED_US="$(check_now_us)"
  if [[ -z "${CHECK_PLAN:-}" ]]; then
    mkdir -p "$CHECK_LOG_DIR"
    : >"$CHECK_LOG_DIR/resultados.tsv"
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
    >>"$CHECK_LOG_DIR/resultados.tsv"
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
      *) printf 'check_run %s: opción desconocida %s\n' "$id" "$1" >&2; return 2 ;;
    esac
  done
  if (($# == 0)); then
    printf 'check_run %s: falta la orden\n' "$id" >&2
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
      "") cause="no se ejecutó «$need»" ;;
      omitido) cause="se omitió «${CHECK_LABEL_OF[$need]}»" ;;
      *) cause="falló «${CHECK_LABEL_OF[$need]}»" ;;
    esac
    printf '\n── %s: omitido, %s\n' "$label" "$cause"
    check_record "$id" "$label" omitido 0 "$cause"
    return 0
  done
  for need in "${after[@]}"; do
    if [[ "${CHECK_RESULT_OF[$need]:-omitido}" == omitido ]]; then
      cause="no se ejecutó «${CHECK_LABEL_OF[$need]:-$need}»"
      printf '\n── %s: omitido, %s\n' "$label" "$cause"
      check_record "$id" "$label" omitido 0 "$cause"
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
  (cd "$cwd" && "$@") </dev/null 2>&1 | tee "$log"
  statuses=("${PIPESTATUS[@]}")
  ((errexit)) && set -e
  CHECK_CURRENT=""

  local causes=() tool="${statuses[0]}" keeper="${statuses[1]}"
  ((tool == 0)) || causes+=("código $tool")
  ((keeper == 0)) || causes+=("no se pudo guardar su salida en $log (tee terminó con $keeper)")
  local report kind path cause_file="$CHECK_LOG_DIR/$id.cause" skip_arguments=()
  for report in "${reports[@]}"; do
    kind="${report%%:*}"
    path="${report#*:}"
    case "$kind" in
      cargo) skip_arguments+=(--cargo-log "$log") ;;
      junit) skip_arguments+=(--junit "$path") ;;
      unittest) skip_arguments+=(--unittest "$path") ;;
      *) causes+=("informe de tipo desconocido $kind") ;;
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
      causes+=("saltos sin comprobar: skips.py terminó con código ${statuses[0]}")
    fi
    ((statuses[1] == 0)) || causes+=("no se pudo guardar su salida en $log")
  fi

  local seconds result=ok joined=""
  seconds="$(check_seconds_since "$started")"
  if ((${#causes[@]})); then
    result=fallo
    joined="${causes[0]}"
    local more
    for more in "${causes[@]:1}"; do joined+="; $more"; done
    printf '── %s: FALLO (%s) en %s s\n' "$label" "$joined" "$seconds"
  else
    printf '── %s: ok en %s s\n' "$label" "$seconds"
  fi
  check_record "$id" "$label" "$result" "$seconds" "$joined"
}

check_interrupted() {
  local signal="$1"
  if [[ -n "${CHECK_CURRENT:-}" ]]; then
    check_record "$CHECK_CURRENT" "$CHECK_CURRENT_LABEL" interrumpido \
      "$(check_seconds_since "$CHECK_CURRENT_STARTED_US")" "señal $signal"
    CHECK_CURRENT=""
  fi
  check_finish || true
}

check_finish() {
  [[ -z "${CHECK_PLAN:-}" ]] || return 0
  python3 -B "$CHECK_RUNNER_DIR/table.py" "$CHECK_LOG_DIR/resultados.tsv" \
    "$(check_seconds_since "$CHECK_STARTED_US")"
  printf 'Salida de cada paso: %s\n' "$CHECK_LOG_DIR"
  local id
  for id in "${CHECK_IDS[@]}"; do
    [[ "${CHECK_RESULT_OF[$id]}" == ok ]] || return 1
  done
  ((${#CHECK_IDS[@]} > 0))
}
