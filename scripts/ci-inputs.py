#!/usr/bin/env python3
"""Select the Rust/FFI checks for the pull request's actual changed inputs."""
import subprocess
import sys


def requires_rust(names):
    rust_files = {'Cargo.lock', 'Cargo.toml', 'rust-toolchain.toml', 'clippy.toml',
                  '.github/workflows/pr-checks.yml', 'scripts/ci-inputs.py',
                  'scripts/build-support.py',
                  'scripts/run-migrations.py'}
    # Crates embed Python, JavaScript, Markdown and fixture bytes as well as
    # Rust. File extensions cannot identify their compile/test inputs. Keep
    # their owner trees together; the acceptance suite consumes the built FFI.
    rust_trees = ('packages/', 'tools/', 'third-party/', '.cargo/', 'tests/acceptance/',
                  'scripts/migrations/')
    return any(
        name.endswith('.rs') or name in rust_files or name.endswith('/Cargo.toml')
        or name.startswith(rust_trees)
        for name in names
    )


def changed_paths(base):
    return subprocess.check_output(
        ['git', 'diff', '--name-only', '--no-renames', '-z', f'{base}...HEAD'], text=True,
    ).split('\0')


def main():
    base, output = sys.argv[1:]
    with open(output, 'a') as stream:
        stream.write(f'rust={str(requires_rust(changed_paths(base))).lower()}\n')


if __name__ == '__main__':
    main()
