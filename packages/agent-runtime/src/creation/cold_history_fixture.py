#!/usr/bin/env python3
import json, os, sys
if '--version' in sys.argv:
    print('codex-cli 0.166.0')
    sys.exit(0)
home = os.environ['CODEX_HOME']
with open(os.path.join(home, 'rpc.pid'), 'w') as out:
    out.write(str(os.getpid()))
legacy = os.path.exists(os.path.join(home, 'legacy'))
long_history = os.path.exists(os.path.join(home, 'long-history'))
def item(n):
    return {'type':'userMessage','id':'message-'+str(n),'content':[{'type':'text','text':'synthetic-'+str(n)}]}
for line in sys.stdin:
    request = json.loads(line)
    if 'id' not in request:
        continue
    method = request['method']
    params = request.get('params', {})
    if method == 'initialize':
        result = {}
    elif method == 'thread/read':
        thread = {'id':'synthetic','name':'synthetic','cwd':home,'historyMode':'legacy' if legacy else 'paginated'}
        if params.get('includeTurns'):
            thread['turns'] = [{'id':'turn','status':'completed','items':[item(1),item(2)]}]
        result = {'thread':thread}
    elif method == 'thread/items/list':
        if long_history:
            page = int(params.get('cursor', '0'))
            results = [{'type':'commandExecution', 'id':f'item-{page}-{i}', 'command':'synthetic',
                        'status':'completed', 'aggregatedOutput':f'page-{page}-{i}\n' + 'x' * (256 * 1024)}
                       for i in range(4)]
            result = {'data':[{'turnId':'turn','item':value} for value in results],
                      'nextCursor': str(page + 1) if page + 1 < 165 else None}
        else:
            second = params.get('cursor') == 'second'
            result = {'data':[{'turnId':'turn','item':item(2 if second else 1)}], 'nextCursor':None if second else 'second'}
    elif method == 'thread/turns/list':
        result = {'data':[{'id':'turn','status':'completed'}], 'nextCursor':None}
    else:
        raise AssertionError('unexpected synthetic RPC: '+method)
    print(json.dumps({'id':request['id'],'result':result}), flush=True)
