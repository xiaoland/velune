#!/usr/bin/env python3
"""Check Codex fork/sample behavior against a localhost synthetic provider.

This checks the installed CLI protocol, rather than Velune's handoff integration.
All history, credentials and model responses are synthetic. It never opens a GUI.
"""
import argparse, hashlib, json, os, queue, shutil, subprocess, tempfile, threading, time, traceback
from pathlib import Path
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

parser=argparse.ArgumentParser(description='Isolated Codex persistent fork + sample regression; localhost synthetic provider only.')
parser.add_argument('--codex',default='/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex')
parser.add_argument('--output-dir',type=Path,default=Path('target/checks/codex-sampling-route'))
parser.add_argument('--temporary-parent',type=Path,default=Path('/tmp'))
parser.add_argument('--summary-model',default='gpt-6-sol')
parser.add_argument('--keep-fixture',action='store_true',help='retain the private synthetic directory for diagnosis')
args=parser.parse_args()
OUT = args.output_dir.resolve(); OUT.mkdir(parents=True,exist_ok=True)
ROOT = Path(tempfile.mkdtemp(prefix='velune-summary-route-', dir=args.temporary_parent))
CLI = str(Path(args.codex).resolve(strict=True))
for directory in ('home', 'codex', 'workspace'):
    (ROOT / directory).mkdir()
REQUESTS, RPC, FINDINGS = [], [], {}
MODE = 'text'
MARKER = ROOT / 'workspace/TOOL_EXECUTED'
OPAQUE = 'SYNTHETIC_ENCRYPTED_COMPACTION_ONLY'

class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
        REQUESTS.append({'path': self.path, 'mode': MODE, 'body': body})
        number = len(REQUESTS)
        answer = 'Synthetic portable summary with retained decisions.'
        item = {'type':'message','id':f'msg_probe_{number}','status':'completed','role':'assistant','phase':'final_answer','content':[{'type':'output_text','text':answer,'annotations':[]}]}
        if MODE == 'tool':
            item = {'type':'function_call','id':f'fc_probe_{number}','call_id':f'call_probe_{number}','name':'exec_command','arguments':json.dumps({'cmd':f'printf executed > {MARKER}'})}
        output = [] if MODE == 'empty' else [item]
        response = {'id':f'resp_probe_{number}','object':'response','status':'completed','model':body['model'],'output':output,'usage':{'input_tokens':20,'output_tokens':5,'total_tokens':25}}
        events = [{'type':'response.created','response':{**response,'status':'in_progress','output':[]}}]
        if output:
            events += [{'type':'response.output_item.added','output_index':0,'item':item},{'type':'response.output_item.done','output_index':0,'item':item}]
        events += [{'type':'response.completed','response':response}]
        payload = ''.join('event: '+e['type']+'\ndata: '+json.dumps(e)+'\n\n' for e in events).encode()
        self.send_response(200); self.send_header('Content-Type','text/event-stream'); self.send_header('Content-Length',str(len(payload))); self.end_headers(); self.wfile.write(payload)

server = ThreadingHTTPServer(('127.0.0.1',0),Handler)
threading.Thread(target=server.serve_forever,daemon=True).start()
(ROOT/'codex/config.toml').write_text(f'''model = "gpt-6.1-sol"
model_provider = "synthetic"
approval_policy = "never"
sandbox_mode = "read-only"
web_search = "disabled"
model_post_turn_compact_threshold_percent = 0
[model_providers.synthetic]
name = "Synthetic"
base_url = "http://127.0.0.1:{server.server_port}/v1"
wire_api = "responses"
experimental_bearer_token = "synthetic-fake-token"
[features]
responses_websockets = false
responses_websockets_v2 = false
hooks = false
goals = false
plugins = false
apps = false
memories = false
shell_snapshot = false
''')
ENV = {'HOME':str(ROOT/'home'),'CODEX_HOME':str(ROOT/'codex'),'PATH':'/usr/bin:/bin:/usr/sbin:/sbin','HTTP_PROXY':'http://127.0.0.1:1','HTTPS_PROXY':'http://127.0.0.1:1','ALL_PROXY':'http://127.0.0.1:1','NO_PROXY':'127.0.0.1,localhost','DISABLE_TELEMETRY':'1'}

