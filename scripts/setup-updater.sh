#!/usr/bin/env bash
# One-time setup of in-app updates.
#
# Auto-update downloads a new version from GitHub Releases and installs it. Tauri only installs
# updates whose signature matches the public key built into the app, so you need a key pair:
#   • the PUBLIC key goes into src-tauri/tauri.conf.json (this script does that), and
#   • the PRIVATE key + its password become GitHub secrets so the release workflow can sign builds.
#
#   bash scripts/setup-updater.sh                 # key at ~/.tauri/fillerncut.key
#   bash scripts/setup-updater.sh /path/to/key
#
# Keep the private key safe and never commit it. If you lose it, installed apps can't be updated.
set -euo pipefail
cd "$(dirname "$0")/.."

KEY="${1:-$HOME/.tauri/fillerncut.key}"
CONF="src-tauri/tauri.conf.json"
mkdir -p "$(dirname "$KEY")"

if [[ -f "$KEY" ]]; then
  echo "Using existing key: $KEY"
  PASSWORD="${TAURI_SIGNING_PRIVATE_KEY_PASSWORD:-}"
else
  read -r -s -p "Choose a password for the new signing key (empty = none): " PASSWORD; echo
  npx tauri signer generate --ci -w "$KEY" -p "$PASSWORD" >/dev/null
  echo "Created $KEY"
fi
[[ -f "$KEY.pub" ]] || { echo "Missing $KEY.pub" >&2; exit 1; }

# Put the public key into tauri.conf.json.
KEY_PUB="$KEY.pub" CONF="$CONF" node -e '
  const fs = require("fs");
  const conf = JSON.parse(fs.readFileSync(process.env.CONF, "utf8"));
  conf.plugins.updater.pubkey = fs.readFileSync(process.env.KEY_PUB, "utf8").trim();
  fs.writeFileSync(process.env.CONF, JSON.stringify(conf, null, 2) + "\n");
'
echo "Public key written to $CONF — commit that change."

echo
if command -v gh >/dev/null && gh auth status >/dev/null 2>&1; then
  read -r -p "Upload the private key to this repo's GitHub secrets now? [y/N] " yn
  if [[ "$yn" =~ ^[Yy]$ ]]; then
    gh secret set TAURI_SIGNING_PRIVATE_KEY < "$KEY"
    gh secret set TAURI_SIGNING_PRIVATE_KEY_PASSWORD --body "$PASSWORD"
    echo "Secrets uploaded."
    exit 0
  fi
fi
cat <<MSG
Add these two secrets in GitHub → Settings → Secrets and variables → Actions:
  TAURI_SIGNING_PRIVATE_KEY           = contents of $KEY
  TAURI_SIGNING_PRIVATE_KEY_PASSWORD  = the password you chose (empty is fine if you chose none)
MSG
