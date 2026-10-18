#!/usr/bin/env python3
"""Reconstruct the runtime-only iroh 1.3.0 baseline from its verified crate archive."""
import hashlib
import re
import subprocess
import sys
import tarfile
from pathlib import Path

CHECKSUM = '885787b892b5e2507c701f132ecbd45d2bad4bbb75157a19427087c16dadb833'

def masked(source):
    # Preserve offsets while hiding Rust comments, strings, and character literals.
    out = list(source)
    token = re.compile(r'//[^\n]*|/\*|(?:br|r)(?P<hash>\#*)"|b?"|b?\'(?:\\.|[^\\\'\n])\'')
    cursor = 0
    while match := token.search(source, cursor):
        start, end = match.span()
        value = match.group()
        if value == '/*':
            depth = 1
            while depth:
                nxt = re.search(r'/\*|\*/', source[end:])
                if nxt is None:
                    raise ValueError('unclosed comment')
                end += nxt.end()
                depth += 1 if nxt.group() == '/*' else -1
        elif match.group('hash') is not None:
            end = source.index('"' + match.group('hash'), end) + 1 + len(match.group('hash'))
        elif value.endswith('"'):
            while True:
                if source[end] == '\\':
                    end += 2
                elif source[end] == '"':
                    end += 1
                    break
                else:
                    end += 1
        for pos in range(start, end):
            if out[pos] != '\n':
                out[pos] = ' '
        cursor = end
    return ''.join(out)

def runtime_only(source):
    pattern = re.compile(r'(?m)^[ \t]*#\[cfg\([^\n]*(?:\btest\b|"test-utils")[^\n]*\)\]\n')
    while match := pattern.search(source):
        # Every matching cfg is test-only, or a test-utils export removed with that feature.
        start, cursor = match.span()
        mask = masked(source)
        depth = 0
        body = False
        angles = 0
        while cursor < len(source):
            char = mask[cursor]
            if char == '<' and not body:
                angles += 1
            elif char == '>' and not body and angles and mask[cursor - 1] not in '-=':
                angles -= 1
            elif char in '([{':
                depth += 1
                if char == '{' and depth == 1:
                    body = True
            elif char in ')]}':
                depth -= 1
                if body and depth == 0:
                    cursor += 1
                    if cursor < len(source) and source[cursor] in ',;':
                        cursor += 1
                    break
            elif char in ',;' and depth == 0 and angles == 0:
                cursor += 1
                break
            cursor += 1
        # Remove the deleted item's immediately preceding documentation too.
        while start > 0:
            previous = source.rfind('\n', 0, start - 1) + 1
            if source[previous:start].lstrip().startswith('///'):
                start = previous
            else:
                break
        source = source[:start] + source[cursor:]
    return source

def main():
    archive, target = map(Path, sys.argv[1:])
    if hashlib.sha256(archive.read_bytes()).hexdigest() != CHECKSUM:
        raise ValueError('crate checksum mismatch')
    if target.exists() and any(target.iterdir()):
        raise ValueError('target must be a new or empty directory')
    target.mkdir(parents=True, exist_ok=True)
    with tarfile.open(archive) as crate:
        for member in crate.getmembers():
            relative = Path(member.name).relative_to('iroh-1.3.0')
            if not member.isfile():
                continue
            if relative.parts[0] == 'src' and not str(relative).startswith(('src/test_utils.', 'src/test_utils/')):
                content = runtime_only(crate.extractfile(member).read().decode())
            elif str(relative) in ('Cargo.toml', 'build.rs', 'LICENSE-BSD3', '.cargo_vcs_info.json'):
                content = crate.extractfile(member).read().decode()
            else:
                continue
            if str(relative) == 'Cargo.toml':
                sections = re.split(r'(?m)(?=^\[)', content)
                sections = [s for s in sections if not re.match(r'\[\[(?:example|test|bench)\]\]', s) and not ('dev-dependencies' in s.split('\n')[0]) and not s.startswith(('[dependencies.axum]', '[dependencies.simple-dns]'))]
                content = ''.join(sections)
                content = re.sub(r'(?m)^test-utils = \[\n.*?\n\]\n', '', content, flags=re.S)
                content = content.replace('path = "src/lib.rs"', 'path = "src/lib.rs"\ndoctest = false')
            destination = target / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_text(content)
    subprocess.run(['rustfmt', '--edition', '2024', str(target / 'src/lib.rs')], check=True)

if __name__ == '__main__':
    main()
