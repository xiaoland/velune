#!/usr/bin/env python3
"""Manual acceptance of a native sub-agent that outlives its parent's gateway
turn on an SSH runtime, through UniFFI.

Velune runs Codex 0.159.3 on the SSH host in a managed HOME below
--remote-root, against the loopback upstream of manual-subagent-activity.py
reached through Velune's gateway and its SSH reverse tunnel. The parent starts
a sub-agent and ends its turn; the upstream holds the sub-agent until released.
Checks that the sub-agent keeps running over the tunnel after the turn, that a
next turn on another gateway model waits in the send queue while one on the
same model reuses the process, that the sub-agent's approval reaches the idle
parent, and that once it has ended the remote app-server ends, the waiting
turn is sent on a new process and tunnel, and the sub-agent's thread is kept
on the Harness's own provider again. --remote-root must be an existing private
directory on the host; only synthetic responses are used, no credential,
remote HOME or session is read; this is not an automated test.
"""
import argparse
import importlib.util
import pathlib
import subprocess
import sys
import tempfile
import threading
import time
from http.server import ThreadingHTTPServer

HERE = pathlib.Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('subagent_activity', HERE / 'manual-subagent-activity.py')
activity = importlib.util.module_from_spec(spec)
spec.loader.exec_module(activity)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('library', 'bindings', 'bundle'):
        parser.add_argument('--' + name, type=pathlib.Path, required=True)
    for name in ('ssh-destination', 'remote-root', 'remote-codex'):
        parser.add_argument('--' + name, required=True)
    args = parser.parse_args()

    def remote(command):
        return subprocess.run(['ssh', '-o', 'BatchMode=yes', args.ssh_destination, command],
                              capture_output=True, text=True, check=True).stdout

    def remote_app_servers():
        # The SSH supervisor's command line names the binary too; count Codex itself.
        return remote("pgrep -u \"$(id -u)\" -x codex || true").split()

    server = ThreadingHTTPServer(('127.0.0.1', 0), activity.Upstream)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    endpoint = 'http://127.0.0.1:%d/v1' % server.server_port
    spec = importlib.util.spec_from_file_location('velune_subagent_ssh', args.bindings / 'velune_bindings.py')
    b = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = b
    spec.loader.exec_module(b)
    project = args.remote_root + '/project'
    remote('mkdir -p %s' % project)

    with tempfile.TemporaryDirectory(prefix='velune-subagent-ssh-') as directory:
        app = b.VeluneApplication.open(b.BindingOptions(home_directory=str(pathlib.Path(directory) / 'velune'),
                                                         resources_directory=str(args.bundle / 'Contents/Resources')))
        try:
            app.upsert_runtime_with_execution(b.BindingRuntimeInstance(
                enabled=True, id='codex', name='Codex', type_id='codex-0.159', gateway_id='default',
                settings={'binary': args.remote_codex, 'nodeBinary': '/usr/bin/node'}),
                b.BindingRuntimeExecution(
                    transport=b.BindingRuntimeTransport.SSH(binary='/usr/bin/ssh', destination=args.ssh_destination, port=None),
                    home=b.BindingRuntimeHome.MANAGED(remote_root=args.remote_root + '/homes'),
                    remote_resources_directory=args.remote_root + '/resources'))
            models = [b.BindingProviderModel(record_key='', provider_model_id=name, nickname=name, icon=None,
                                             context_window=32768, max_output_tokens=1024, reasoning_levels=None)
                      for name in ('synthetic-gateway', 'synthetic-gateway-2')]
            app.save_provider('default', b.BindingProviderDraft(
                id='responses', name='responses', protocol=b.BindingGatewayProtocol.RESPONSES_V1, endpoint=endpoint,
                models=models, execution_backend=b.BindingExecutionBackend.HTTP(), request_profile=b.BindingRequestProfile.NATIVE, authentication_source=b.BindingProviderAuthenticationSource.API_KEY, quota_source=b.BindingProviderQuotaSource.NONE), b.BindingAuthenticationEdit.SET_API_KEY(value='SYNTHETIC_ONLY'))
            keys = [model.record_key for model in next(p for p in app.list().gateways[0].providers if p.id == 'responses').models]
            first, second = (b.BindingModelSelection.MANAGED(model_record_key=key) for key in keys)

            def snapshot(conversation):
                return app.snapshot('codex', conversation).snapshot

            def replies(conversation):
                return [block.text for message in snapshot(conversation).messages for block in message.blocks
                        if isinstance(block, b.BindingMessageBlock.TEXT)].count('PARENT_DONE')

            def wait(condition, what, seconds=90):
                deadline = time.monotonic() + seconds
                while True:
                    found = condition()
                    if found:
                        return found
                    assert time.monotonic() < deadline, 'timed out: ' + what
                    time.sleep(.3)

            def running(parent):
                return [item.conversation_id for item in app.conversation_activity(0).active
                        if item.parent_conversation_id == parent and item.run_state == b.BindingRunState.RUNNING]

            def turn(conversation, selection, text):
                return app.send_turn_with_selection('codex', conversation, 'codex', selection, b.BindingComposedTurn(
                    text=text, effort=None, attachments=[]))

            def idle_after(conversation, count, what):
                wait(lambda: snapshot(conversation).run_state == b.BindingRunState.IDLE and replies(conversation) == count, what)

            activity.RELEASE.clear()
            conversation = app.create_conversation_with_selection('codex', project, first).snapshot.conversation.id
            conversation = turn(conversation, first, 'PARENT_TASK').snapshot.conversation.id
            idle_after(conversation, 1, 'remote parent turn ends')
            child = snapshot(conversation).sub_agents[0].conversation_id
            wait(lambda: child in running(conversation), 'remote sub-agent running after the turn')
            servers = remote_app_servers()
            assert len(servers) == 1, servers
            print('PASS ssh: the remote sub-agent keeps running after the parent\'s gateway turn ended', flush=True)
            turn(conversation, second, 'OTHER_MODEL_NEXT')
            follow = snapshot(conversation).follow_up
            assert follow.waiting == b.BindingQueueWait.SUB_AGENTS and len(follow.queue) == 1, follow
            turn(conversation, first, 'SAME_MODEL_NEXT')
            idle_after(conversation, 2, 'same-model turn over the tunnel')
            assert child in running(conversation) and remote_app_servers() == servers
            print('PASS ssh: another model waits in the queue; the same model reused the remote process and tunnel', flush=True)
            activity.RELEASE.set()
            pending = wait(lambda: snapshot(conversation).pending_interactions, 'remote sub-agent approval')
            app.reply_runtime_interaction('codex', conversation, pending[0].id, b.BindingRuntimeInteractionReply.DECISION(
                option_id=pending[0].kind.options[0].id))
            wait(lambda: child not in running(conversation), 'remote sub-agent ends', 120)
            idle_after(conversation, 3, 'queued turn on the other model')
            assert not snapshot(conversation).follow_up.queue
            wait(lambda: not remote_app_servers(), 'remote app-servers end')
            provider = remote("python3 -c \"import glob,sqlite3,sys; db=sqlite3.connect(glob.glob('%s/homes/*/.codex/state_*.sqlite')[0]); "
                              "print(db.execute('select model_provider from threads where id=?',(sys.argv[1],)).fetchone()[0])\" %s"
                              % (args.remote_root, child.split(':', 1)[1])).strip()
            assert provider != 'velune', provider
            print('PASS ssh: after the sub-agent ended the remote app-server ended, the waiting turn ran on a new '
                  'process and tunnel, and the sub-agent\'s thread is on the Harness\'s own provider (%s)' % provider, flush=True)
        finally:
            activity.RELEASE.set()
            app.shutdown_stopping_work()
            server.shutdown()


if __name__ == '__main__':
    main()
