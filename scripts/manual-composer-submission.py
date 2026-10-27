#!/usr/bin/env python3
"""Explicitly check the desktop composer action matrix without opening user data.

Compiles the actual platform decision files; no UI frameworks or packages are
installed. Optional --csharp checks the Windows decision with an existing SDK.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
# phase, input, composing, send, queue, resume, expected action, enabled
CASES = [
    ('idle', False, False, True, False, False, 'send', False),
    ('idle', False, False, True, False, True, 'resume', True),
    ('idle', True, False, True, False, True, 'send', True),
    ('idle', True, True, True, False, True, 'send', False),
    ('idle', False, True, True, False, True, 'resume', False),
    ('idle', True, False, False, False, True, 'send', False),
    ('running', False, False, False, True, True, 'none', False),
    ('running', True, False, False, True, True, 'queue', True),
    ('running', True, True, False, True, True, 'queue', False),
    ('running', True, False, False, False, True, 'queue', False),
    ('stopping', False, False, True, True, True, 'stopping', False),
    ('stopping', True, False, True, True, True, 'stopping', False),
    ('sending', True, False, True, True, True, 'sending', False),
    ('resuming', False, False, True, True, True, 'resuming', False),
    ('resuming', True, False, True, True, True, 'resuming', False),
    ('blocked', False, False, True, True, True, 'send', False),
    ('blocked', True, False, True, True, True, 'send', False),
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--csharp', action='store_true')
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix='velune-composer-actions-') as directory:
        root = Path(directory)
        (root/'cases.json').write_text(json.dumps(CASES))
        swift = '''import Foundation
let phases: [String: ComposerSubmission.Phase] = ["idle": .idle, "running": .running, "stopping": .stopping, "sending": .sending, "resuming": .resuming, "blocked": .blocked]
let rows = try JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1]))) as! [[Any]]
for (index, row) in rows.enumerated() {
    let value = ComposerSubmission(phase: phases[row[0] as! String]!, hasInput: row[1] as! Bool, isComposing: row[2] as! Bool, canSend: row[3] as! Bool, canQueue: row[4] as! Bool, canResume: row[5] as! Bool)
    precondition(String(describing: value.action) == row[6] as! String && value.enabled == row[7] as! Bool, "case \\(index)")
    precondition(!(value.canSubmitDraft && value.canResume))
}
print("PASS Mac: \\(rows.count) composer cases; send/queue/resume are exclusive, in-flight and IME block submission")
'''
        (root/'main.swift').write_text(swift)
        subprocess.run(['xcrun','swiftc',str(ROOT/'app/mac/ComposerSubmission.swift'),str(root/'main.swift'),'-o',str(root/'check')],check=True)
        subprocess.run([str(root/'check'),str(root/'cases.json')],check=True)
        if args.csharp:
            sdks = subprocess.check_output(['dotnet','--list-sdks'],text=True).splitlines()
            major = int(sdks[-1].split('.')[0])
            (root/'check.csproj').write_text(f'<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net{major}.0</TargetFramework><ImplicitUsings>enable</ImplicitUsings></PropertyGroup><ItemGroup><Compile Include="{ROOT}/app/windows/ComposerSubmission.cs" /></ItemGroup></Project>')
            (root/'NuGet.Config').write_text('<configuration><packageSources><clear /></packageSources></configuration>')
            (root/'Program.cs').write_text('''using System.Text.Json;
using Velune;
var rows = JsonDocument.Parse(File.ReadAllText(args[0])).RootElement;
foreach (var row in rows.EnumerateArray()) {
    var phase = Enum.Parse<ComposerSubmission.Phase>(row[0].GetString()!, true);
    var value = ComposerSubmission.Resolve(phase,row[1].GetBoolean(),row[2].GetBoolean(),row[3].GetBoolean(),row[4].GetBoolean(),row[5].GetBoolean());
    if (value.Action.ToString().ToLowerInvariant() != row[6].GetString() || value.Enabled != row[7].GetBoolean() || (value.CanSubmitDraft && value.CanResume)) throw new Exception(row.ToString());
}
Console.WriteLine($"PASS Windows: {rows.GetArrayLength()} composer cases; same action contract as Mac");
''')
            subprocess.run(['dotnet','run','--project',str(root/'check.csproj'),'--',str(root/'cases.json')],check=True,cwd=root)


if __name__ == '__main__':
    main()
