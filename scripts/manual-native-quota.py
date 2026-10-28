#!/usr/bin/env python3
"""Isolated native login quota and headless Mac composer state acceptance; synthetic data only, never CI."""
import argparse
import importlib.util
import json
import os
import platform
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time

from mac_telemetry_build import add_app_sources, add_telemetry

FAKE = r'''#!__PYTHON__
import json, os, sys, time
from pathlib import Path
directory = Path(os.environ.get('CLAUDE_CONFIG_DIR') or Path(os.environ['HOME']) / '.claude')
with open(__LOG__, 'a') as out:
    out.write(json.dumps({'args':sys.argv[1:], 'home':os.environ['HOME'], 'directory':str(directory), 'explicit':os.environ.get('CLAUDE_CONFIG_DIR')})+'\n')
if sys.argv[1:] == ['--version']:
    print('2.1.286 (Claude Code)'); sys.exit(0)
fixture = json.loads((directory/'fixture.json').read_text())
if sys.argv[1:3] == ['auth', 'status']:
    print(json.dumps({'loggedIn':True,'authMethod':'claude.ai','accountUuid':fixture['account'],'configDirectory':str(directory)})); sys.exit(0)
assert sys.argv[1:3] == ['-p', '/usage'], 'Never retry usage as a model prompt'
(directory/'started').write_text('started')
while (directory/'hold').exists(): time.sleep(.02)
print(json.dumps({'type':'result','is_error':False,'result':f"Current session: {fixture['used'][0]}% used · Resets later\nCurrent week (all models): {fixture['used'][1]}% used"}), flush=True)
# EOF precedes process exit; a valid sample must survive normal exit cleanup.
os.close(1); time.sleep(.1)
os._exit(3 if fixture.get('fail') else 0)
'''


