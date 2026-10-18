import test from 'node:test';
import assert from 'node:assert/strict';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));

test('V07 child raw A/result binding rejects cursor, session, tool and PID substitutions', () => {
  const python = process.env.GOGOKE_TEST_PYTHON ?? 'python';
  const identityReader = path.resolve(here, '..', 'm2-provider-capture-readback.py');
  const stopReader = path.resolve(here, '..', 'm2-stop-worktree-readback.py');
  const source = `import ast,copy,hashlib,importlib.util,json,sys,types\n` +
    `from pathlib import Path\n` +
    `provider_tree=ast.parse(Path(sys.argv[1]).read_text(encoding='utf-8'))\n` +
    `provider_fn=next(node for node in provider_tree.body if isinstance(node,ast.FunctionDef) and node.name=='process_liveness_observation')\n` +
    `identity=types.SimpleNamespace(); exec(compile(ast.Module(body=[provider_fn],type_ignores=[]),sys.argv[1],'exec'),identity.__dict__)\n` +
    `sys.path.insert(0,str(Path(sys.argv[2]).parent)); spec=importlib.util.spec_from_file_location('stop_reader',sys.argv[2]); reader=importlib.util.module_from_spec(spec); spec.loader.exec_module(reader)\n` +
    `assert identity.process_liveness_observation(0x102)=={'waitForSingleObjectResult':0x102,'aliveAtObservation':True}\n` +
    `for value,error,needle in [(0,0,'WAIT_OBJECT_0'),(0xffffffff,5,'Win32 error=5'),(7,0,'unexpected status')]:\n` +
    ` try: identity.process_liveness_observation(value,error)\n except RuntimeError as e: assert needle in str(e)\n else: raise AssertionError('non-live/unknown wait status passed')\n` +
    `nonce='case_nonce_123'; pid='4567'; marker_path=r'D:\\F\\.case-ready.json'; body='one original prompt'; command='signed-node-fixture --bound'\n` +
    `child={'beforeRawHighwater':'0','terminalCursor':'5','sourceEpoch':'epoch','vendorSessionId':'vendor',` +
    `'processOperationId':'op','generation':'3','ticket':'ticket','custodianNonce':'custody','cliVersion':'2.1.196',` +
    `'worktreePath':r'D:\\F','body':body,'userUuid':'user-uuid','rawResultSha256':'',` +
    `'toolName':'Bash','exactCommand':command,'childNonce':nonce,'childPid':pid,'readyMarkerPath':marker_path}\n` +
    `child['sendRequestId']='send-1'; child['terminalOutputRequestId']='out-1'; child['terminalReceipt']={'requestId':'send-1','status':'APPLIED','result':{'sourceEpoch':'epoch','sourceCursor':'5'}}\n` +
    `out_request={'schema':'gogoke.37.operations.v1','family':'K-SESSION','operation':'output-stream','requestId':'out-1','domainId':'D','targetId':'h','expectedRevision':'4','payload':{'generation':'3','afterCursor':'4'}}\n` +
    `out_page={'generation':'3','rawHighwater':'5','nativeInputReceipts':[{'requestId':'send-1','phase':'RECEIPTED','receipt':child['terminalReceipt']}]}\n` +
    `out_entry={'request':out_request,'rawFrame':json.dumps(out_request),'receipt':{'requestId':'out-1','operation':'output-stream','targetId':'h','result':out_page}}\n` +
    `assert reader.child_terminal_output_receipt({'operations':[out_entry]},{'id':'h','generation':'3'},child)['receipt']==child['terminalReceipt']\n` +
    `bad_child=copy.deepcopy(child); bad_child['sendRequestId']='other-send'\n` +
    `try: reader.child_terminal_output_receipt({'operations':[out_entry]},{'id':'h','generation':'3'},bad_child)\n` +
    `except RuntimeError: pass\nelse: raise AssertionError('wrong H send ID output receipt passed')\n` +
    `def raw(cursor,frame,state='PENDING',reason=None,event=None):\n` +
    ` b=json.dumps(frame,separators=(',',':')).encode(); return {'source_epoch':'epoch','source_cursor':str(cursor),` +
    `'raw_bytes':b,'state':state,'no_event_reason':reason,'resolved_event_id':event,'operation_id':'op',` +
    `'generation':'3','process_ticket':'ticket','custodian_nonce':'custody'}\n` +
    `frames=[\n` +
    ` raw(1,{'type':'user','session_id':'vendor','uuid':'user-uuid','parent_tool_use_id':None,'message':{'role':'user','content':[{'type':'text','text':body}]}},'NO_EVENT','CLAUDE_STDIN_ACK'),\n` +
    ` raw(2,{'type':'assistant','session_id':'vendor','message':{'content':[{'type':'tool_use','id':'tool-1','name':'Bash','input':{'command':command}}]}}),\n` +
    ` raw(3,{'type':'user','session_id':'vendor','parent_tool_use_id':None,'message':{'role':'user','content':[{'type':'tool_result','tool_use_id':'tool-1','content':json.dumps({'nonce':nonce,'pid':int(pid),'readyMarkerPath':marker_path}),'is_error':False}]} }),\n` +
    ` raw(4,{'type':'assistant','session_id':'vendor','message':{'content':[{'type':'text','text':json.dumps({'nonce':nonce,'pid':int(pid),'readyMarkerPath':marker_path})}] }},'RESOLVED',None,'event-4'),\n` +
    ` raw(5,{'type':'result','subtype':'success','is_error':False,'session_id':'vendor','result':json.dumps({'nonce':nonce,'pid':int(pid),'readyMarkerPath':marker_path})},'NO_EVENT','CLAUDE_RESULT_RESPONSE')]\n` +
    `terminal=frames[-1]; child['rawResultSha256']=hashlib.sha256(terminal['raw_bytes']).hexdigest()\n` +
    `init={'type':'system','subtype':'init','session_id':'vendor','claude_code_version':'2.1.196','cwd':child['worktreePath']}\n` +
    `facts=reader.child_tool_source_facts(frames,init,child); assert facts['toolUseId']=='tool-1' and facts['toolResultSourceCursor']=='3' and facts['rawResultSha256']==child['rawResultSha256']\n` +
    `def reject(rows=frames,init_frame=init,case=child):\n` +
    ` try: reader.child_tool_source_facts(rows,init_frame,case)\n except RuntimeError: return\n raise AssertionError('invalid original A child binding passed')\n` +
    `bad=copy.deepcopy(frames); bad[2]['raw_bytes']=bad[2]['raw_bytes'].replace(b'tool-1',b'tool-x'); reject(bad)\n` +
    `bad=copy.deepcopy(frames); bad[2]['raw_bytes']=bad[2]['raw_bytes'].replace(b'vendor',b'other'); reject(bad)\n` +
    `bad=copy.deepcopy(frames); bad[1]['raw_bytes']=bad[1]['raw_bytes'].replace(command.encode(),b'other-command'); reject(bad)\n` +
    `bad=copy.deepcopy(frames); bad[2]['state']='RESOLVED'; bad[2]['resolved_event_id']='event-tool'; reject(bad)\n` +
    `bad=copy.deepcopy(frames); bad[0]['no_event_reason']='wrong'; reject(bad)\n` +
    `bad=copy.deepcopy(frames); bad[2]['source_cursor']='6'; reject(bad)\n` +
    `bad=copy.deepcopy(frames); bad[1]['process_ticket']='other-ticket'; reject(bad)\n` +
    `bad=copy.deepcopy(frames); bad.insert(3,copy.deepcopy(bad[1])); bad[3]['source_cursor']='4'; reject(bad)\n` +
    `bad=copy.deepcopy(child); bad['sourceEpoch']='other'\n` +
    `try: reader.child_tool_source_facts(frames,init,bad)\n` +
    `except RuntimeError: pass\n` +
    `else: raise AssertionError('wrong source epoch passed')\n` +
    `process={'processIdentity':{'processId':pid,'creationTime100ns':'100','imageSha256':'a'*64,` +
    `'waitForSingleObjectResult':0x102,'aliveAtObservation':True,'observationTimeUtc':'2026-10-18T12:00:00+00:00'},` +
    `'childPid':pid,'childNonce':nonce,'signedNodeSha256':'a'*64,'readyMarkerSha256':'b'*64}\n` +
    `marker={'nonce':nonce,'pid':int(pid),'ppid':111}\n` +
    `assert reader.child_identity_facts(process,marker)['aliveAtObservation'] is True\n` +
    `for change in [('waitForSingleObjectResult',0),('aliveAtObservation',False),('processId','9999')]:\n` +
    ` bad=copy.deepcopy(process); bad['processIdentity'][change[0]]=change[1]\n` +
    ` try: reader.child_identity_facts(bad,marker)\n except RuntimeError: pass\n else: raise AssertionError('invalid same-handle identity passed')\n`;
  const run = spawnSync(python, ['-c', source, identityReader, stopReader],
    { encoding: 'utf8', windowsHide: true });
  assert.equal(run.status, 0, run.stderr || run.error?.message);
});

