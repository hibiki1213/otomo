#!/usr/bin/env node
// Write src-tauri/resources/THIRD_PARTY_LICENSES.html, the licenses and
// notices of everything the app bundle ships that is not Otomo's own: Rust
// crates (via cargo-about), the JavaScript packages in the UI bundle, the
// vendored sfizz sampler and its libraries, the fonts the score engravers
// embed, and the bundled sounds. The app opens it from the sidebar.
//
// Needs cargo-about (`cargo install cargo-about --features cli`). Run after
// `pnpm sounds`: the SoundFont's license file comes with it.
//
//   --if-missing  do nothing when the file already exists (for app:dev)
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
const out = path.join(root, "src-tauri/resources/THIRD_PARTY_LICENSES.html");

if (process.argv.includes("--if-missing") && fs.existsSync(out)) process.exit(0);

const read = (p) => fs.readFileSync(path.resolve(root, p), "utf8").replace(/\r\n/g, "\n").trimEnd();
const sfizz = "src-tauri/vendor/sfizz";
const lic = "src-tauri/licenses";

// Not seen by either package manager: C/C++ sources built into the app,
// fonts embedded inside other packages, and data files.
const components = [
  {
    group: "Audio engine (C/C++, statically linked)",
    items: [
      { name: "sfizz 1.2.3", license: "BSD-2-Clause", url: "https://sfz.tools/sfizz", texts: [`${sfizz}/LICENSE`] },
      { name: "Abseil", license: "Apache-2.0", url: "https://abseil.io", texts: [`${sfizz}/external/abseil-cpp/LICENSE`] },
      { name: "atomic_queue", license: "MIT", url: "https://github.com/max0x7ba/atomic_queue", texts: [`${sfizz}/external/atomic_queue/LICENSE`] },
      { name: "Cephes (sfizz wrapper)", license: "BSD-3-Clause", url: "https://www.netlib.org/cephes/", texts: [`${sfizz}/external/cephes/LICENSE.txt`] },
      { name: "cpuid", license: "BSD-3-Clause", url: "https://github.com/steinwurf/cpuid", texts: [`${sfizz}/src/external/cpuid/LICENSE.rst`] },
      { name: "dr_libs", license: "Unlicense OR MIT-0", url: "https://github.com/mackron/dr_libs", texts: [`${sfizz}/external/st_audiofile/thirdparty/dr_libs/LICENSE`] },
      { name: "ghc::filesystem", license: "MIT", url: "https://github.com/gulrak/filesystem", texts: [`${sfizz}/external/filesystem/LICENSE`] },
      { name: "HIIR", license: "WTFPL", url: "http://ldesoras.free.fr/prod.html", texts: [`${sfizz}/src/external/hiir/license.txt`] },
      { name: "jsl", license: "BSL-1.0", url: "https://github.com/jpcima/jsl", texts: [`${sfizz}/external/jsl/LICENSE.md`] },
      { name: "KISS FFT", license: "BSD-3-Clause", url: "https://github.com/mborgerding/kissfft", texts: [`${sfizz}/src/external/kiss_fft/COPYING`] },
      { name: "libaiff", license: "MIT", texts: [`${sfizz}/external/st_audiofile/thirdparty/libaiff/LICENSE`] },
      { name: "pugixml", license: "MIT", url: "https://pugixml.org", texts: [`${sfizz}/src/external/pugixml/LICENSE.md`] },
      { name: "SIMD Everywhere", license: "MIT", url: "https://github.com/simd-everywhere/simde", texts: [`${sfizz}/external/simde/COPYING`] },
      { name: "spline", license: "BSD-3-Clause", texts: [`${sfizz}/src/external/spline/LICENSE`] },
      { name: "st_audiofile", license: "BSD-2-Clause", url: "https://github.com/jpcima/st_audiofile", texts: [`${sfizz}/external/st_audiofile/LICENSE.md`] },
      { name: "stb_vorbis", license: "MIT OR Unlicense", url: "https://github.com/nothings/stb", texts: [`${sfizz}/external/st_audiofile/thirdparty/stb_vorbis/LICENSE`] },
      { name: "Surge tuning library", license: "MIT", url: "https://github.com/surge-synthesizer/tuning-library", texts: [`${sfizz}/src/external/tunings/LICENSE.md`] },
      {
        name: "ThreadPool",
        license: "Zlib",
        url: "https://github.com/progschj/ThreadPool",
        copyright: "Copyright (c) 2012 Jakob Progsch, Václav Zeman",
        texts: [`${lic}/Zlib.txt`],
      },
      { name: "WavPack", license: "BSD-3-Clause", url: "https://www.wavpack.com", texts: [`${sfizz}/external/st_audiofile/thirdparty/wavpack/license.txt`] },
    ],
  },
  {
    group: "Fonts (embedded in Verovio and VexFlow, and in exported PDFs)",
    items: [
      {
        name: "Leipzig",
        license: "OFL-1.1",
        url: "https://github.com/rism-digital/leipzig",
        copyright: 'Copyright (c) 2014, Etienne Darbellay, Jean-Francois Marti and Laurent Pugin, with Reserved Font Name "Leipzig".',
        texts: [`${lic}/OFL-1.1.txt`],
      },
      {
        name: "Bravura, Petaluma, Petaluma Script, Academico",
        license: "OFL-1.1",
        url: "https://github.com/steinbergmedia/bravura",
        copyright:
          'Copyright (c) 2018-2022, Steinberg Media Technologies GmbH (http://www.steinberg.net/), with Reserved Font Names "Bravura", "Petaluma" and "Academico".',
        texts: [`${lic}/OFL-1.1.txt`],
      },
      {
        name: "Leland",
        license: "OFL-1.1",
        url: "https://github.com/MuseScoreFonts/Leland",
        copyright: 'Copyright (c) MuseScore BVBA (http://www.musescore.org/), with Reserved Font Name "Leland".',
        texts: [`${lic}/OFL-1.1.txt`],
      },
      {
        name: "Gootville",
        license: "OFL-1.1",
        url: "https://github.com/musescore/MuseScore/tree/master/fonts/gootville",
        copyright: "Designed by Grzegorz Pruchniakowski, based on Bravura by Daniel Spreadbury (Steinberg).",
        texts: [`${lic}/OFL-1.1.txt`],
      },
      {
        name: "Gonville",
        license: "Public domain",
        url: "https://www.chiark.greenend.org.uk/~sgtatham/gonville/",
        copyright: "By Simon Tatham. No copyright is claimed on this font file.",
        texts: [],
      },
      {
        name: "Liberation Serif 1.04",
        license: "GPL-2.0-or-later WITH Font-exception-2.0",
        url: "https://github.com/liberationfonts/liberation-fonts",
        copyright:
          "Digitized data copyright (c) 2007 Ascender Corporation. Designed by Steve Matteson.\n\n" +
          "As a special exception, if you create a document which uses this font, and embed this font or unaltered " +
          "portions of this font into the document, this font does not by itself cause the resulting document to be " +
          "covered by the GNU General Public License. This exception does not however invalidate any other reasons " +
          "why the document might be covered by the GNU General Public License.",
        texts: [`${lic}/GPL-2.0-or-later.txt`],
      },
    ],
  },
  {
    group: "Sounds",
    items: [
      {
        name: "GeneralUser GS 1.471",
        license: "GeneralUser GS License v2.0",
        url: "https://schristiancollins.com/generaluser",
        copyright: "By S. Christian Collins.",
        texts: ["src-tauri/resources/GeneralUser-LICENSE.txt"],
      },
      {
        name: "SADIE II HRTF Database (KU100)",
        license: "Apache-2.0",
        url: "https://www.york.ac.uk/sadie-project/database.html",
        texts: ["src-tauri/assets/NOTICE-SADIE.txt", "src-tauri/assets/LICENSE-APACHE-2.0.txt"],
      },
    ],
  },
];

