#!/usr/bin/env bash
set -euo pipefail
set +x

repo="${GITHUB_REPOSITORY:-pwrdrvr/PwrRoute}"
environment="${GITHUB_ENVIRONMENT:-windows-signing}"
vault_name="${OP_VAULT_NAME:-PwrDrvr LLC}"
item_title="${OP_ITEM_TITLE:-PwrRoute - Windows Azure Trusted Signing}"

# Read by release.yml's windows-sign job as vars.* (not secret).
variable_fields=(
  WIN_AZURE_SIGN_PUBLISHER_NAME
  WIN_AZURE_SIGN_ENDPOINT
  WIN_AZURE_SIGN_ACCOUNT
  WIN_AZURE_SIGN_PROFILE
)
# Read as secrets.*; the Artifact Signing dlib takes the service principal
# from these.
secret_fields=(
  AZURE_TENANT_ID
  AZURE_CLIENT_ID
  AZURE_CLIENT_SECRET
)

usage() {
  cat >&2 <<'EOF'
Uploads PwrRoute Windows (Azure Artifact Signing) settings from 1Password to
the GitHub windows-signing environment: four environment variables and three
environment secrets.

Usage:
  scripts/release/upload-azure-signing-from-1password.sh
  scripts/release/upload-azure-signing-from-1password.sh --list-accounts

Required:
  OP_ACCOUNT=<1Password account identifier>

Use the USER ID from `op account list` when the URL or email appears more than
once. The URL is ambiguous on machines with multiple accounts on the same
1Password domain.

The item needs one field per name, labeled exactly:
  WIN_AZURE_SIGN_PUBLISHER_NAME, WIN_AZURE_SIGN_ENDPOINT,
  WIN_AZURE_SIGN_ACCOUNT, WIN_AZURE_SIGN_PROFILE,
  AZURE_TENANT_ID, AZURE_CLIENT_ID, AZURE_CLIENT_SECRET

Optional overrides:
  GITHUB_REPOSITORY=pwrdrvr/PwrRoute
  GITHUB_ENVIRONMENT=windows-signing
  OP_VAULT_NAME="PwrDrvr LLC"
  OP_ITEM_TITLE="PwrRoute - Windows Azure Trusted Signing"

Example:
  OP_ACCOUNT=<USER_ID_FROM_OP_ACCOUNT_LIST> scripts/release/upload-azure-signing-from-1password.sh
EOF
}

print_accounts() {
  local accounts_json
  if ! accounts_json="$(op account list --format json 2>/dev/null)"; then
    echo "Could not list configured 1Password accounts." >&2
    return 1
  fi

  if ! jq -e 'length > 0' <<<"$accounts_json" >/dev/null; then
    echo "No configured 1Password accounts found." >&2
    return 1
  fi

  cat >&2 <<'EOF'
Configured 1Password accounts:

NAME		URL			EMAIL			USER ID
EOF

  while IFS=$'\t' read -r user_uuid url email; do
    local account_json
    local name
    if account_json="$(op account get --account "$user_uuid" --format json 2>/dev/null)"; then
      name="$(jq -r '.name // "(name unavailable)"' <<<"$account_json")"
    else
      name="(name unavailable)"
    fi
    printf '%s\t%s\t%s\t%s\n' "$name" "$url" "$email" "$user_uuid" >&2
  done < <(jq -r '.[] | [.user_uuid, .url, .email] | @tsv' <<<"$accounts_json")

  cat >&2 <<'EOF'

Use the USER ID for the account that contains the configured vault.
Run again with:
  OP_ACCOUNT=<USER ID> ./scripts/release/upload-azure-signing-from-1password.sh
EOF
}

