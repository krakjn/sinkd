use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_ALLOW_CROSS");

    let reflect = pkg_config::Config::new()
        .cargo_metadata(false)
        .probe("reflect")
        .unwrap_or_else(|e| {
            panic!(
                "pkg-config could not find reflect ({e}). Install the libreflect package \
so reflect.pc is on the default search path."
            );
        });

    let header = reflect
        .include_paths
        .iter()
        .map(|dir| dir.join("reflect.h"))
        .find(|path| path.exists())
        .unwrap_or_else(|| {
            panic!(
                "reflect.h was not in the include path from reflect.pc ({:?})",
                reflect.include_paths
            );
        });

    let header_text = std::fs::read_to_string(&header).expect("read reflect.h");
    if !header_text
        .lines()
        .any(|line| line.trim() == "#define REFLECT_API_VERSION 4")
    {
        panic!("reflect.h ABI is not 4; sinkd was written against REFLECT_API_VERSION 4");
    }

    let mut builder = bindgen::Builder::default()
        .header(header.to_string_lossy())
        .allowlist_function("reflect_.*")
        .allowlist_type("reflect_.*")
        .allowlist_var("REFLECT_.*")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()));
    for dir in &reflect.include_paths {
        builder = builder.clang_arg(format!("-I{}", dir.display()));
    }
    let bindings = builder.generate().expect("bindgen reflect.h");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    bindings
        .write_to_file(out.join("bindings.rs"))
        .expect("write bindings");
    // reflect.pc says -L/usr/lib. Passing that through makes -lc bind glibc's libc.a
    // on Arch, where the rustc sysroot is /usr. Link the archive from OUT_DIR only.
    let src = reflect
        .link_paths
        .iter()
        .map(|dir| dir.join("libreflect.a"))
        .find(|path| path.exists())
        .unwrap_or_else(|| panic!("libreflect.a was not in {:?}", reflect.link_paths));
    println!("cargo:rerun-if-changed={}", src.display());
    std::fs::copy(&src, out.join("libreflect.a")).expect("copy libreflect.a");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=reflect");
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    assert!(
        os != "linux" || env == "musl",
        "sinkd links the musl libreflect archive; build for x86_64-unknown-linux-musl",
    );
}