// npm packages that ship without a license file.
const npmFallback = {
  // MIT per its package.json; the repository has no license file either.
  smplr: {
    copyright: "Copyright (c) danigb",
    texts: [`${lic}/MIT.txt`],
  },
  verovio: {
    note:
      "Otomo bundles the unmodified Verovio WebAssembly build from npm. Its source is at " +
      "https://github.com/rism-digital/verovio; to use a modified Verovio, replace the package and rebuild Otomo from source.",
    texts: [`${lic}/LGPL-3.0-or-later.txt`, `${lic}/GPL-3.0-or-later.txt`],
  },
};

const ours = new Set(["otomo", "otomo-mcp", "otomo-score", "otomo-synth"]);

function rustCrates() {
  const json = execFileSync(
    "cargo",
    ["about", "generate", "--workspace", "--fail", "--format", "json", "-c", "about.toml"],
    {
      cwd: path.join(root, "src-tauri"),
      encoding: "utf8",
      maxBuffer: 256 * 1024 * 1024,
      stdio: ["ignore", "pipe", "inherit"],
      env: { ...process.env, PATH: `${process.env.HOME}/.cargo/bin:${process.env.PATH}` },
    },
  );
  const about = JSON.parse(json);
  const licenses = about.licenses
    .map((l) => ({
      id: l.id,
      name: l.name,
      text: l.text.trimEnd(),
      crates: l.used_by.map((u) => u.crate).filter((c) => !ours.has(c.name)),
    }))
    .filter((l) => l.crates.length > 0);
  // Apache-2.0 asks for the NOTICE files of what is redistributed, which
  // cargo-about leaves out.
  const notices = [];
  for (const { package: p } of about.crates) {
    if (ours.has(p.name)) continue;
    const dir = path.dirname(p.manifest_path);
    for (const f of fs.readdirSync(dir)) {
      if (/^notice/i.test(f) && fs.statSync(path.join(dir, f)).isFile()) {
        notices.push({ crate: p, text: fs.readFileSync(path.join(dir, f), "utf8").trimEnd() });
      }
    }
  }
  return { licenses, notices, count: about.crates.filter((c) => !ours.has(c.package.name)).length };
}

