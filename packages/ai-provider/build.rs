use std::{env, path::PathBuf, process::Command};

fn main() {
    for path in [
        "relaykit-bridge/main.go",
        "relaykit-bridge/go.mod",
        "relaykit-bridge/go.sum",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    for key in [
        "GO",
        "CC",
        "VELUNE_RELAYKIT_CC",
        "CGO_CFLAGS",
        "CGO_LDFLAGS",
        "SDKROOT",
        "MACOSX_DEPLOYMENT_TARGET",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    let os = env::var("CARGO_CFG_TARGET_OS").expect("Cargo target OS");
    let arch = env::var("CARGO_CFG_TARGET_ARCH").expect("Cargo target architecture");
    let go_arch = match arch.as_str() {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        "x86" => "386",
        "arm" => "arm",
        "riscv64" => "riscv64",
        other => panic!("RelayKit Go bridge has no configured architecture for {other}"),
    };
    let go_os = match os.as_str() {
        "macos" => "darwin",
        "ios" => "ios",
        "linux" => "linux",
        "android" => "android",
        "windows" => "windows",
        "freebsd" => "freebsd",
        other => panic!("RelayKit Go bridge has no Go target mapping for {other}"),
    };
    // A MinGW static archive does not link reliably into MSVC binaries, so
    // Windows uses a DLL, bound by raw-dylib in the bridge module.
    let (mode, artifact) = match os.as_str() {
        "android" => ("-buildmode=c-shared", "libvelune_relaykit.so"),
        "windows" => ("-buildmode=c-shared", "velune_relaykit.dll"),
        _ => ("-buildmode=c-archive", "libvelune_relaykit.a"),
    };
    // The build runs in a console, not in the GUI host.
    #[allow(clippy::disallowed_methods)]
    let mut command = Command::new(env::var_os("GO").unwrap_or_else(|| "go".into()));
    if os == "windows" {
        // Cargo's CC may name MSVC; cgo needs a MinGW compiler.
        let compiler = env::var_os("VELUNE_RELAYKIT_CC")
            .unwrap_or_else(|| format!("{arch}-w64-mingw32-clang").into());
        command.env("CC", compiler);
    }
    let status = command
        .current_dir("relaykit-bridge")
        .env("GOWORK", "off")
        .env("CGO_ENABLED", "1")
        .env("GOOS", go_os)
        .env("GOARCH", go_arch)
        .args(["build", "-mod=readonly", "-trimpath", mode, "-o"])
        .arg(out.join(artifact))
        .arg(".")
        .status()
        .expect("RelayKit requires Go 1.25.1+ and a target C compiler");
    assert!(status.success(), "RelayKit bridge build failed");
    if os == "windows" {
        // Windows finds the DLL beside the executable: OUT_DIR is
        // <profile>/build/<package>/out, so place a copy in <profile>.
        let profile = out.ancestors().nth(3).expect("Cargo profile directory");
        std::fs::copy(out.join(artifact), profile.join(artifact))
            .expect("RelayKit DLL copied beside the executables");
        return;
    }
    println!("cargo:rustc-link-search=native={}", out.display());
    if os == "android" {
        println!("cargo:rustc-link-lib=dylib=velune_relaykit");
    } else {
        println!("cargo:rustc-link-lib=static=velune_relaykit");
    }
    if os == "macos" || os == "ios" {
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=Security");
        println!("cargo:rustc-link-lib=resolv");
    } else if os != "android" {
        for library in ["pthread", "dl", "m", "resolv"] {
            println!("cargo:rustc-link-lib={library}");
        }
    }
}
