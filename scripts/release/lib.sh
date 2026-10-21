#!/usr/bin/env bash

release_test_tools() {
  [[ ${ORBIT_RELEASE_UNDER_TEST:-} == 1 ]] || return 0
  if [[ -z ${ORBIT_TEST_BIN:-} || ! -d $ORBIT_TEST_BIN ]]; then
    echo 'ORBIT_TEST_BIN is missing: the directory of test doubles.' >&2
    return 1
  fi
  local tool found
  for tool in ssh rsync codesign xcrun spctl security cc rustup; do
    found="$(type -P "$tool" || true)"
    if [[ $found != "$ORBIT_TEST_BIN/$tool" || ! -f $found || ! -x $found || -L $found ]]; then
      echo "Test stopped: $tool is not the executable double in ORBIT_TEST_BIN ($found)." >&2
      return 1
    fi
  done
}

release_arguments() {
  local version=${1:-} commit=${2:-}
  if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ || ! $commit =~ ^[0-9a-f]{40}$ ]]; then
    echo "Invalid version or commit: version='$version', commit='$commit'." >&2
    return 2
  fi
}

release_connection() {
  if [[ ! ${ORBIT_MAC_HOST:-} =~ ^[A-Za-z0-9_][A-Za-z0-9_.-]*(@[A-Za-z0-9_][A-Za-z0-9_.-]*)?$ ]]; then
    echo "Invalid ORBIT_MAC_HOST: '${ORBIT_MAC_HOST:-}'." >&2
    return 2
  fi
  if [[ ! ${ORBIT_MAC_SSH_KEY:-} =~ ^/[A-Za-z0-9_./-]+$ || ! -f ${ORBIT_MAC_SSH_KEY:-} ]]; then
    echo "ORBIT_MAC_SSH_KEY missing or invalid: '${ORBIT_MAC_SSH_KEY:-}'." >&2
    return 2
  fi
}

# Every ssh and rsync of a release leaves with these. BatchMode=yes refuses a
# password prompt nobody is at the other end to answer; ConnectTimeout=15
# bounds the wait for a sleeping Mac to seconds instead of TCP's own patience;
# IdentitiesOnly=yes keeps ssh from offering any key but ORBIT_MAC_SSH_KEY.
RELEASE_SSH_OPTIONS=(-o BatchMode=yes -o ConnectTimeout=15 -o IdentitiesOnly=yes)

# Kept literal so the remote shell expands its own HOME. Local Mac scripts
# substitute HOME as data, without evaluating shell code.
RELEASE_HOST_PATH='$HOME/.cargo/bin:$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin'

release_source_commit() {
  release_arguments "$version" "$commit" || return
  if ! git -C "$repo" cat-file -e "$commit^{commit}" 2>/dev/null; then
    echo "No such commit: version='$version', commit='$commit'." >&2
    return 1
  fi
}

# Read signing material as data, never source it. Both the release build and
# Orbit Pruebas reject unknown keys, symlinks and files exposed to other users.
release_private_file() {
  local file="$1" label="$2" mode
  if [[ ! -f $file || -L $file ]]; then
    echo "$label is not a regular file: $file" >&2
    return 1
  fi
  RELEASE_PRIVATE_FILE="$(cd -- "$(dirname -- "$file")" && pwd -P)/$(basename -- "$file")"
  if [[ $RELEASE_PRIVATE_FILE == "$repo"/* ]]; then
    echo "$label must be outside the repository: $RELEASE_PRIVATE_FILE" >&2
    return 1
  fi
  if ! mode="$(stat -f '%Lp' "$RELEASE_PRIVATE_FILE" 2>/dev/null)"; then
    mode="$(stat -c '%a' "$RELEASE_PRIVATE_FILE")" || return
  fi
  if [[ ! $mode =~ ^[0-7]{3,4}$ || $((8#$mode & 077)) -ne 0 ]]; then
    echo "$label has mode $mode; group and others must not be able to read it." >&2
    return 1
  fi
}

release_read_signing_env() {
  release_private_file "$ORBIT_SIGNING_ENV" ORBIT_SIGNING_ENV || return
  local line key value
  while IFS= read -r line || [[ -n $line ]]; do
    [[ -z $line || $line == \#* ]] && continue
    if [[ $line != *=* ]]; then
      echo 'ORBIT_SIGNING_ENV contains a line without KEY=value.' >&2
      return 1
    fi
    key=${line%%=*}
    value=${line#*=}
    case $key in
      ORBIT_SIGN_IDENTITY|ORBIT_SIGN_KEYCHAIN|ORBIT_SIGN_KEYCHAIN_PASSWORD_FILE|ORBIT_NOTARY_KEY|ORBIT_NOTARY_KEY_ID|ORBIT_NOTARY_ISSUER)
        printf -v "$key" '%s' "$value"
        export "$key"
        ;;
      *) echo "ORBIT_SIGNING_ENV contains a key that is not accepted: $key" >&2; return 1 ;;
    esac
  done < "$RELEASE_PRIVATE_FILE"
}