function npmPackages() {
  const json = execFileSync("pnpm", ["licenses", "list", "--prod", "--json"], {
    cwd: root,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
  });
  const pkgs = Object.values(JSON.parse(json)).flat();
  return pkgs
    .map((p) => {
      const dir = p.paths[0];
      const files = fs
        .readdirSync(dir)
        .filter((f) => /^(licen[cs]e|copying|notice)/i.test(f))
        .sort();
      const fallback = npmFallback[p.name];
      const texts = files.map((f) => fs.readFileSync(path.join(dir, f), "utf8").trimEnd());
      if (texts.length === 0 && !fallback) {
        throw new Error(`${p.name} has no license file; add it to npmFallback in ${path.relative(root, import.meta.filename)}`);
      }
      return {
        name: p.name,
        version: p.versions.join(", "),
        license: p.license,
        url: p.homepage,
        author: p.author,
        note: fallback?.note,
        texts:
          texts.length > 0
            ? texts
            : fallback.texts.map((t) => read(t).replace("Copyright (c) <year> <copyright holders>", fallback.copyright)),
      };
    })
    .sort((a, b) => a.name.localeCompare(b.name));
}

const esc = (s) =>
  String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
const link = (url, label) => (url ? `<a href="${esc(url)}">${esc(label ?? url)}</a>` : esc(label ?? ""));
const pre = (t) => `<pre>${esc(t)}</pre>`;
const slug = (s) => s.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "");