require_command() {
  local command_name="$1"
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "Missing required command: $command_name" >&2
    exit 127
  fi
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  usage
  exit 0
fi

if [[ "${1:-}" == "--list-accounts" ]]; then
  require_command op
  require_command jq
  print_accounts
  exit 0
fi

require_command jq
require_command op

if [[ -z "${OP_ACCOUNT:-}" ]]; then
  cat >&2 <<'EOF'
Set OP_ACCOUNT to the 1Password account identifier first.

Use the USER ID column when the URL or email appears more than once.
EOF
  print_accounts
  exit 2
fi

require_command gh

vault_id="$(
  op vault list --account "$OP_ACCOUNT" --format json \
    | jq -r --arg name "$vault_name" 'first(.[] | select(.name == $name) | .id) // empty'
)"
if [[ -z "$vault_id" ]]; then
  echo "Could not find vault '$vault_name' in 1Password account '$OP_ACCOUNT'." >&2
  exit 1
fi

item_id="$(
  op item list --account "$OP_ACCOUNT" --vault "$vault_id" --format json \
    | jq -r --arg title "$item_title" 'first(.[] | select(.title == $title) | .id) // empty'
)"
if [[ -z "$item_id" ]]; then
  echo "Could not find item '$item_title' in vault '$vault_name'." >&2
  exit 1
fi

item_json="$(op item get "$item_id" --account "$OP_ACCOUNT" --vault "$vault_id" --reveal --format json)"

# Fields are matched by label anywhere in the item, sections included.
read_field() {
  local field_name="$1"
  local count
  local value
  count="$(jq --arg label "$field_name" '[.fields[]? | select(.label == $label)] | length' <<<"$item_json")"
  if [[ "$count" -gt 1 ]]; then
    echo "Item '$item_title' has $count fields labeled '$field_name'; keep one." >&2
    return 1
  fi
  value="$(
    jq -r --arg label "$field_name" \
      'first(.fields[]? | select(.label == $label) | .value // empty) // empty' <<<"$item_json"
  )"
  if [[ -z "$value" ]]; then
    echo "Field '$field_name' is missing or empty on item '$item_title'." >&2
    return 1
  fi
  printf "%s" "$value"
}

guid_pattern='^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$'

# Catches the usual Entra mix-ups before they surface as a failed release.
check_field() {
  local field_name="$1"
  local value="$2"
  case "$field_name" in
    AZURE_TENANT_ID | AZURE_CLIENT_ID)
      if [[ ! "$value" =~ $guid_pattern ]]; then
        echo "$field_name on item '$item_title' is not a GUID." >&2
        return 1
      fi
      ;;
    AZURE_CLIENT_SECRET)
      if [[ "$value" =~ $guid_pattern ]]; then
        echo "AZURE_CLIENT_SECRET on item '$item_title' looks like the Secret ID;" >&2
        echo "it must be the secret's Value, shown once when the secret is created." >&2
        return 1
      fi
      ;;
  esac
}

# Read everything before uploading anything, so a missing or wrong field
# leaves the environment as it was.
# Indexed arrays only: macOS ships bash 3.2, which has no associative arrays.
read_fields() {
  local field
  local value
  for field in "$@"; do
    value="$(read_field "$field")" || exit 1
    check_field "$field" "$value" || exit 1
    field_values+=("$value")
  done
}
field_values=()
read_fields "${variable_fields[@]}"
variable_values=("${field_values[@]}")
field_values=()
read_fields "${secret_fields[@]}"
secret_values=("${field_values[@]}")

for i in "${!variable_fields[@]}"; do
  gh variable set "${variable_fields[$i]}" --repo "$repo" --env "$environment" --body "${variable_values[$i]}"
  echo "Set variable ${variable_fields[$i]}=${variable_values[$i]} on $repo environment '$environment'."
done

for i in "${!secret_fields[@]}"; do
  printf "%s" "${secret_values[$i]}" | gh secret set "${secret_fields[$i]}" --repo "$repo" --env "$environment"
  echo "Uploaded secret ${secret_fields[$i]} to $repo environment '$environment'."
done

echo "Uploaded all Windows signing settings from 1Password item '$item_title'."
