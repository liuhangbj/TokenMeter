#!/usr/bin/env bash

set -euo pipefail

usage() {
  cat <<'EOF'
Usage: verify-macos-artifacts.sh \
  --target <aarch64-apple-darwin|x86_64-apple-darwin> \
  --version <version> \
  --bundle-id <identifier> \
  --dmg <path> \
  --updater <path-to-app.tar.gz> \
  --updater-signature <path-to-app.tar.gz.sig> \
  [--negative-test]

Validates the signed .app inside both the final DMG and updater archive.
This checks complete ad-hoc bundle integrity; it does not claim Developer ID,
notarization, or Gatekeeper trust.
EOF
}

fail() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

target=""
version=""
bundle_id=""
dmg_path=""
updater_path=""
updater_signature_path=""
negative_test=false

while (( $# > 0 )); do
  case "$1" in
    --target)
      (( $# >= 2 )) || fail "--target requires a value"
      target="$2"
      shift 2
      ;;
    --version)
      (( $# >= 2 )) || fail "--version requires a value"
      version="$2"
      shift 2
      ;;
    --bundle-id)
      (( $# >= 2 )) || fail "--bundle-id requires a value"
      bundle_id="$2"
      shift 2
      ;;
    --dmg)
      (( $# >= 2 )) || fail "--dmg requires a value"
      dmg_path="$2"
      shift 2
      ;;
    --updater)
      (( $# >= 2 )) || fail "--updater requires a value"
      updater_path="$2"
      shift 2
      ;;
    --updater-signature)
      (( $# >= 2 )) || fail "--updater-signature requires a value"
      updater_signature_path="$2"
      shift 2
      ;;
    --negative-test)
      negative_test=true
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

[[ "$(uname -s)" == "Darwin" ]] || fail "macOS is required"
[[ -n "$target" ]] || fail "--target is required"
[[ -n "$version" ]] || fail "--version is required"
[[ -n "$bundle_id" ]] || fail "--bundle-id is required"
[[ -f "$dmg_path" ]] || fail "DMG not found: $dmg_path"
[[ -f "$updater_path" ]] || fail "updater archive not found: $updater_path"
[[ -s "$updater_signature_path" ]] || fail "updater signature missing or empty: $updater_signature_path"

case "$target" in
  aarch64-apple-darwin)
    expected_arch="arm64"
    wrong_arch="x86_64"
    ;;
  x86_64-apple-darwin)
    expected_arch="x86_64"
    wrong_arch="arm64"
    ;;
  *)
    fail "unsupported macOS target: $target"
    ;;
esac

for required_command in codesign ditto file find hdiutil lipo plutil shasum tar; do
  command -v "$required_command" >/dev/null 2>&1 || fail "required command not found: $required_command"
done

verify_root="$(mktemp -d "${TMPDIR:-/tmp}/tokenmeter-macos-artifacts.XXXXXX")"
mount_dir="$verify_root/dmg"
updater_dir="$verify_root/updater"
mounted=false

cleanup() {
  if [[ "$mounted" == true ]]; then
    hdiutil detach "$mount_dir" >/dev/null 2>&1 || true
  fi
  if [[ -d "$verify_root" && "$verify_root" == *tokenmeter-macos-artifacts.* ]]; then
    find "$verify_root" -depth -delete >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

plist_value() {
  local plist="$1"
  local key="$2"
  /usr/libexec/PlistBuddy -c "Print :$key" "$plist"
}

assert_equal() {
  local label="$1"
  local actual="$2"
  local expected="$3"
  [[ "$actual" == "$expected" ]] || fail "$label mismatch: expected '$expected', got '$actual'"
}

app_executable() {
  local app="$1"
  local plist="$app/Contents/Info.plist"
  [[ -f "$plist" ]] || return 1
  local executable_name
  executable_name="$(plist_value "$plist" CFBundleExecutable)" || return 1
  printf '%s\n' "$app/Contents/MacOS/$executable_name"
}

check_architecture() {
  local executable="$1"
  local wanted_arch="$2"
  local actual_archs
  actual_archs="$(lipo -archs "$executable")" || return 1
  [[ "$actual_archs" == "$wanted_arch" ]]
}

find_single_app() {
  local root="$1"
  local label="$2"
  local list_file="$verify_root/${label}-apps.txt"
  find "$root" -maxdepth 3 -type d -name '*.app' -print > "$list_file"
  local count
  count="$(awk 'END { print NR }' "$list_file")"
  [[ "$count" == "1" ]] || fail "$label must contain exactly one .app, found $count"
  sed -n '1p' "$list_file"
}

validate_app() {
  local label="$1"
  local app="$2"
  local plist="$app/Contents/Info.plist"
  local executable
  local display_log="$verify_root/${label}-codesign-display.txt"

  [[ -f "$plist" ]] || fail "$label is missing Contents/Info.plist"
  assert_equal "$label bundle identifier" "$(plist_value "$plist" CFBundleIdentifier)" "$bundle_id"
  assert_equal "$label short version" "$(plist_value "$plist" CFBundleShortVersionString)" "$version"
  assert_equal "$label bundle version" "$(plist_value "$plist" CFBundleVersion)" "$version"

  executable="$(app_executable "$app")" || fail "$label executable could not be resolved"
  [[ -f "$executable" ]] || fail "$label executable not found: $executable"
  file "$executable" | grep -F 'Mach-O' >/dev/null || fail "$label executable is not Mach-O"
  check_architecture "$executable" "$expected_arch" || fail "$label architecture is not exactly $expected_arch"

  codesign --verify --deep --strict --verbose=4 "$app"
  codesign -dvvv --verbose=4 "$app" > "$display_log" 2>&1

  grep -Fqx 'Signature=adhoc' "$display_log" || fail "$label is not fully ad-hoc signed"
  grep -Fqx 'TeamIdentifier=not set' "$display_log" || fail "$label unexpectedly has a TeamIdentifier"
  grep -Eq '^Info\.plist entries=[1-9][0-9]*$' "$display_log" || fail "$label Info.plist is not bound by the signature"
  grep -Eq '^Sealed Resources version=[0-9]+ rules=[0-9]+ files=[1-9][0-9]*$' "$display_log" || fail "$label has no complete sealed resource envelope"
  if grep -Fq 'linker-signed' "$display_log"; then
    fail "$label still has only a linker-generated signature"
  fi

  local macho_count=0
  while IFS= read -r candidate; do
    if file -b "$candidate" | grep -Fq 'Mach-O'; then
      codesign --verify --strict --verbose=4 "$candidate"
      macho_count=$((macho_count + 1))
    fi
  done < <(find "$app/Contents" -type f -print)
  (( macho_count > 0 )) || fail "$label contains no signed Mach-O files"

  LAST_EXECUTABLE="$executable"
  LAST_CDHASH="$(awk -F= '/^CDHash=/{print $2; exit}' "$display_log")"
  [[ -n "$LAST_CDHASH" ]] || fail "$label CDHash is missing"
  printf 'verified %s: arch=%s version=%s cdhash=%s macho_files=%s\n' \
    "$label" "$expected_arch" "$version" "$LAST_CDHASH" "$macho_count"
}

mkdir -p "$mount_dir" "$updater_dir"
hdiutil attach -readonly -nobrowse -mountpoint "$mount_dir" "$dmg_path" > "$verify_root/hdiutil-attach.txt"
mounted=true
dmg_app="$(find_single_app "$mount_dir" dmg)"
validate_app "DMG app" "$dmg_app"
dmg_cdhash="$LAST_CDHASH"

hdiutil detach "$mount_dir" > "$verify_root/hdiutil-detach.txt"
mounted=false

tar -tzf "$updater_path" > "$verify_root/updater-archive-list.txt"
awk '$0 ~ /^\// || $0 ~ /(^|\/)\.\.($|\/)/ { bad=1 } END { exit bad }' \
  "$verify_root/updater-archive-list.txt" || fail "updater archive contains unsafe paths"
tar -xzf "$updater_path" -C "$updater_dir"
updater_app="$(find_single_app "$updater_dir" updater)"
validate_app "updater app" "$updater_app"
updater_cdhash="$LAST_CDHASH"
updater_executable="$LAST_EXECUTABLE"

assert_equal "DMG/updater CDHash" "$updater_cdhash" "$dmg_cdhash"

/usr/bin/base64 -D < "$updater_signature_path" > "$verify_root/updater-signature.decoded"
grep -Fq 'untrusted comment: signature from tauri secret key' "$verify_root/updater-signature.decoded" \
  || fail "updater signature does not have the expected Tauri signature envelope"

shasum -a 256 "$dmg_path" "$updater_path" "$updater_signature_path"

if [[ "$negative_test" == true ]]; then
  if check_architecture "$updater_executable" "$wrong_arch"; then
    fail "negative test failed: wrong architecture '$wrong_arch' was accepted"
  fi
  printf 'negative test passed: wrong architecture %s was rejected\n' "$wrong_arch"

  tampered_app="$verify_root/Tampered.app"
  ditto "$updater_app" "$tampered_app"
  tampered_resource="$(find "$tampered_app/Contents/Resources" -type f -print | sed -n '1p')"
  [[ -n "$tampered_resource" && -f "$tampered_resource" ]] || fail "negative test fixture resource is missing"
  printf '\nintentional signature validation mutation\n' >> "$tampered_resource"
  if codesign --verify --deep --strict --verbose=4 "$tampered_app" > "$verify_root/tampered-verify.txt" 2>&1; then
    fail "negative test failed: modified sealed resource was accepted"
  fi
  printf 'negative test passed: modified sealed resource was rejected\n'
fi

printf 'macOS artifact verification passed for %s\n' "$target"
