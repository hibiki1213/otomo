# Third-party notices

Otomo is released under the [MIT License](LICENSE). It also ships other people's work, each under its own license. This page lists the parts that matter most. The app carries the complete list with every license text, and you can open it from **Licenses** at the foot of the sidebar. To produce that list from source, run `pnpm licenses` ([scripts/generate-licenses.mjs](scripts/generate-licenses.mjs)).

## Engraving

| Component | License | Notes |
| --- | --- | --- |
| [Verovio](https://github.com/rism-digital/verovio) 6.3.0 | LGPL-3.0-or-later | The unmodified WebAssembly build from npm, loaded in its own worker. To use a modified Verovio, replace the package and rebuild Otomo. |
| [VexFlow](https://github.com/vexflow/vexflow) 5 | MIT | |

## Fonts

These fonts are embedded in Verovio and VexFlow. The music fonts are also subset into the PDFs Otomo exports.

| Font | License | Copyright |
| --- | --- | --- |
| [Leipzig](https://github.com/rism-digital/leipzig) | SIL OFL 1.1 | Etienne Darbellay, Jean-François Marti, Laurent Pugin |
| [Bravura](https://github.com/steinbergmedia/bravura), [Petaluma](https://github.com/steinbergmedia/petaluma), Petaluma Script, Academico | SIL OFL 1.1 | Steinberg Media Technologies GmbH |
| [Leland](https://github.com/MuseScoreFonts/Leland) | SIL OFL 1.1 | MuseScore BVBA |
| Gootville | SIL OFL 1.1 | Grzegorz Pruchniakowski |
| Gonville | Public domain | Simon Tatham |
| Liberation Serif 1.04 | GPL-2.0-or-later with font exception | Ascender Corporation (designed by Steve Matteson) |

## Audio

| Component | License | Notes |
| --- | --- | --- |
| [sfizz](https://github.com/sfztools/sfizz) 1.2.3 | BSD-2-Clause | Vendored in `src-tauri/vendor/sfizz` and statically linked. It brings in Abseil (Apache-2.0), WavPack, KISS FFT, cpuid and spline (BSD-3-Clause), pugixml, atomic_queue, ghc::filesystem, SIMDe, libaiff and the Surge tuning library (MIT), dr_libs and stb_vorbis (MIT or Unlicense), jsl (BSL-1.0), ThreadPool (Zlib), and HIIR (WTFPL). |
| [RustySynth](https://github.com/sinshu/rustysynth) | MIT | Vendored in `src-tauri/crates/rustysynth`. |
| [GeneralUser GS](https://schristiancollins.com/generaluser.php) 1.471 | GeneralUser GS License v2.0 | The bundled SoundFont, by S. Christian Collins. Fetched at build time by `pnpm sounds`. |
| [SADIE II](https://www.york.ac.uk/sadie-project/database.html) HRTF (KU100) | Apache-2.0 | University of York. Directions subsampled and repacked; see [src-tauri/assets/NOTICE-SADIE.txt](src-tauri/assets/NOTICE-SADIE.txt). |

Sample libraries that you download from inside the app are not part of Otomo. Each one keeps its own license, and the app shows it next to the download button.

## Packages

The app also includes about 350 Rust crates and a dozen npm packages. They are mostly under MIT or Apache-2.0, with a few under BSD, Zlib, Unicode-3.0, MPL-2.0 or the Unlicense. [src-tauri/about.toml](src-tauri/about.toml) lists the licenses Rust dependencies may use. CI fails if a dependency's license is not on that list.
