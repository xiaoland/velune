import json,sys,os,time
mode=sys.argv[1]
root=os.path.dirname(__file__)
pending = None
count = 0
seen = set()
live_model = 'model/a'
auto_values = {'model/a':False,'model/b':'default'}
def emit(value): print(json.dumps(dict(jsonrpc='2.0',**value)),flush=True)
def chunk(text,session='synthetic'):
    emit({'method':'session/update','params':{'sessionId':session,'update':{'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':text}}}})
def form():
    emit({'id':'q','method':'elicitation/create','params':{'mode':'form','sessionId':'synthetic','message':'Value?', 'requestedSchema':{'type':'object','required':['value'],'properties':{'value':{'type':'string'}}}}})
def interactions():
    emit({'id':'p','method':'session/request_permission','params':{'sessionId':'synthetic','toolCall':{'toolCallId':'t','title':'Synthetic tool'},'options':[{'optionId':'yes','name':'Yes','kind':'allow_once'}]}})
    emit({'id':'q','method':'elicitation/create','params':{'mode':'form','sessionId':'synthetic','message':'Value?', 'requestedSchema':{'type':'object','required':['value'],'properties':{'value':{'type':'string'}}}}})
def options(value='model/a'):
    result=[{'id':'opaque/model-id','name':'Model','category':'model','type':'select','currentValue':value,'options':[{'value':'model/a','name':'A'},{'value':'model/b','name':'B'}]}]
    if mode=='model-auto-compaction':
        if value=='model/a':
            result.append({'id':'auto_compact_enabled','name':'Automatically compact','type':'boolean','currentValue':auto_values[value]})
        else:
            result.append({'id':'vendor.retention','name':'Retention','type':'select','currentValue':auto_values[value], 'options':[{'value':'default','name':'Default'},{'value':'less','name':'Less'}]})
        result.append({'id':'opaque/safety','name':'Permissions','category':'_permissions','type':'boolean','currentValue':False})
    return result

for line in sys.stdin:
    request=json.loads(line)
    method=request.get('method')
    params=request.get('params',{})
    if method=='initialize':
        assert params['clientCapabilities']['elicitation']['form']=={}
        result={'protocolVersion':1,'agentCapabilities':{'loadSession':True,'sessionCapabilities':{'list':{},'resume':{},'close':{},'delete':{}}},'authMethods':[]}
    elif method=='session/new' and mode=='reject-open':
        result={'sessionId':'rejected','configOptions':options() + options()}
    elif method=='session/new':
        if mode=='history':
            chunk('first')
            chunk('ignored','unknown')
            chunk(' second')
        result={'sessionId':'synthetic','configOptions':options()}
        if mode=='idle-form':
            emit({'id':request['id'],'result':result})
            form()
            continue
    elif method in ('session/load','session/resume') and mode=='reject-open':
        chunk('unconfirmed history',params['sessionId'])
        emit({'id':request['id'],'error':{'code':-32000,'message':'not opened','data':{'target':params['sessionId']}}})
        continue
    elif method=='session/load':
        chunk('authoritative replay',params['sessionId'])
        live_model='model/b'
        result={'configOptions':options(live_model)}
    elif method=='session/resume': result={'configOptions':options()}
    elif method=='session/list':
        result={'sessions':[]}
    elif method=='session/set_config_option' and mode=='model-auto-compaction':
        assert params['sessionId']=='synthetic'
        config,value=params['configId'],params['value']
        if config=='opaque/model-id':
            assert auto_values[live_model] == (False if live_model=='model/a' else 'default')
            live_model=value
        elif live_model=='model/a':
            assert config=='auto_compact_enabled' and params['type']=='boolean' and isinstance(value,bool)
            auto_values[live_model]=value
        else:
            assert config=='vendor.retention' and value in ('default','less')
            auto_values[live_model]=value
        with open(os.path.join(root,'setting-writes'),'a') as f: f.write(config+'\n')
        result={'configOptions':options(live_model)}
    elif method=='session/set_config_option':
        assert params['configId']=='opaque/model-id'
        if mode=='reject':
            emit({'id':request['id'],'error':{'code':-32000,'message':'synthetic rejected','data':{'native':'kept'}}})
            continue
        result={'configOptions':options('model/b')}
        if mode=='accepted-disconnect':
            emit({'id':request['id'],'result':result})
            emit({'id':'unsupported','method':'synthetic/unsupported','params':{}})
            sys.exit(0)
    elif method=='session/prompt':
        if mode=='model-auto-compaction':
            assert auto_values[live_model] == json.loads(params['prompt'][0]['text'])
            emit({'id':request['id'],'result':{'stopReason':'end_turn'}})
            continue
        if mode=='prompt-error':
            chunk('output before error')
            emit({'id':request['id'],'error':{'code':-32000,'message':'after execution','data':{'toolRan':True}}})
            continue
        if mode=='disconnect':
            form()
            while not os.path.exists(os.path.join(root,'disconnect-now')): time.sleep(0.005)
            chunk('output before disconnect')
            sys.exit(0)
        pending=request['id']
        if mode in ('duplicate-permission-first','duplicate-elicitation-first'):
            permission={'id':'same','method':'session/request_permission','params':{'sessionId':'synthetic','toolCall':{'toolCallId':'t'},'options':[{'optionId':'yes','name':'Yes','kind':'allow_once'}]}}
            form={'id':'same','method':'elicitation/create','params':{'mode':'form','sessionId':'synthetic','message':'Value?', 'requestedSchema':{'type':'object','properties':{'value':{'type':'string'}}}}}
            for value in ([permission,form] if mode=='duplicate-permission-first' else [form,permission]): emit(value)
            continue
        if mode=='live':
            emit({'id':'foreign','method':'session/request_permission','params':{'sessionId':'other','toolCall':{'toolCallId':'t'},'options':[{'optionId':'yes','name':'Yes','kind':'allow_once'}]}})
            interactions()
            continue
        count+=1
        form={'mode':'form','sessionId':'synthetic','message':'How many?',
              'requestedSchema':{'type':'object','required':['count'],'properties':{'count':{'type':'integer','minimum':1}}}}
        emit({'id':'q','method':'elicitation/create','params':form})
        if count==2: emit({'method':'$/cancel_request','params':{'requestId':'q'}})
        continue
    elif method=='session/cancel':
        if mode=='live':
            seen.add('cancel')
            chunk('cancel delivered')
            assert seen=={'p','q','foreign','cancel'}
            emit({'id':pending,'result':{'stopReason':'cancelled'}})
        continue
    elif method is None and mode=='idle-form':
        assert request['id']=='q' and request['result']=={'action':'accept','content':{'value':'still here'}}
        count+=1
        with open(os.path.join(root,'reply-count'),'w') as f: f.write(str(count))
        chunk('idle answer delivered')
        continue
    elif method is None and mode.startswith('duplicate-'):
        with open(os.path.join(root,'duplicate-reply'),'w') as f: f.write('unexpected response')
        continue
    elif method is None and mode=='live':
        seen.add(request['id'])
        if request['id']=='foreign':
            assert request['result']=={'outcome':{'outcome':'cancelled'}}
            continue
        if request['id']=='p': assert request['result']['outcome']['optionId']=='yes'
        else: assert request['result']['content']['value']=='answer'
        chunk(request['id']+' delivered')
        continue
    elif method is None and request['id']=='q':
        if count==1: assert request['result']['content']['count']==2
        else: assert request['result']['action']=='cancel'
        emit({'id':pending,'result':{'stopReason':'end_turn'}})
        continue
    else: result={}
    emit({'id':request['id'],'result':result})