class Client:
    def __init__(self, name):
        self.log = (OUT/f'{name}.stderr.log').open('w')
        self.p = subprocess.Popen([CLI,'app-server','--listen','stdio://'],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=self.log,env=ENV,text=True,cwd=ROOT/'workspace',bufsize=1)
        self.q = queue.Queue(); self.events = []; self.next_id = 0
        threading.Thread(target=self.reader,daemon=True).start()
        try:
            self.call('initialize',{'clientInfo':{'name':'velune_summary_probe','version':'1'},'capabilities':{'experimentalApi':True}})
            self.send({'method':'initialized','params':{}})
        except BaseException:
            self.close()
            raise
    def reader(self):
        for line in self.p.stdout:
            try: self.q.put(json.loads(line))
            except ValueError: pass
    def send(self, data):
        RPC.append({'direction':'request','data':data})
        self.p.stdin.write(json.dumps(data)+'\n'); self.p.stdin.flush()
    def receive(self, timeout=30):
        record = self.q.get(timeout=timeout); RPC.append({'direction':'response','data':record})
        if record.get('method') and 'id' in record:
            self.send({'id':record['id'],'error':{'code':-32601,'message':'Synthetic probe forbids client tools and approval'}})
            raise AssertionError('unexpected client request: '+record['method'])
        return record
    def call(self, method, params, timeout=30):
        self.next_id += 1; identity = self.next_id
        self.send({'id':identity,'method':method,'params':params})
        deadline=time.monotonic()+timeout
        while time.monotonic()<deadline:
            record=self.receive(max(.1,deadline-time.monotonic()))
            if record.get('id')==identity:
                if 'error' in record: raise RuntimeError(method+': '+json.dumps(record['error']))
                return record.get('result')
            self.events.append(record)
        raise TimeoutError(method)
    def completed(self, turn):
        deadline=time.monotonic()+30
        while time.monotonic()<deadline:
            for i,e in enumerate(self.events):
                if e.get('method')=='turn/completed' and e['params']['turn']['id']==turn:
                    return self.events.pop(i)
            self.events.append(self.receive(max(.1,deadline-time.monotonic())))
        raise TimeoutError('turn completion')
    def close(self):
        if self.p.poll() is None:
            self.p.stdin.close()
            try:self.p.wait(timeout=8)
            except subprocess.TimeoutExpired:
                self.p.terminate()
                try:self.p.wait(timeout=8)
                except subprocess.TimeoutExpired:self.p.kill();self.p.wait(timeout=8)
        self.log.close()