def wait_for(condition, message):
    deadline = time.monotonic() + 20
    while not condition() and time.monotonic() < deadline:
        time.sleep(.04)
    assert condition(), message


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--bindings', type=Path, required=True, help='matching Python bindings directory')
    parser.add_argument('--swift-build', type=Path, required=True, help='SwiftPM dependency build directory')
    parser.add_argument('--sources', type=Path, help='optional frozen app/mac source directory')
    args = parser.parse_args()
    repository = Path(__file__).resolve().parent.parent
    library = args.bundle/'Contents/Frameworks/libvelune_bindings.dylib'
    for path in (library, args.bindings/'velune_bindings.py', args.swift_build/'description.json'):
        if not path.is_absolute() or not path.exists(): parser.error('absolute existing matching build paths required')
    with tempfile.TemporaryDirectory(prefix='velune-native-quota-') as directory:
        root = Path(directory)
        home = root/'home'; home.mkdir()
        for name, used in [('home/.claude', [54,25]), ('B', [22,65]), ('C', [8,12]), ('failed', [0,0])]:
            target = root/name; target.mkdir(parents=True, exist_ok=True)
            (target/'fixture.json').write_text(json.dumps({'account':name,'used':used,'fail':name=='failed'}))
        (root/'resources').mkdir()
        log = root/'cli.jsonl'
        binary = root/'claude'
        binary.write_text(FAKE.replace('__PYTHON__',sys.executable).replace('__LOG__',repr(str(log))))
        binary.chmod(0o755)
        env = {'HOME':str(home),'VELUNE_HOME':str(root/'unused-home'),'PATH':'/usr/bin:/bin'}
        os.environ.clear(); os.environ.update(env)
        module = root/'python'; module.mkdir()
        shutil.copy(args.bindings/'velune_bindings.py',module)
        (module/library.name).symlink_to(library)
        spec = importlib.util.spec_from_file_location('velune_bindings',module/'velune_bindings.py')
        b = importlib.util.module_from_spec(spec); sys.modules[spec.name]=b; spec.loader.exec_module(b)

        def open_app(name):
            return b.VeluneApplication.open(b.BindingOptions(home_directory=str(root/name),resources_directory=str(root/'resources')))

        def runtime(app, name, target):
            value = b.BindingRuntimeInstance(enabled=True,id=name,name=name,type_id='claude-code-2.1',gateway_id='default',settings={'binary':str(binary),'agentDir':str(target)})
            app.upsert_runtime(value)
            return value

        def fresh(app, name):
            result = app.runtime_quota(name,False)
            return result if result.freshness == b.BindingQuotaFreshness.FRESH else None

        def calls():
            return [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []

        app = open_app('core-boundaries')
        try:
            a = runtime(app,'A',home/'.claude'); runtime(app,'B',root/'B')
            wait_for(lambda:fresh(app,'A'), 'Native default never refreshed')
            assert [w.used_percent for w in fresh(app,'A').windows] == [54,25]
            assert any(c['args'][:2]==['-p','/usage'] and c['explicit'] is None for c in calls()), 'Default login must not set CLAUDE_CONFIG_DIR'
            (home/'.claude/started').unlink(); (home/'.claude/hold').touch()
            app.runtime_quota('A',True)
            wait_for(lambda:(home/'.claude/started').exists(),'Old request did not start')
            runtime(app,'A',root/'C')
            wait_for(lambda:fresh(app,'A'),'Changed source never refreshed')
            (home/'.claude/hold').unlink()
            assert [w.used_percent for w in fresh(app,'A').windows] == [8,12], 'Old account overwrote the changed source'
            (root/'C/started').unlink(); (root/'C/hold').touch()
            app.runtime_quota('A',True)
            wait_for(lambda:(root/'C/started').exists(),'Deletion fixture did not start')
            app.delete_runtime('A'); (root/'C/hold').unlink()
            try: app.runtime_quota('A',False)
            except b.BindingError: pass
            else: raise AssertionError('Deleted runtime retained quota')
            runtime(app,'failure',root/'failed')
            wait_for(lambda:app.runtime_quota('failure',False).problem is not None,'Failure stayed invisible')
            assert not app.runtime_quota('failure',False).windows
            a = runtime(app,'unsupported',home/'.claude')
            for execution in [
                b.BindingRuntimeExecution(transport=b.BindingRuntimeTransport.LOCAL(),home=b.BindingRuntimeHome.EXISTING(path=str(home)),remote_resources_directory=None),
                b.BindingRuntimeExecution(transport=b.BindingRuntimeTransport.SSH(binary='/usr/bin/false',destination='fixture.invalid',port=None),home=b.BindingRuntimeHome.INHERIT(),remote_resources_directory='/tmp/fixture-resources')]:
                before = sum(c['args'][:2]==['-p','/usage'] for c in calls())
                app.upsert_runtime_with_execution(a,execution)
                result=app.runtime_quota('unsupported',True)
                assert result.freshness == b.BindingQuotaFreshness.UNSUPPORTED and result.problem and not result.windows
                assert sum(c['args'][:2]==['-p','/usage'] for c in calls()) == before, 'Unsupported execution read the local login'
            print('PASS native source/default directory/change/cancellation/deletion/failure/HOME/SSH')
        finally: app.shutdown()

        for profile in ['without-provider','with-provider']:
            app=open_app(profile)
            try:
                runtime(app,'A',home/'.claude'); runtime(app,'B',root/'B')
                if profile=='with-provider':
                    draft=b.BindingProviderDraft(id='',name='提供商 B',protocol=b.BindingGatewayProtocol.MESSAGES_V1,endpoint='',execution_backend=b.BindingExecutionBackend.CLAUDE_CLI(binary=str(binary)),request_profile=b.BindingRequestProfile.NATIVE,authentication_source=b.BindingProviderAuthenticationSource.CLAUDE_CLI,quota_source=b.BindingProviderQuotaSource.CLAUDE_CLI_USAGE,models=[b.BindingProviderModel(record_key='',provider_model_id='sonnet',nickname='',icon=None,context_window=None,max_output_tokens=None,reasoning_levels=None)],quota_limits=[])
                    app.save_provider('default',draft,b.BindingAuthenticationEdit.USE_CLAUDE_CODE_LOGIN(login=b.BindingLinkedClaudeLogin.RUNTIME(instance_id='B')))
            finally: app.shutdown()

        # A fresh binding object avoids linking an old cached ABI into the background check.
        modules=root/'Modules'; modules.mkdir()
        binding_object=root/'VeluneBindings.o'
        subprocess.run(['xcrun','swiftc','-target',platform.machine()+'-apple-macosx15.0','-parse-as-library','-whole-module-optimization','-module-name','VeluneBindings','-package-name','velune','-I',str(repository/'target/swift-ffi'),'-emit-module','-emit-object','-emit-module-path',str(modules/'VeluneBindings.swiftmodule'),'-o',str(binding_object),str(repository/'target/swift-bindings/VeluneBindings.swift')],check=True,cwd=repository)
        # Freeze sources so another task editing the shared checkout cannot interrupt compilation.
        source_root=root/'source'
        shutil.copytree(args.sources or repository/'app/mac', source_root/'app/mac')
        store=root/'Store.swift'
        store.write_text((source_root/'app/mac/Store.swift').read_text()+'''
extension AppStore {
    @MainActor func quotaAcceptanceLoad(_ configuration: BindingConfigurationSnapshot) { applyList(configuration) }
    @MainActor func quotaAcceptanceSelection(runtimeID: String, model: ModelSelection?) {
        hasInitializedNextTurnIntent = true
        nextTurnRuntimeID = runtimeID; nextTurnModel = model
        statusLine = [.rateLimits]; usageDisplay = .remaining
    }
}
''')
        executable=root/'native-quota-check'
        command=['xcrun','swiftc','-target',platform.machine()+'-apple-macosx15.0','-parse-as-library','-whole-module-optimization','-swift-version','5','-package-name','velune','-warnings-as-errors','-I',str(modules),'-I',str(args.swift_build/'Modules'),'-I',str(repository/'target/swift-ffi'),str(repository/'tasks/account-quota-display/acceptance/native.swift')]
        add_app_sources(command,source_root,root,store)
        add_telemetry(command,args.swift_build,source_root)
        command=[value for value in command if '/VeluneBindings.build/' not in value]
        command=[('-fmodule-map-file='+str(repository/'target/swift-ffi/module.modulemap')) if value.startswith('-fmodule-map-file=') and value.endswith('/swift-ffi/module.modulemap') else value for value in command]
        command.extend([str(binding_object),'-L',str(library.parent),'-lvelune_bindings','-Xlinker','-rpath','-Xlinker',str(library.parent),'-o',str(executable)])
        subprocess.run(command,check=True,cwd=repository)
        subprocess.run([str(executable),str(root),str(args.bundle/'Contents/Resources')],env=env,check=True,timeout=90)


if __name__=='__main__':
    try: main()
    except subprocess.CalledProcessError as error:
        sys.exit(f'Check command failed with exit status {error.returncode}; see diagnostics above.')
