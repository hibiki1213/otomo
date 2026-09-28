use std::path::PathBuf;

fn main() {
    build_sfizz();
    tauri_build::build()
}

/// Build the vendored sfizz SFZ sampler (vendor/sfizz, 1.2.3 release tarball
/// minus tests/benchmarks) as static libraries and link them.
fn build_sfizz() {
    let src = PathBuf::from("vendor/sfizz");
    // Only the top-level CMakeLists: the configure step writes files into
    // vendor/sfizz/src, so watching that tree would rebuild on every build.
    println!("cargo:rerun-if-changed=vendor/sfizz/CMakeLists.txt");

    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    // 1.2.3 adds -mfpu=neon for any "arm*" processor name, which Apple clang
    // rejects on arm64; naming it aarch64 avoids that path.
    let processor = if arch == "aarch64" { "aarch64" } else { "x86_64" };

    let dst = cmake::Config::new(&src)
        // A debug sfizz is far too slow for realtime use.
        .profile("Release")
        .define("SFIZZ_SHARED", "OFF")
        .define("SFIZZ_JACK", "OFF")
        .define("SFIZZ_RENDER", "OFF")
        .define("SFIZZ_TESTS", "OFF")
        .define("SFIZZ_BENCHMARKS", "OFF")
        .define("SFIZZ_DEMOS", "OFF")
        .define("SFIZZ_DEVTOOLS", "OFF")
        .define("SFIZZ_USE_SNDFILE", "OFF")
        .define("ENABLE_LTO", "OFF")
        .define("PROJECT_SYSTEM_PROCESSOR", processor)
        .define("CMAKE_DISABLE_FIND_PACKAGE_OpenMP", "ON")
        // Newer clang makes the bundled atomic_queue's `Base::template f(...)`
        // (no template argument list) a hard error by default.
        .cxxflag("-Wno-missing-template-arg-list-after-template-kw")
        .build_target("sfizz_static")
        .build();

    let lib = dst.join("build").join("library").join("lib");
    println!("cargo:rustc-link-search=native={}", lib.display());
    let mut libs: Vec<String> = std::fs::read_dir(&lib)
        .expect("sfizz build produced no libraries")
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| Some(n.strip_prefix("lib")?.strip_suffix(".a")?.to_owned()))
        .collect();
    libs.sort();
    for name in libs {
        println!("cargo:rustc-link-lib=static={name}");
    }
    println!("cargo:rustc-link-lib=c++");
}
