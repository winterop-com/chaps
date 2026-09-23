#!/usr/bin/env bash
# Push the six Apple signing secrets from 1Password into GitHub, for the
# macOS signing and notarization steps in .github/workflows/release.yml.
#
# The values are piped straight from `op read` into `gh secret set` through a
# process substitution, so they never touch disk, never appear in the process
# list, and are never echoed. Only the secret names are printed.
#
# Two scopes, chosen with --scope:
#
#   --scope repo   (the default) sets the six secrets on winterop-com/chaps
#                  alone. Nothing else in the organisation can read them.
#
#   --scope org    sets them once for the winterop-com organisation with
#                  `--visibility selected --repos chaps`, so the same values
#                  can later be shared with another repository by adding it to
#                  that list instead of uploading the certificate again. The
#                  same six secrets already exist as repository secrets on
#                  winterop-com/maneki; promoting them to the organisation is
#                  the tidier end state, but it needs admin rights on the
#                  organisation and it does not remove maneki's own copies.
#
# Either way the workflow reads them as plain `secrets.APPLE_*`: a repository
# secret shadows an organisation secret of the same name, so setting both is
# harmless but pointless.
#
# Prerequisites: `gh auth login` with repo access (and admin:org for
# --scope org), and `op signin` for the 1Password account holding the item.
#
# Adjust OP_VAULT and OP_ITEM below, or pass them in the environment, to match
# where the Developer ID material actually lives:
#
#   OP_VAULT="Engineering" OP_ITEM="Apple Developer ID" scripts/github-secrets.sh
#
# The item is expected to have one field per secret; the field names are the
# right-hand side of each SECRETS entry and can be edited there.

set -euo pipefail

ORG="${ORG:-winterop-com}"
REPO_NAME="${REPO_NAME:-chaps}"
REPO="${REPO:-${ORG}/${REPO_NAME}}"

# Placeholders: change these to the vault and item that hold the Developer ID
# certificate, or export OP_VAULT / OP_ITEM before running.
OP_VAULT="${OP_VAULT:-Private}"
OP_ITEM="${OP_ITEM:-Apple Developer ID}"

# GitHub secret name : 1Password field name
SECRETS=(
  "APPLE_CERTIFICATE:certificate_base64"
  "APPLE_CERTIFICATE_PASSWORD:certificate_password"
  "APPLE_SIGNING_IDENTITY:signing_identity"
  "APPLE_ID:apple_id"
  "APPLE_PASSWORD:app_specific_password"
  "APPLE_TEAM_ID:team_id"
)

scope="repo"
dry_run=0

usage() {
  cat <<USAGE
usage: scripts/github-secrets.sh [--scope repo|org] [--dry-run]

  --scope repo   set the secrets on ${REPO} (default)
  --scope org    set them on the ${ORG} organisation, visible to ${REPO_NAME}
  --dry-run      print what would be set, read nothing from 1Password

environment: ORG, REPO_NAME, REPO, OP_VAULT, OP_ITEM
USAGE
}

while [ $# -gt 0 ]; do
  case "$1" in
    --scope)
      shift
      scope="${1:-}"
      ;;
    --scope=*)
      scope="${1#--scope=}"
      ;;
    --dry-run)
      dry_run=1
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
  shift
done

case "$scope" in
  repo | org) ;;
  *)
    echo "error: --scope must be 'repo' or 'org', got '${scope}'" >&2
    exit 2
    ;;
esac

for tool in gh op; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "error: ${tool} is not installed or not on PATH" >&2
    exit 1
  fi
done

echo "organisation-level secrets already on ${ORG}:"
if ! gh secret list --org "$ORG" 2>/dev/null; then
  echo "  (none, or the current gh token has no admin:org access)"
fi
echo

if [ "$scope" = "org" ]; then
  echo "setting ${#SECRETS[@]} secrets on the ${ORG} organisation, visible to ${REPO_NAME}"
else
  echo "setting ${#SECRETS[@]} secrets on ${REPO}"
fi
echo "reading values from op://${OP_VAULT}/${OP_ITEM}/<field>"
echo

for entry in "${SECRETS[@]}"; do
  name="${entry%%:*}"
  field="${entry#*:}"
  reference="op://${OP_VAULT}/${OP_ITEM}/${field}"

  if [ "$dry_run" -eq 1 ]; then
    if [ "$scope" = "org" ]; then
      echo "would set ${name} on ${ORG} from ${reference}"
    else
      echo "would set ${name} on ${REPO} from ${reference}"
    fi
    continue
  fi

  # The value travels from op to gh on a file descriptor. It is never assigned
  # to a shell variable, written to a file, or passed as an argument.
  if [ "$scope" = "org" ]; then
    gh secret set "$name" \
      --org "$ORG" \
      --visibility selected \
      --repos "$REPO_NAME" \
      < <(op read "$reference")
  else
    gh secret set "$name" \
      --repo "$REPO" \
      < <(op read "$reference")
  fi
  echo "set ${name}"
done

echo
if [ "$dry_run" -eq 1 ]; then
  echo "dry run: nothing was set"
else
  echo "done; the next tagged build will sign and notarize the macOS binaries"
fi
