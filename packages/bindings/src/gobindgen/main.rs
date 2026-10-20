//! Go bindings for the Windows app, generated from the UniFFI metadata in the compiled library.
//!
//! The output is a Go module that opens the library at run time through purego, so the app
//! needs neither cgo nor a C toolchain. purego passes the 24-byte RustBuffer according to each
//! platform's C calling convention, which also lets the same package load the macOS dylib.
mod emit;

use camino::Utf8PathBuf;
use std::fs;
use uniffi_bindgen::{EmptyCrateConfigSupplier, find_components};

const RUNTIME: [(&str, &str); 3] = [
    ("runtime.go", include_str!("go/runtime.go")),
    ("load_windows.go", include_str!("go/load_windows.go")),
    ("load_unix.go", include_str!("go/load_unix.go")),
];

const GO_MOD: &str =
    "module velune/bindings\n\ngo 1.25.0\n\nrequire github.com/ebitengine/purego v0.11.2\n";

fn main() -> emit::Result<()> {
    let mut args = std::env::args().skip(1);
    let (Some(library), Some(out_dir), None) = (args.next(), args.next(), args.next()) else {
        return Err("用法：velune-gobindgen <library> <out-dir>".into());
    };
    let mut components = find_components(&Utf8PathBuf::from(library), &EmptyCrateConfigSupplier)?;
    let index = components
        .iter()
        .position(|component| component.ci.crate_name() == "velune_bindings")
        .ok_or("库中没有 velune_bindings 组件")?;
    let mut ci = components.swap_remove(index).ci;
    ci.derive_ffi_funcs()?;
    let bindings = emit::bindings(&ci)?;

    let out_dir = Utf8PathBuf::from(out_dir);
    fs::create_dir_all(&out_dir)?;
    // The directory holds only generated files; drop Go sources from an older interface.
    for entry in fs::read_dir(&out_dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "go") {
            fs::remove_file(path)?;
        }
    }
    fs::write(out_dir.join("go.mod"), GO_MOD)?;
    fs::write(out_dir.join("bindings.go"), bindings)?;
    for (name, contents) in RUNTIME {
        fs::write(out_dir.join(name), contents)?;
    }
    Ok(())
}
