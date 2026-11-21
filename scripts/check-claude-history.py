#!/usr/bin/env python3
"""Synthetic checks of transcript continuity versus compacted model context."""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
RESOURCES = ROOT / 'packages/agent-runtime/resources'
SESSION = '00000000-0000-4000-8000-000000000001'


def message(id, parent, text, role='user', **extra):
    return dict(uuid=id, parentUuid=parent, type=role, message={'role': role, 'content': text}, **extra)


class ClaudeHistory(unittest.TestCase):
    def read(self, records, operation='read'):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / (SESSION + '.jsonl')
            path.write_text(''.join(json.dumps(r) + '\n' for r in records))
            source = (RESOURCES / 'claude_context.py').read_text() + '\n' + (RESOURCES / 'claude_history.py').read_text()
            result = subprocess.run(['python3', '-c', source], input=json.dumps(dict(
                roots=[directory], runtimeId='synthetic', nativeId=SESSION, operation=operation)),
                text=True, capture_output=True, check=True)
            return json.loads(result.stdout)

    def test_api_failure_preserves_login_reason_after_task_notification(self):
        records = [message('u', None, 'question'),
                   dict(uuid='a', parentUuid='u', type='assistant', message=dict(content='answer', stop_reason='end_turn')),
                   message('notification', 'a', '<task-notification><task-id>job</task-id><status>completed</status><summary>done</summary></task-notification>',
                           origin=dict(kind='task-notification'), promptSource='system'),
                   dict(uuid='error', parentUuid='notification', type='assistant', isApiErrorMessage=True,
                        error='authentication_failed', message=dict(content=[dict(type='text', text='Login expired · Please run /login')], stop_reason='stop_sequence')),
                   dict(uuid='duration', parentUuid='error', type='system', subtype='turn_duration', durationMs=2356)]
        history = self.read(records)['history']
        self.assertEqual(history['messages'][-1]['role'], 'system')
        self.assertEqual(history['messages'][-1]['blocks'], [
            dict(kind='runtimeFailure', failure='other', detail='Login expired · Please run /login\nClaude Code 错误类型：authentication_failed', retry=None)])
        execution = history['executions']['records'][-1]
        self.assertEqual(execution['outcome'], 'failed')
        self.assertIn('error', execution['message_ids'])
        records[-2]['message']['content'] = []
        records[-2].pop('error')
        self.assertEqual(self.read(records)['history']['messages'][-1]['blocks'], [
            dict(kind='runtimeFailure', failure='other', detail='Claude Code 执行失败（上游未提供错误详情）。', retry=None)])

    def test_api_failure_recovery_belongs_to_its_execution(self):
        records = [message('u', None, 'question'),
                   dict(uuid='error', parentUuid='u', type='assistant', isApiErrorMessage=True,
                        error='authentication_failed', message=dict(content=[], stop_reason='stop_sequence'))]
        recovered = records + [dict(uuid='answer', parentUuid='error', type='assistant',
                                    message=dict(content='recovered', stop_reason='end_turn'))]
        history = self.read(recovered)['history']
        self.assertEqual(history['messages'][1]['blocks'][0]['retry'], dict(kind='recovered'))
        self.assertEqual(history['executions']['records'][-1]['outcome'], 'completed')
        new_turn = records + [message('u2', 'error', 'next question'),
                              dict(uuid='answer', parentUuid='u2', type='assistant',
                                   message=dict(content='new answer', stop_reason='end_turn'))]
        history = self.read(new_turn)['history']
        self.assertIsNone(history['messages'][1]['blocks'][0]['retry'])
        self.assertEqual([e['outcome'] for e in history['executions']['records']], ['failed', 'completed'])

    def test_compaction_keeps_transcript_but_exports_only_effective_context(self):
        records = [message('u1', None, 'original question'), message('a1', 'u1', 'original answer', 'assistant'),
                   dict(uuid='b1', parentUuid=None, logicalParentUuid='a1', type='system', subtype='compact_boundary'),
                   message('s1', 'b1', 'summary one', isCompactSummary=True, isVisibleInTranscriptOnly=True),
                   message('u2', 's1', 'next question'), message('a2', 'u2', 'next answer', 'assistant'),
                   dict(uuid='b2', parentUuid=None, logicalParentUuid='a2', type='system', subtype='compact_boundary'),
                   message('s2', 'b2', 'summary two', isCompactSummary=True), message('u3', 's2', 'last question')]
        history = self.read(records)['history']
        self.assertEqual([m['id'] for m in history['messages']], ['u1', 'a1', 'b1', 'u2', 'a2', 'b2', 'u3'])
        self.assertEqual(history['title']['text'], 'original question')
        self.assertEqual([e['message_ids'][0] for e in history['executions']['records']], ['u1', 'u2', 'u3'])
        context = self.read(records, 'export_context')['messages']
        self.assertEqual([m['content'][0]['text'] for m in context], ['summary two', 'last question'])

    def test_compact_summary_is_detail_of_boundary_never_a_user_turn(self):
        records = [message('u', None, 'question'),
                   dict(uuid='b', parentUuid=None, logicalParentUuid='u', type='system', subtype='compact_boundary'),
                   message('s', 'b', 'native summary', isCompactSummary=True, isVisibleInTranscriptOnly=True)]
        history = self.read(records)['history']
        self.assertEqual([m['id'] for m in history['messages']], ['u', 'b'])
        self.assertEqual(history['messages'][-1]['blocks'], [dict(kind='contextCompaction', text='上下文已压缩\n\nnative summary')])
        self.assertEqual(len(history['executions']['records']), 1)

    def test_safety_meta_is_visible_and_not_a_new_turn_or_cancelled_success(self):
        records = [message('u', None, 'question', promptId='prompt', promptSource='human'),
                   message('a', 'u', 'partial', 'assistant'),
                   message('s', 'a', 'synthetic safety stop reason', isMeta=True, refusedMessageId='msg_refused', promptId='prompt'),
                   message('i', 's', [dict(type='text', text='[Request interrupted by user]')], promptId='prompt')]
        history = self.read(records)['history']
        self.assertEqual([m['id'] for m in history['messages']], ['u', 'a', 's'])
        self.assertEqual(history['messages'][-1]['role'], 'system')
        self.assertIn('synthetic safety stop reason', str(history['messages'][-1]['blocks']))
        self.assertEqual(history['executions']['records'][0]['outcome'], 'failed')
        self.assertEqual(len(history['executions']['records']), 1)
        # A later genuine safe answer can complete the native recovery.
        records.append(dict(uuid='a2', parentUuid='i', type='assistant', message=dict(content='safe answer', stop_reason='end_turn')))
        self.assertEqual(self.read(records)['history']['executions']['records'][0]['outcome'], 'completed')

    def test_branch_excludes_abandoned_messages(self):
        records = [message('u1', None, 'question'), message('a1', 'u1', 'answer', 'assistant'),
                   message('abandoned', 'a1', 'old branch'), message('replacement', 'a1', 'new branch'),
                   dict(uuid='b', parentUuid=None, logicalParentUuid='replacement', type='system', subtype='compact_boundary'),
                   message('s', 'b', 'summary', isCompactSummary=True)]
        self.assertEqual([m['id'] for m in self.read(records)['history']['messages']], ['u1', 'a1', 'replacement', 'b'])

    def test_retained_order_repairs_replayed_parent_links(self):
        records = [message('u', None, 'question'), message('a', 'u', 'answer', 'assistant'),
                   message('u2', 'a', 'next question'), message('a2', 'u2', 'next answer', 'assistant'),
                   dict(uuid='hook', parentUuid='a', type='system', subtype='stop_hook_summary'),
                   dict(uuid='b', parentUuid=None, logicalParentUuid='hook', type='system', subtype='compact_boundary',
                        compactMetadata=dict(preservedMessages=dict(allUuids=['u2', 'a2', 'hook']))),
                   message('s', 'b', 'summary', isCompactSummary=True)]
        self.assertEqual([m['id'] for m in self.read(records)['history']['messages']], ['u', 'a', 'u2', 'a2', 'b'])

    def test_compaction_updates_context_without_billing_summary_as_a_turn(self):
        records = [message('u', None, 'question'),
                   dict(uuid='a', parentUuid='u', type='assistant', message=dict(id='reply', model='test', content='answer', usage=dict(input_tokens=100, output_tokens=20))),
                   dict(uuid='b', parentUuid=None, logicalParentUuid='a', type='system', subtype='compact_boundary', compactMetadata=dict(postTokens=15)),
                   message('s', 'b', 'summary', isCompactSummary=True)]
        usage = self.read(records)['history']['messages'][0]['turn_usage']
        self.assertEqual(usage['contextTokens'], 15)
        self.assertEqual(usage['inputTokens'], 100)
        self.assertEqual(usage['outputTokens'], 20)

    def test_missing_pre_compact_history_keeps_available_messages(self):
        records = [dict(uuid='b', parentUuid=None, logicalParentUuid='missing', type='system', subtype='compact_boundary'),
                   message('s', 'b', 'summary', isCompactSummary=True), message('u', 's', 'question')]
        self.assertEqual([m['id'] for m in self.read(records)['history']['messages']], ['b', 'u'])

    def test_two_auto_compactions_keep_execution_order_and_manual_is_separate(self):
        records = [message('u', None, 'question'), message('a1', 'u', 'working', 'assistant')]
        for index, parent in [(1, 'a1'), (2, 'a2')]:
            records.extend([
                dict(uuid=f'b{index}', parentUuid=None, logicalParentUuid=parent,
                     type='system', subtype='compact_boundary', compactMetadata=dict(trigger='auto')),
                message(f's{index}', f'b{index}', f'summary {index}', isCompactSummary=True),
                message(f'a{index + 1}', f's{index}', 'more work' if index == 1 else 'final answer', 'assistant')])
        records.extend([
            dict(uuid='manual', parentUuid=None, logicalParentUuid='a3', type='system',
                 subtype='compact_boundary', compactMetadata=dict(trigger='manual')),
            message('sm', 'manual', 'manual summary', isCompactSummary=True),
            message('u2', 'sm', 'next turn'), message('a4', 'u2', 'next answer', 'assistant')])
        history = self.read(records)['history']
        self.assertEqual([m['id'] for m in history['messages']],
                         ['u', 'a1', 'b1', 'a2', 'b2', 'a3', 'manual', 'u2', 'a4'])
        executions = history['executions']['records']
        self.assertEqual(executions[0]['message_ids'], ['u', 'a1', 'b1', 'a2', 'b2', 'a3'])
        self.assertEqual(executions[1]['kind'], 'ContextMaintenance')
        self.assertEqual(executions[1]['message_ids'], ['manual'])
        self.assertEqual(executions[2]['message_ids'], ['u2', 'a4'])

    def test_parallel_result_siblings_complete_their_original_rows(self):
        records = [message('u', None, 'question')]
        for index in (1, 2):
            records.append(dict(uuid=f'a{index}', parentUuid='u' if index == 1 else 'a1',
                                type='assistant', message=dict(id='api-batch', content=[
                                    dict(type='tool_use', id=f't{index}', name='Read', input={})])))
        for index in (1, 2):
            records.append(message(f'r{index}', f'a{index}', [
                dict(type='tool_result', tool_use_id=f't{index}', content=f'result {index}')]))
        records.append(message('answer', 'r2', 'answer', 'assistant'))
        history = self.read(records)['history']
        self.assertEqual([m['id'] for m in history['messages']], ['u', 'a1', 'a2', 'answer'])
        tools = [b for m in history['messages'] for b in m['blocks'] if b['kind'] == 'tool']
        self.assertEqual([(b['tool_id'], b['state'], b['output']) for b in tools],
                         [('t1', 'completed', 'result 1'), ('t2', 'completed', 'result 2')])


if __name__ == '__main__':
    unittest.main()
