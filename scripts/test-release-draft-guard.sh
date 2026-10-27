#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GUARD="$SCRIPT_DIR/guard-release-draft.sh"
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/tokenmeter-release-guard-test.XXXXXX")"
trap 'rm -rf "$TEST_ROOT"' EXIT

mkdir -p "$TEST_ROOT/bin" "$TEST_ROOT/tmp"
cat > "$TEST_ROOT/bin/curl" <<'STUB'
#!/usr/bin/env bash
set -u

printf '%s\n' "$@" >> "$STUB_ARGS_FILE"

output=''
url="${!#}"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output)
      output="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

if [[ "$STUB_MODE" == network ]]; then
  printf '000'
  exit 7
fi

if [[ "$url" != */releases/tags/* ]]; then
  if [[ "$STUB_MODE" == auth-error ]]; then
    printf '{"message":"Bad credentials"}\n' > "$output"
    printf '401'
  else
    printf '{"full_name":"example/tokenmeter"}\n' > "$output"
    printf '200'
  fi
  exit 0
fi

case "$STUB_MODE" in
  public)
    printf '{"draft":false}\n' > "$output"
    printf '200'
    ;;
  draft|missing)
    # GitHub's by-tag endpoint hides draft releases, so both states are 404.
    printf '{"message":"Not Found"}\n' > "$output"
    printf '404'
    ;;
  server-error)
    printf '{"message":"Server Error"}\n' > "$output"
    printf '500'
    ;;
  malformed)
    printf '{not-json}\n' > "$output"
    printf '200'
    ;;
  *)
    exit 99
    ;;
esac
STUB
chmod +x "$TEST_ROOT/bin/curl"

run_case() {
  local mode="$1"
  local expected="$2"
  local output="$TEST_ROOT/$mode.log"
  local args="$TEST_ROOT/$mode.args"
  local status=0

  set +e
  PATH="$TEST_ROOT/bin:$PATH" \
    TMPDIR="$TEST_ROOT/tmp" \
    GH_TOKEN='synthetic-test-token' \
    GITHUB_API_URL='https://api.github.test' \
    STUB_MODE="$mode" \
    STUB_ARGS_FILE="$args" \
    "$GUARD" 'example/tokenmeter' 'v0.9.12' > "$output" 2>&1
  status=$?
  set -e

  if [[ "$expected" == pass && $status -ne 0 ]]; then
    echo "FAIL $mode: expected success, got $status" >&2
    cat "$output" >&2
    exit 1
  fi
  if [[ "$expected" == fail && $status -eq 0 ]]; then
    echo "FAIL $mode: expected failure" >&2
    cat "$output" >&2
    exit 1
  fi
  if grep -Eq '^(--request|-X|--data|--data-binary|--upload-file|-T)(=|$)' "$args"; then
    echo "FAIL $mode: guard attempted a mutating HTTP operation" >&2
    cat "$args" >&2
    exit 1
  fi
  echo "PASS $mode ($expected)"
}

run_case public fail
run_case draft pass
run_case missing pass
run_case server-error fail
run_case network fail
run_case malformed fail
run_case auth-error fail

echo "All release draft guard tests passed."
