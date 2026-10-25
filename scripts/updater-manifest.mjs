#!/usr/bin/env node
// Write latest.json, the manifest the in-app updater reads from the latest
// GitHub release (plugins.updater.endpoints in src-tauri/tauri.conf.json).
//
//   node scripts/updater-manifest.mjs --version 0.2.0 \
//     --url https://github.com/<repo>/releases/download/v0.2.0/Otomo_0.2.0_aarch64.app.tar.gz \
//     --signature src-tauri/target/release/bundle/macos/Otomo.app.tar.gz.sig \
//     [--notes notes.md] [--platform darwin-aarch64] --out latest.json
//
// The signature is the updater key's (TAURI_SIGNING_PRIVATE_KEY), made by
// `tauri build` next to the .app.tar.gz; the app checks downloads against
// the public key in tauri.conf.json before swapping anything in.
import { readFileSync, writeFileSync } from "node:fs";
import { parseArgs } from "node:util";

const { values } = parseArgs({
  options: {
    version: { type: "string" },
    url: { type: "string" },
    signature: { type: "string" },
    notes: { type: "string" },
    platform: { type: "string", default: "darwin-aarch64" },
    out: { type: "string" },
  },
});

for (const key of ["version", "url", "signature", "out"]) {
  if (!values[key]) {
    console.error(`missing --${key}`);
    process.exit(1);
  }
}
if (!/^\d+\.\d+\.\d+(-[\w.]+)?$/.test(values.version)) {
  console.error(`not a semver version: ${values.version}`);
  process.exit(1);
}

// A signed tag's message ends in its PGP signature, which is not notes.
const notes = values.notes
  ? readFileSync(values.notes, "utf8").replace(/-----BEGIN PGP SIGNATURE-----[\s\S]*$/, "").trim()
  : "";

const manifest = {
  version: values.version,
  notes,
  pub_date: new Date().toISOString().replace(/\.\d{3}Z$/, "Z"),
  platforms: {
    [values.platform]: {
      signature: readFileSync(values.signature, "utf8").trim(),
      url: values.url,
    },
  },
};

writeFileSync(values.out, JSON.stringify(manifest, null, 2) + "\n");
console.log(`wrote ${values.out} for ${values.version} (${values.platform})`);