test('V07 stopped worktree entry uses the original exact custody row and rejects a missing child H join', () => {
  const python = process.env.GOGOKE_TEST_PYTHON ?? 'python';
  const stopReader = path.resolve(here, '..', 'm2-stop-worktree-readback.py');
  const source = String.raw`import importlib.util,json,sys,tempfile
from pathlib import Path
sys.path.insert(0,str(Path(sys.argv[1]).parent))
spec=importlib.util.spec_from_file_location('stop_reader',sys.argv[1]); reader=importlib.util.module_from_spec(spec); spec.loader.exec_module(reader)
native=reader.native
binding={'seat_id':'seat','seat_incarnation':'inc','selected_instance_id':'instance'}
native.verify_binding=lambda *args,**kwargs:{'binding':binding,'selection':None,'mode':'native_v2'}
stop_fact='sha256:'+'a'*64
custody={'operation_id':'process-op','domain_id':'D','generation':'3','state':'STOPPED','stop_proof_hash':stop_fact,'ticket':'original-ticket','custodian_nonce':'original-nonce'}
claim={'state':'RELEASED','generation':'3','process_operation_id':'process-op','stop_fact_id':stop_fact,'instance_id':'instance','revision':'5'}
episode={'generation':'3','process_operation_id':'process-op','stop_fact_id':stop_fact,'phase':'STOPPED','instance_id':'instance','seat_id':'seat','seat_incarnation':'inc','stop_request_id':'stop-id','raw_hex':''}
native.verify_stop_tuple=lambda *args,**kwargs:{'claim':claim,'episode':episode,'custody':custody,'binding':binding,'bindingMode':'native_v2','bindingEvidence':{'binding':binding,'selection':None},'stopFactId':stop_fact}
names=['admission-reserve','admission-commit','open','stop','admission-release']
ids={name:name+'-id' for name in names}
entries=[]; durable={}
revisions={'admission-reserve':('0','1'),'admission-commit':('1','2'),'open':('2','3'),'stop':('3','4'),'admission-release':('4','5')}
for name in names:
 previous,revision=revisions[name]
 payload={'generation':'3'}
 if name=='open': payload.update({'seatId':'seat','repositoryId':'repo','worktreeId':'tree'})
 expected='4' if name=='admission-release' else previous
 request={'schema':'gogoke.37.operations.v1','family':'K-SESSION','operation':name,'requestId':ids[name],'domainId':'D','targetId':'session','expectedRevision':expected,'payload':payload}
 result={'stopFact':stop_fact} if name=='stop' else {}
 receipt={'requestId':ids[name],'operation':name,'targetId':'session','status':'APPLIED','previousRevision':expected,'revision':revision,'result':result}
 raw=json.dumps(request,separators=(',',':'))
 entry={'request':request,'rawFrame':raw,'receipt':receipt}; entries.append(entry)
 durable[ids[name]]={'raw_hex':raw.encode().hex(),'operation':name,'session_id':'session','status':'APPLIED','previous_revision':expected,'revision':revision}
opened_raw=next(row['rawFrame'] for row in entries if row['request']['operation']=='open')
episode['raw_hex']=opened_raw.encode().hex()
class DB:
 def __init__(self,tree,missing_custody=False): self.tree=tree; self.custody_reads=[]; self.missing_custody=missing_custody
 def execute(self,sql,args=()):
  if 'FROM v37_ledger_session' in sql: return [{'domain_id':'D','seat_id':'seat','purpose':'WORK','side_id':''}]
  if 'FROM gogoke_v37_seats' in sql: return [{'incarnation':'inc'}]
  if 'FROM gogoke_v37_h_claim' in sql and 'SELECT process_operation_id' in sql: return [{'process_operation_id':'process-op'}]
  if 'FROM gogoke_v37_worktrees' in sql: return [self.tree]
  if 'FROM gogoke_coordination_process_custody' in sql:
   self.custody_reads.append(tuple(args)); return [custody] if not self.missing_custody and tuple(args)==('D','process-op') else []
  if 'FROM gogoke_v37_h_operation WHERE domain_id=? AND request_id=?' in sql: return [durable[args[1]]] if args[1] in durable else []
  if 'SELECT domain_id,session_id,raw_hex FROM gogoke_v37_h_operation' in sql: return [HybridRow(domain_id='D',session_id='session',raw_hex=episode['raw_hex'])]
  if 'FROM gogoke_v37_worktree_lifecycle WHERE worktree_id=?' in sql: return [{'state':'REGISTERED','revision':7,'stop_fact_id':''}]
  if 'FROM gogoke_v37_worktree_lifecycle_ops WHERE worktree_id=?' in sql: return []
  if 'FROM gogoke_v37_h_stdin_journal' in sql: return []
  return []
class HybridRow(dict):
 def __getitem__(self,key): return list(self.values())[key] if isinstance(key,int) else super().__getitem__(key)
native.contained_physical=lambda root,child,child_must_exist=True: tree_path
with tempfile.TemporaryDirectory(prefix='gogoke-v07-reader-') as temp:
 root=Path(temp); state=root/'state'; state.mkdir(); tree_path=state/'v37-worktrees'/'tree'; tree_path.mkdir(parents=True)
 tree={'worktree_id':'tree','path_id':'path','repository_id':'repo','domain_id':'D','seat_id':'seat','seat_incarnation':'inc','seat_generation':1,'instance_id':'instance','worktree_path':'tree','worktree_identity':'identity','git_pointer_hash':'hash','git_pointer_identity':'pointer','common_identity':'common','baseline_commit':'commit','state':'REGISTERED','revision':7}
 case={'stateRoot':str(state),'domainId':'D','repositoryId':'repo','seatId':'seat','instanceId':'instance','worktreeId':'tree','graphRevision':7,'preStopCleanupRequestId':'denied-id','session':{'id':'session','generation':'3','reserveRequestId':ids['admission-reserve'],'commitRequestId':ids['admission-commit'],'openRequestId':ids['open'],'stopRequestId':ids['stop'],'releaseRequestId':ids['admission-release'],'stopFactId':stop_fact}}
 denied={'schema':'gogoke.37.operations.v1','family':'K-WORKTREE','operation':'cleanup','requestId':'denied-id','domainId':'D','targetId':'tree','expectedRevision':'7','payload':{}}
 journal={'operations':entries+[{'request':denied,'rawFrame':json.dumps(denied),'receipt':{'requestId':'denied-id','operation':'cleanup','targetId':'tree','status':'DENIED','revision':'7'}}]}
 db=DB(tree)
 normal=reader.worktree_facts(db,journal,case,'stopped')
 assert normal['session']['processTicket']=='original-ticket' and normal['session']['custodianNonce']=='original-nonce'
 assert 'custody' not in normal['session'] and normal['session']['custodyState']=='STOPPED'
 assert normal['childEvidence']=={'state':'NOT_RUN_OR_INCOMPLETE','acceptance':False}
 assert db.custody_reads==[('D','process-op')]
 try: reader.worktree_facts(DB(tree,missing_custody=True),journal,case,'stopped')
 except RuntimeError as error: assert 'One original row required, found 0' in str(error)
 else: raise AssertionError('missing original custody ticket/nonce row was accepted')
 child={'state':'PRESTOP_IDENTITY_VERIFIED','sendRequestId':'send-id','beforeRawHighwaterRequestId':'before-id','terminalOutputRequestId':'output-id','beforeRawHighwater':'0','sourceEpoch':'epoch','terminalCursor':'5','body':'one original prompt'}
 pre={'schema':'gogoke.37.operations.v1','family':'K-SESSION','operation':'output-stream','requestId':'before-id','domainId':'D','targetId':'session','expectedRevision':'0','payload':{'generation':'3','afterCursor':'0'}}
 sent={'schema':'gogoke.37.operations.v1','family':'K-SESSION','operation':'send','requestId':'send-id','domainId':'D','targetId':'session','expectedRevision':'0','payload':{'generation':'3','body':'one original prompt'}}
 terminal={'requestId':'send-id','status':'APPLIED','result':{'sourceEpoch':'epoch','sourceCursor':'5'}}
 output={'schema':'gogoke.37.operations.v1','family':'K-SESSION','operation':'output-stream','requestId':'output-id','domainId':'D','targetId':'session','expectedRevision':'0','payload':{'generation':'3','afterCursor':'4'}}
 more=[{'request':pre,'rawFrame':json.dumps(pre),'receipt':{'requestId':'before-id','operation':'output-stream','targetId':'session','status':'APPLIED','result':{'rawHighwater':'0'}}},
       {'request':sent,'rawFrame':json.dumps(sent),'receipt':{'requestId':'send-id','operation':'send','targetId':'session','status':'APPLIED'}},
       {'request':output,'rawFrame':json.dumps(output),'receipt':{'requestId':'output-id','operation':'output-stream','targetId':'session','status':'APPLIED','result':{'generation':'3','rawHighwater':'5','nativeInputReceipts':[{'requestId':'send-id','phase':'RECEIPTED','receipt':terminal}]}}}]
 child.update({'terminalReceipt':terminal})
 case['childCase']=child; journal['operations'].extend(more)
 try: reader.worktree_facts(db,journal,case,'stopped')
 except RuntimeError as error: assert 'One original row required, found 0' in str(error)
 else: raise AssertionError('child-enabled original H stdin join with no durable row was accepted')
`;
  const run = spawnSync(python, ['-c', source, stopReader], { encoding: 'utf8', windowsHide: true });
  assert.equal(run.status, 0, run.stderr || run.error?.message);
});
