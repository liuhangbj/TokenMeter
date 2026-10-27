#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "Usage: $0 <owner/repository> <tag>" >&2
}

if [[ $# -ne 2 ]]; then
  usage
  exit 2
fi

REPOSITORY="$1"
TAG="$2"
API_URL="${GITHUB_API_URL:-https://api.github.com}"

if [[ -z "${GH_TOKEN:-}" ]]; then
  echo "::error::GH_TOKEN is required to inspect the target release" >&2
  exit 1
fi

if [[ ! "$REPOSITORY" =~ ^[^/]+/[^/]+$ ]] || [[ -z "$TAG" ]]; then
  echo "::error::Expected a non-empty owner/repository and tag" >&2
  exit 2
fi

if [[ ! "$API_URL" =~ ^https://[^/]+(/.*)?$ ]]; then
  echo "::error::GITHUB_API_URL must use HTTPS" >&2
  exit 2
fi

ENCODED_REPOSITORY="$(node - "$REPOSITORY" <<'NODE'
const repository = process.argv[2];
const [owner, name] = repository.split('/');
process.stdout.write(`${encodeURIComponent(owner)}/${encodeURIComponent(name)}`);
NODE
)"
ENCODED_TAG="$(node - "$TAG" <<'NODE'
process.stdout.write(encodeURIComponent(process.argv[2]));
NODE
)"

RESPONSE_FILE="$(mktemp "${TMPDIR:-/tmp}/tokenmeter-release-guard.XXXXXX")"
trap 'rm -f "$RESPONSE_FILE"' EXIT

github_get() {
  local url="$1"

  set +e
  HTTP_STATUS="$(curl \
    --silent \
    --show-error \
    --output "$RESPONSE_FILE" \
    --write-out '%{http_code}' \
    --header 'Accept: application/vnd.github+json' \
    --header "Authorization: Bearer $GH_TOKEN" \
    --header 'X-GitHub-Api-Version: 2022-11-28' \
    "$url")"
  CURL_EXIT=$?
  set -e

  if [[ $CURL_EXIT -ne 0 ]]; then
    echo "::error::Could not inspect release state; refusing to upload assets" >&2
    exit 1
  fi
}

github_get "${API_URL%/}/repos/$ENCODED_REPOSITORY"
if [[ "$HTTP_STATUS" != "200" ]]; then
  echo "::error::GitHub repository authentication check returned HTTP $HTTP_STATUS; refusing to upload assets" >&2
  exit 1
fi

github_get "${API_URL%/}/repos/$ENCODED_REPOSITORY/releases/tags/$ENCODED_TAG"
case "$HTTP_STATUS" in
  404)
    # GitHub's by-tag endpoint only returns published releases. A draft and no
    # release both return 404, and both are safe for tauri-action to continue.
    echo "No public release exists for $TAG; draft asset upload may continue."
    ;;
  200)
    set +e
    RELEASE_STATE="$(node - "$RESPONSE_FILE" <<'NODE'
const fs = require('node:fs');
let release;
try {
  release = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
} catch {
  process.exit(2);
}
if (typeof release.draft !== 'boolean') process.exit(2);
process.stdout.write(release.draft ? 'draft' : 'public');
NODE
)"
    PARSE_EXIT=$?
    set -e
    if [[ $PARSE_EXIT -ne 0 ]]; then
      echo "::error::GitHub returned an invalid release response; refusing to upload assets" >&2
      exit 1
    fi
    if [[ "$RELEASE_STATE" != "draft" ]]; then
      echo "::error::Release $TAG is already public; refusing to replace or delete its assets" >&2
      exit 1
    fi
    echo "Release $TAG is still a draft; asset upload may continue."
    ;;
  *)
    echo "::error::GitHub release lookup returned HTTP $HTTP_STATUS; refusing to upload assets" >&2
    exit 1
    ;;
esac
