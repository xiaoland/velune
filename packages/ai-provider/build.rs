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
    let (mode, artifact) = if os == "android" {
        ("-buildmode=c-shared", "libvelune_relaykit.so")
    } else {
        ("-buildmode=c-archive", "libvelune_relaykit.a")
    };
    let status = Command::new(env::var_os("GO").unwrap_or_else(|| "go".into()))
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
    } else if os == "windows" {
        for library in ["ws2_32", "winmm", "ntdll", "bcrypt"] {
            println!("cargo:rustc-link-lib={library}");
        }
    } else if os != "android" {
        for library in ["pthread", "dl", "m", "resolv"] {
            println!("cargo:rustc-link-lib={library}");
        }
    }
}
