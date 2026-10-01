#!/usr/bin/env node
// Keeps the app version in sync across every file that carries it.
//
//   node scripts/version.mjs 0.2.0         bump everything to 0.2.0
//   node scripts/version.mjs --check v0.2.0   exit 1 unless every file already says 0.2.0 (used by the release workflow)
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const files = {
  packageJson: join(root, "package.json"),
  tauriConf: join(root, "src-tauri/tauri.conf.json"),
  appCargo: join(root, "src-tauri/Cargo.toml"),
  coreCargo: join(root, "crates/fillerncut-core/Cargo.toml"),
};

const readJson = (p) => JSON.parse(readFileSync(p, "utf8"));
const cargoVersion = (p) => readFileSync(p, "utf8").match(/^version\s*=\s*"([^"]+)"/m)?.[1];

function current() {
  return {
    "package.json": readJson(files.packageJson).version,
    "src-tauri/tauri.conf.json": readJson(files.tauriConf).version,
    "src-tauri/Cargo.toml": cargoVersion(files.appCargo),
    "crates/fillerncut-core/Cargo.toml": cargoVersion(files.coreCargo),
  };
}

const args = process.argv.slice(2);
const semver = /^\d+\.\d+\.\d+(-[0-9A-Za-z.-]+)?$/;

if (args[0] === "--check") {
  const want = (args[1] ?? "").replace(/^v/, "");
  if (!semver.test(want)) {
    console.error(`"${args[1]}" is not a version tag like v1.2.3`);
    process.exit(1);
  }
  const bad = Object.entries(current()).filter(([, v]) => v !== want);
  if (bad.length) {
    console.error(`Tag is v${want} but:\n${bad.map(([f, v]) => `  ${f} says ${v}`).join("\n")}\nRun: node scripts/version.mjs ${want}`);
    process.exit(1);
  }
  console.log(`All files are at ${want}`);
  process.exit(0);
}

const next = (args[0] ?? "").replace(/^v/, "");
if (!semver.test(next)) {
  console.error("Usage: node scripts/version.mjs <version>   e.g. 0.2.0");
  process.exit(1);
}
for (const p of [files.packageJson, files.tauriConf]) {
  const j = readJson(p);
  j.version = next;
  writeFileSync(p, JSON.stringify(j, null, 2) + "\n");
}
for (const p of [files.appCargo, files.coreCargo]) {
  writeFileSync(p, readFileSync(p, "utf8").replace(/^version\s*=\s*"[^"]+"/m, `version = "${next}"`));
}
console.log(`Version set to ${next}. Next:
  cargo update -w            # refresh Cargo.lock
  git commit -am "Release v${next}" && git tag v${next} && git push && git push --tags`);