function render({ rust, npm }) {
  const sections = [];
  const toc = [];

  for (const g of components) {
    const id = slug(g.group);
    toc.push(`<li><a href="#${id}">${esc(g.group)}</a></li>`);
    sections.push(
      `<h2 id="${id}">${esc(g.group)}</h2>` +
        g.items
          .map(
            (c) =>
              `<section><h3>${link(c.url, c.name)} <span class="lic">${esc(c.license)}</span></h3>` +
              (c.copyright ? `<p class="copy">${esc(c.copyright).replace(/\n/g, "<br>")}</p>` : "") +
              c.texts.map((t) => pre(read(t))).join("") +
              `</section>`,
          )
          .join(""),
    );
  }

  toc.push(`<li><a href="#javascript">JavaScript packages</a> (${npm.length})</li>`);
  sections.push(
    `<h2 id="javascript">JavaScript packages</h2>` +
      npm
        .map(
          (p) =>
            `<section><h3>${link(p.url, `${p.name} ${p.version}`)} <span class="lic">${esc(p.license)}</span></h3>` +
            (p.author ? `<p class="copy">${esc(p.author)}</p>` : "") +
            (p.note ? `<p>${esc(p.note)}</p>` : "") +
            p.texts.map(pre).join("") +
            `</section>`,
        )
        .join(""),
  );

  toc.push(`<li><a href="#rust">Rust crates</a> (${rust.count})</li>`);
  sections.push(
    `<h2 id="rust">Rust crates</h2>` +
      rust.licenses
        .map(
          (l) =>
            `<section><h3>${esc(l.name)} <span class="lic">${esc(l.id)}</span></h3>` +
            `<p class="used">Used by: ${l.crates
              .map((c) => link(c.repository ?? `https://crates.io/crates/${c.name}`, `${c.name} ${c.version}`))
              .join(", ")}</p>` +
            pre(l.text) +
            `</section>`,
        )
        .join("") +
      (rust.notices.length > 0
        ? `<h3 id="rust-notices">NOTICE files</h3>` +
          rust.notices.map((n) => `<section><h4>${esc(`${n.crate.name} ${n.crate.version}`)}</h4>${pre(n.text)}</section>`).join("")
        : ""),
  );

  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Otomo — Third-Party Licenses</title>
<style>
  :root { color-scheme: light dark; --bg: #fff; --fg: #1d1f24; --muted: #6b7280; --code: #f4f5f7; --line: #e5e7eb; --accent: #0a6cff; }
  @media (prefers-color-scheme: dark) { :root { --bg: #1c1c1e; --fg: #e8e8ea; --muted: #9a9aa2; --code: #26262a; --line: #34343a; --accent: #5ea2ff; } }
  body { margin: 0 auto; max-width: 860px; padding: 32px 20px 80px; background: var(--bg); color: var(--fg);
    font: 14px/1.55 -apple-system, BlinkMacSystemFont, "Helvetica Neue", sans-serif; }
  a { color: var(--accent); text-decoration: none; } a:hover { text-decoration: underline; }
  h1 { font-size: 24px; margin: 0 0 6px; } h2 { font-size: 19px; margin: 40px 0 8px; padding-top: 12px; border-top: 1px solid var(--line); }
  h3 { font-size: 15px; margin: 22px 0 6px; } h4 { font-size: 13px; margin: 16px 0 6px; }
  .lic { font-weight: 500; font-size: 12px; color: var(--muted); margin-left: 6px; }
  .copy, .used, .intro { color: var(--muted); } .used { font-size: 12.5px; }
  pre { background: var(--code); padding: 12px 14px; border-radius: 8px; font: 11.5px/1.5 ui-monospace, Menlo, monospace;
    white-space: pre-wrap; overflow-wrap: anywhere; max-height: 360px; overflow: auto; }
</style>
</head>
<body>
<h1>Third-Party Licenses</h1>
<p class="intro">Otomo is MIT-licensed open source software (${link("https://github.com/hibiki1213/otomo")}).
It is built on the work below; their licenses and notices follow.</p>
<ul>${toc.join("")}</ul>
${sections.join("\n")}
</body>
</html>
`;
}

const html = render({ rust: rustCrates(), npm: npmPackages() });
fs.mkdirSync(path.dirname(out), { recursive: true });
fs.writeFileSync(out, html);
console.log(`wrote ${path.relative(root, out)} (${Math.round(html.length / 1024)} KB)`);