client=None
try:
    FINDINGS['cli_version']=subprocess.check_output([CLI,'--version'],env=ENV,text=True).strip()
    FINDINGS['private_root']=str(ROOT)
    client=Client('seed')
    source=client.call('thread/start',{'cwd':str(ROOT/'workspace'),'model':'gpt-6.1-sol','modelProvider':'synthetic','historyMode':'legacy','approvalPolicy':'never','sandbox':'read-only'})
    source_id=source['thread']['id']
    started=client.call('turn/start',{'threadId':source_id,'input':[{'type':'text','text':'Synthetic original user task.','text_elements':[]}]})
    turn_id=started['turn']['id']; completed=client.completed(turn_id)
    assert completed['params']['turn']['status']=='completed',completed
    source_path=Path(source['thread']['path'])
    client.close();client=None
    history=[{'type':'message','id':'msg_retained_user','role':'user','content':[{'type':'input_text','text':'Synthetic retained user.'}]},{'type':'compaction','id':'cmp_synthetic','encrypted_content':OPAQUE},{'type':'message','id':'msg_retained_assistant','role':'assistant','content':[{'type':'output_text','text':'Synthetic retained tail.'}]}]
    with source_path.open('a') as file:
        file.write(json.dumps({'timestamp':'2026-11-12T00:00:00.000Z','type':'compacted','payload':{'message':'','replacement_history':history}})+'\n')
    baseline=source_path.read_bytes()
    FINDINGS['source_id']=source_id; FINDINGS['source_sha256']=hashlib.sha256(baseline).hexdigest()
    client=Client('sample')
    cases=[]
    def workspace_state():
        return {str(p.relative_to(ROOT/'workspace')):hashlib.sha256(p.read_bytes()).hexdigest() for p in (ROOT/'workspace').rglob('*') if p.is_file()}
    workspace_baseline=workspace_state()
    for mode in ('text','tool','empty'):
        MODE=mode
        selected=args.summary_model
        # The tool case deliberately removes filesystem sandbox protection:
        # absence of the canary then tests the sampler, not sandbox denial.
        fork=client.call('thread/fork',{'threadId':source_id,'model':selected,'modelProvider':'synthetic','ephemeral':False,'excludeTurns':True,'approvalPolicy':'never','sandbox':'danger-full-access' if mode=='tool' else 'read-only','threadSource':'velune_summary_probe','config':{'features.hooks':False,'features.goals':False,'features.plugins':False,'features.apps':False,'features.memories':False}})
        identity=fork['thread']['id']; fork_path=Path(fork['thread']['path'])
        assert identity!=source_id and fork_path!=source_path
        assert fork['model']==selected and fork['modelProvider']=='synthetic',fork
        turns=client.call('thread/turns/list',{'threadId':identity,'limit':10})
        entries=turns.get('data',turns.get('turns',[]))
        latest=entries[0]['id'] if entries else turn_id
        fork_baseline=fork_path.read_bytes()
        before_requests=len(REQUESTS)
        sample=client.call('thread/sample',{'threadId':identity,'expectedTurnId':latest,'prompt':'Produce a portable summary of the inherited task. Return only plain text. Do not execute any action.'})
        assert len(REQUESTS)==before_requests+1,REQUESTS[before_requests:]
        request=REQUESTS[-1]['body']
        assert request['model']==selected,request['model']
        assert any(x.get('type')=='compaction' and x.get('encrypted_content')==OPAQUE for x in request['input']),request['input']
        input_text=json.dumps(request['input'])
        assert 'Synthetic original user task.' not in input_text
        assert 'Synthetic retained user.' in input_text and 'Synthetic retained tail.' in input_text
        if mode=='text':assert sample['text'] and sample['text'].startswith('Synthetic portable summary'),sample
        else:assert sample.get('text') is None,sample
        assert not MARKER.exists()
        assert workspace_state()==workspace_baseline
        assert source_path.read_bytes()==baseline
        assert fork_path.read_bytes()==fork_baseline
        client.call('thread/archive',{'threadId':identity})
        archived=client.call('thread/read',{'threadId':identity,'includeTurns':False})['thread']
        archived_path=Path(archived['path'])
        assert 'archived_sessions' in archived_path.parts,archived_path
        assert archived_path.read_bytes()==fork_baseline
        assert source_path.read_bytes()==baseline
        cases.append({'mode':mode,'fork_id':identity,'selected_model':selected,'sample':sample,'opaque_preserved':True,'effective_compacted_window':True,'one_http_request':True,'source_unchanged':True,'fork_unchanged_by_sample':True,'tool_canary_absent':True,'workspace_unchanged':True,'fork_archived':True,'sandbox':fork['sandbox']})
        print('PASS',mode,flush=True)
    assert not any(r['direction']=='request' and r['data'].get('method')=='thread/resume' for r in RPC)
    assert not any(r['direction']=='response' and r['data'].get('method') and 'id' in r['data'] for r in RPC)
    FINDINGS['no_thread_resume']=True;FINDINGS['no_client_tool_or_approval_requests']=True
    FINDINGS['cases']=cases;FINDINGS['status']='passed'
except BaseException as error:
    FINDINGS['status']='failed';FINDINGS['error']=str(error);FINDINGS['traceback']=traceback.format_exc()
    print(FINDINGS['traceback'],flush=True)
finally:
    if client:client.close()
    server.shutdown();server.server_close()
    if not args.keep_fixture:
        shutil.rmtree(ROOT)
    FINDINGS['fixture_retained']=args.keep_fixture
    (OUT/'requests.json').write_text(json.dumps(REQUESTS,ensure_ascii=False,indent=2))
    (OUT/'rpc.json').write_text(json.dumps(RPC,ensure_ascii=False,indent=2))
    (OUT/'result.json').write_text(json.dumps(FINDINGS,ensure_ascii=False,indent=2))
    print(json.dumps(FINDINGS,ensure_ascii=False,indent=2),flush=True)
    if FINDINGS['status']!='passed':raise SystemExit(1)
