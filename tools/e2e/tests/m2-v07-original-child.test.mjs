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

test('V07 stopped worktree entry preserves child-disabled behavior and refuses missing or replaced child facts', () => {
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
 def __init__(self,tree,missing_custody=False,stdin_rows=None,source_rows=None,prior_rows=None):
  self.tree=tree; self.custody_reads=[]; self.missing_custody=missing_custody
  self.stdin_rows=stdin_rows or []; self.source_rows=source_rows or []; self.prior_rows=prior_rows or []
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
  if 'FROM gogoke_v37_h_stdin_journal' in sql:
   if 'operation=\'send\'' in sql and 'ORDER BY rowid' in sql: return [{'request_id':row['request_id']} for row in self.stdin_rows]
   return self.stdin_rows
  if 'FROM v37_ledger_raw_source' in sql:
   return self.source_rows if 'CAST(source_cursor AS INTEGER)>?' in sql else self.prior_rows
  return []
class HybridRow(dict):
 def __getitem__(self,key): return list(self.values())[key] if isinstance(key,int) else super().__getitem__(key)
with tempfile.TemporaryDirectory(prefix='gogoke-v07-reader-') as temp:
 root=Path(temp); state=root/'state'; state.mkdir(); tree_path=state/'v37-worktrees'/'tree'; tree_path.mkdir(parents=True)
 marker_path=tree_path/'ready.json'
 native.contained_physical=lambda root,child,child_must_exist=True: Path(child) if Path(child).name=='ready.json' else tree_path
 tree={'worktree_id':'tree','path_id':'path','repository_id':'repo','domain_id':'D','seat_id':'seat','seat_incarnation':'inc','seat_generation':1,'instance_id':'instance','worktree_path':str(tree_path),'worktree_identity':'identity','git_pointer_hash':'hash','git_pointer_identity':'pointer','common_identity':'common','baseline_commit':'commit','state':'REGISTERED','revision':7}
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
 def build_child_fixture():
  marker={'nonce':'child-nonce','pid':4567,'ppid':1234}; marker_raw=json.dumps(marker,separators=(',',':')).encode(); marker_path.write_bytes(marker_raw)
  body='one original prompt'; command='node signed-node-child-fixture'; value={'nonce':'child-nonce','pid':4567,'readyMarkerPath':str(marker_path)}
  terminal_frame={'type':'result','subtype':'success','is_error':False,'session_id':'vendor','result':json.dumps(value,separators=(',',':'))}
  terminal_raw=json.dumps(terminal_frame,separators=(',',':')).encode(); result_sha=__import__('hashlib').sha256(terminal_raw).hexdigest()
  user={'type':'user','session_id':'vendor','uuid':'user-uuid','parent_tool_use_id':None,'message':{'role':'user','content':[{'type':'text','text':body}]}}
  use={'type':'assistant','session_id':'vendor','message':{'content':[{'type':'tool_use','id':'tool-1','name':'Bash','input':{'command':command,'cwd':str(tree_path)}}]}}
  tool={'type':'user','session_id':'vendor','parent_tool_use_id':None,'message':{'role':'user','content':[{'type':'tool_result','tool_use_id':'tool-1','content':json.dumps(value,separators=(',',':')),'is_error':False}]}}
  answer={'type':'assistant','session_id':'vendor','message':{'content':[{'type':'text','text':json.dumps(value,separators=(',',':'))}]}}
  frames=[user,use,tool,answer,terminal_frame]
  states=[('NO_EVENT','CLAUDE_STDIN_ACK',None),('PENDING',None,None),('NO_EVENT',None,None),('RESOLVED',None,'event-4'),('NO_EVENT','CLAUDE_RESULT_RESPONSE',None)]
  raw_rows=[{'domain_id':'D','session_id':'session','generation':'3','operation_id':'process-op','source_epoch':'epoch','source_cursor':str(i+1),'raw_bytes':json.dumps(frame,separators=(',',':')).encode(),'state':states[i][0],'no_event_reason':states[i][1],'resolved_event_id':states[i][2],'process_ticket':'original-ticket','custodian_nonce':'original-nonce'} for i,frame in enumerate(frames)]
  init={'type':'system','subtype':'init','session_id':'vendor','claude_code_version':'2.1.196','cwd':str(tree_path)}
  prior=[{'source_epoch':'epoch','source_cursor':'0','raw_bytes':json.dumps(init,separators=(',',':')).encode(),'state':'NO_EVENT','no_event_reason':'CLAUDE_STARTUP_INIT'}]
  terminal_receipt={'requestId':'send-id','operation':'send','targetId':'session','status':'APPLIED','result':{'createdTurn':True,'deliveryBasis':'CLAUDE_USER_REPLAY_AND_RESULT','resultSubtype':'success','userUuid':'user-uuid','vendorSessionId':'vendor','sourceEpoch':'epoch','sourceCursor':'5','rawResultSha256':result_sha}}
  child={'state':'PRESTOP_IDENTITY_VERIFIED','generation':'3','sendRequestId':'send-id','beforeRawHighwaterRequestId':'before-id','terminalOutputRequestId':'output-id','beforeRawHighwater':'0','sourceEpoch':'epoch','terminalCursor':'5','vendorSessionId':'vendor','userUuid':'user-uuid','body':body,'cliVersion':'2.1.196','worktreePath':str(tree_path),'toolName':'Bash','exactCommand':command,'childNonce':'child-nonce','childPid':'4567','signedNodeSha256':'a'*64,'readyMarkerPath':str(marker_path),'readyMarkerSha256':__import__('hashlib').sha256(marker_raw).hexdigest(),'readyMarker':marker,'rawResultSha256':result_sha,'terminalReceipt':terminal_receipt,'processIdentity':{'processId':'4567','creationTime100ns':'100','imageSha256':'a'*64,'waitForSingleObjectResult':0x102,'aliveAtObservation':True,'observationTimeUtc':'2026-10-18T12:00:00+00:00'}}
  pre={'schema':'gogoke.37.operations.v1','family':'K-SESSION','operation':'output-stream','requestId':'before-id','domainId':'D','targetId':'session','expectedRevision':'0','payload':{'generation':'3','afterCursor':'0'}}
  sent={'schema':'gogoke.37.operations.v1','family':'K-SESSION','operation':'send','requestId':'send-id','domainId':'D','targetId':'session','expectedRevision':'0','payload':{'generation':'3','body':body}}
  output={'schema':'gogoke.37.operations.v1','family':'K-SESSION','operation':'output-stream','requestId':'output-id','domainId':'D','targetId':'session','expectedRevision':'0','payload':{'generation':'3','afterCursor':'4'}}
  more=[{'request':pre,'rawFrame':json.dumps(pre),'receipt':{'requestId':'before-id','operation':'output-stream','targetId':'session','status':'APPLIED','result':{'rawHighwater':'0'}}},
        {'request':sent,'rawFrame':json.dumps(sent),'receipt':terminal_receipt,'rawReceipt':json.dumps(terminal_receipt)},
        {'request':output,'rawFrame':json.dumps(output),'receipt':{'requestId':'output-id','operation':'output-stream','targetId':'session','status':'APPLIED','result':{'generation':'3','rawHighwater':'5','nativeInputReceipts':[{'requestId':'send-id','phase':'RECEIPTED','receipt':terminal_receipt}]}}}]
  denied_entry=journal['operations'][-1]
  operations=[entries[0],entries[1],entries[2],*more,denied_entry,entries[3],entries[4]]
  child['identityJournalOperationCount']=operations.index(denied_entry)
  case_child=dict(case); case_child['childCase']=child
  journal_child={'operations':operations}
  request_hex=json.dumps(sent,separators=(',',':')).encode().hex(); receipt_hex=json.dumps(terminal_receipt,separators=(',',':')).encode().hex()
  stdin={'phase':'RECEIPTED','receipt_status':'APPLIED','request_hex':request_hex,'session_id':'session','generation':'3','ticket':'original-ticket','process_operation_id':'process-op','custodian_nonce':'original-nonce','receipt_hex':receipt_hex,'request_id':'send-id'}
  return case_child,journal_child,DB(tree,stdin_rows=[stdin],source_rows=raw_rows,prior_rows=prior),stdin,sent,terminal_receipt
 child_case,child_journal,child_db,stdin,sent,terminal_receipt=build_child_fixture()
 positive=reader.worktree_facts(child_db,child_journal,child_case,'stopped')
 assert positive['childEvidence']['toolEvidence']['toolUseId']=='tool-1'
 assert positive['childEvidence']['toolEvidence']['rawResultSha256']==child_case['childCase']['rawResultSha256']
 assert positive['childEvidence']['sendRequestId']=='send-id' and positive['childEvidence']['acceptance'] is False
 assert child_db.custody_reads==[('D','process-op')]
 child_case,child_journal,missing_db,_,_,_=build_child_fixture(); missing_db.stdin_rows=[]
 try: reader.worktree_facts(missing_db,child_journal,child_case,'stopped')
 except RuntimeError as error: assert 'One original row required, found 0' in str(error)
 else: raise AssertionError('child-enabled original H stdin join with no durable row was accepted')
 for field,bad in [('request_hex','zz'),('request_hex','ff')]:
  child_case,child_journal,bad_db,_,_,_=build_child_fixture(); bad_db.stdin_rows[0][field]=bad
  try: reader.worktree_facts(bad_db,child_journal,child_case,'stopped')
  except (ValueError,UnicodeError): pass
  else: raise AssertionError('malformed child H request hex or UTF-8 was accepted')
 child_case,child_journal,bad_db,_,sent,_=build_child_fixture()
 replacement=dict(sent); replacement['payload']=dict(sent['payload'],body='replaced request')
 bad_db.stdin_rows[0]['request_hex']=json.dumps(replacement,separators=(',',':')).encode().hex()
 try: reader.worktree_facts(bad_db,child_journal,child_case,'stopped')
 except RuntimeError: pass
 else: raise AssertionError('replacement original H request was accepted')
 child_case,child_journal,bad_db,_,_,receipt=build_child_fixture()
 replacement=dict(receipt); replacement['requestId']='other-send'
 bad_db.stdin_rows[0]['receipt_hex']=json.dumps(replacement,separators=(',',':')).encode().hex()
 try: reader.worktree_facts(bad_db,child_journal,child_case,'stopped')
 except RuntimeError: pass
 else: raise AssertionError('replacement original H receipt was accepted')
`;
  const run = spawnSync(python, ['-c', source, stopReader], { encoding: 'utf8', windowsHide: true });
  assert.equal(run.status, 0, run.stderr || run.error?.message);
});

test('V07 complete child path reaches strict H stdin parsing through the real shared StopFact reader', () => {
  const python = process.env.GOGOKE_TEST_PYTHON ?? 'python';
  const stopReader = path.resolve(here, '..', 'm2-stop-worktree-readback.py');
  const source = String.raw`import hashlib,importlib.util,json,sqlite3,sys,tempfile
from pathlib import Path
sys.path.insert(0,str(Path(sys.argv[1]).parent))
spec=importlib.util.spec_from_file_location('stop_reader_real_core',sys.argv[1]); reader=importlib.util.module_from_spec(spec); spec.loader.exec_module(reader)
with tempfile.TemporaryDirectory(prefix='gogoke-v07-real-core-') as scratch:
 root=Path(scratch); state=root/'state'; tree_path=state/'v37-worktrees'/'tree'; tree_path.mkdir(parents=True)
 marker_path=tree_path/'ready.json'; marker={'nonce':'child-nonce','pid':4567,'ppid':1234}; marker_raw=json.dumps(marker,separators=(',',':')).encode(); marker_path.write_bytes(marker_raw)
 db=sqlite3.connect(':memory:'); db.row_factory=sqlite3.Row
 db.executescript('''
 CREATE TABLE v37_ledger_session(domain_id,session_id,seat_id,purpose,side_id);
 CREATE TABLE gogoke_v37_seats(domain_id,seat_id,incarnation);
 CREATE TABLE gogoke_v37_h_claim(domain_id,session_id,state,generation,process_operation_id,stop_fact_id,instance_id,revision);
 CREATE TABLE gogoke_v37_native_selection(domain_id,session_id,seat_id,seat_incarnation,seat_authorization_generation,selected_instance_id);
 CREATE TABLE gogoke_v37_session_binding_v2(domain_id,session_id,seat_id,seat_incarnation,seat_authorization_generation,selected_instance_id,provenance);
 CREATE TABLE gogoke_v37_h_seat_binding(domain_id,session_id);
 CREATE TABLE gogoke_v37_h_process_episode(domain_id,session_id,generation,process_operation_id,stop_fact_id,phase,instance_id,seat_id,seat_incarnation,stop_request_id,raw_hex);
 CREATE TABLE gogoke_coordination_process_custody(operation_id,domain_id,generation,state,stop_proof_hash,profile_id,ticket,custodian_nonce);
 CREATE TABLE gogoke_v37_h_operation(domain_id,request_id,session_id,operation,raw_hex,status,previous_revision,revision);
 CREATE TABLE gogoke_v37_worktrees(worktree_id,path_id,repository_id,domain_id,seat_id,seat_incarnation,seat_generation,instance_id,worktree_path,worktree_identity,git_pointer_hash,git_pointer_identity,common_identity,baseline_commit,state,revision);
 CREATE TABLE gogoke_v37_worktree_lifecycle(worktree_id,state,revision,stop_fact_id);
 CREATE TABLE gogoke_v37_worktree_lifecycle_ops(request_id,request_hash,worktree_id,operation,phase);
 CREATE TABLE gogoke_v37_h_stdin_journal(domain_id,session_id,request_id,operation,phase,receipt_status,request_hex,generation,ticket,process_operation_id,custodian_nonce,receipt_hex);
 CREATE TABLE v37_ledger_raw_source(domain_id,session_id,generation,operation_id,source_epoch,source_cursor,raw_bytes,state,process_ticket,custodian_nonce,no_event_reason,resolved_event_id);
 ''')
 db.execute('INSERT INTO v37_ledger_session VALUES(?,?,?,?,?)',('D','session','seat','WORK',None)); db.execute('INSERT INTO gogoke_v37_seats VALUES(?,?,?)',('D','seat','inc'))
 bind=('D','session','seat','inc','1','instance'); db.execute('INSERT INTO gogoke_v37_native_selection VALUES(?,?,?,?,?,?)',bind); db.execute('INSERT INTO gogoke_v37_session_binding_v2 VALUES(?,?,?,?,?,?,?)',(*bind,'NATIVE_V2'))
 stop_fact='sha256:'+'a'*64; names=['admission-reserve','admission-commit','open','stop','admission-release']; ids={name:name+'-id' for name in names}; entries=[]
 revisions={'admission-reserve':('0','1'),'admission-commit':('1','2'),'open':('2','3'),'stop':('3','4'),'admission-release':('4','5')}
 for name in names:
  previous,revision=revisions[name]; payload={'generation':'3'}
  if name=='open': payload.update({'seatId':'seat','repositoryId':'repo','worktreeId':'tree'})
  expected='4' if name=='admission-release' else previous
  req={'schema':'gogoke.37.operations.v1','family':'K-SESSION','operation':name,'requestId':ids[name],'domainId':'D','targetId':'session','expectedRevision':expected,'payload':payload}
  result={'stopFact':stop_fact} if name=='stop' else {}; receipt={'requestId':ids[name],'operation':name,'targetId':'session','status':'APPLIED','previousRevision':expected,'revision':revision,'result':result}
  raw=json.dumps(req,separators=(',',':')); entries.append({'request':req,'rawFrame':raw,'receipt':receipt})
  db.execute('INSERT INTO gogoke_v37_h_operation VALUES(?,?,?,?,?,?,?,?)',('D',ids[name],'session',name,raw.encode().hex(),'APPLIED',expected,revision))
 open_raw=next(item['rawFrame'] for item in entries if item['request']['operation']=='open')
 db.execute('INSERT INTO gogoke_v37_h_claim VALUES(?,?,?,?,?,?,?,?)',('D','session','RELEASED','3','process-op',stop_fact,'instance','5'))
 db.execute('INSERT INTO gogoke_v37_h_process_episode VALUES(?,?,?,?,?,?,?,?,?,?,?)',('D','session','3','process-op',stop_fact,'STOPPED','instance','seat','inc',ids['stop'],open_raw.encode().hex()))
 db.execute('INSERT INTO gogoke_coordination_process_custody VALUES(?,?,?,?,?,?,?,?)',('process-op','D','3','STOPPED',stop_fact,'instance','original-ticket','original-nonce'))
 tree_path_text=str(tree_path); tree=('tree','path','repo','D','seat','inc',1,'instance',tree_path_text,'tree-identity','git-hash','pointer-identity','common-identity','base-commit','REGISTERED',7)
 db.execute('INSERT INTO gogoke_v37_worktrees VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)',tree); db.execute('INSERT INTO gogoke_v37_worktree_lifecycle VALUES(?,?,?,?)',('tree','REGISTERED',7,''))
 body='one original prompt'; command='node signed-node-child-fixture'; marker_value={'nonce':'child-nonce','pid':4567,'readyMarkerPath':str(marker_path)}
 terminal_frame={'type':'result','subtype':'success','is_error':False,'session_id':'vendor','result':json.dumps(marker_value,separators=(',',':'))}; terminal_raw=json.dumps(terminal_frame,separators=(',',':')).encode(); result_sha=hashlib.sha256(terminal_raw).hexdigest()
 frames=[{'type':'user','session_id':'vendor','uuid':'user-uuid','parent_tool_use_id':None,'message':{'role':'user','content':[{'type':'text','text':body}]}},
 {'type':'assistant','session_id':'vendor','message':{'content':[{'type':'tool_use','id':'tool-1','name':'Bash','input':{'command':command,'cwd':tree_path_text}}]}},
 {'type':'user','session_id':'vendor','parent_tool_use_id':None,'message':{'role':'user','content':[{'type':'tool_result','tool_use_id':'tool-1','content':json.dumps(marker_value,separators=(',',':')),'is_error':False}]}},
 {'type':'assistant','session_id':'vendor','message':{'content':[{'type':'text','text':json.dumps(marker_value,separators=(',',':'))}]}},terminal_frame]
 states=[('NO_EVENT','CLAUDE_STDIN_ACK',None),('PENDING',None,None),('NO_EVENT',None,None),('RESOLVED',None,'event-4'),('NO_EVENT','CLAUDE_RESULT_RESPONSE',None)]
 for index,(frame,status) in enumerate(zip(frames,states),1): db.execute('INSERT INTO v37_ledger_raw_source VALUES(?,?,?,?,?,?,?,?,?,?,?,?)',('D','session','3','process-op','epoch',str(index),json.dumps(frame,separators=(',',':')).encode(),status[0],'original-ticket','original-nonce',status[1],status[2]))
 init={'type':'system','subtype':'init','session_id':'vendor','claude_code_version':'2.1.196','cwd':tree_path_text}
 db.execute('INSERT INTO v37_ledger_raw_source VALUES(?,?,?,?,?,?,?,?,?,?,?,?)',('D','session','3','process-op','epoch','0',json.dumps(init,separators=(',',':')).encode(),'NO_EVENT','original-ticket','original-nonce','CLAUDE_STARTUP_INIT',None))
 terminal_receipt={'requestId':'send-id','operation':'send','targetId':'session','status':'APPLIED','result':{'createdTurn':True,'deliveryBasis':'CLAUDE_USER_REPLAY_AND_RESULT','resultSubtype':'success','userUuid':'user-uuid','vendorSessionId':'vendor','sourceEpoch':'epoch','sourceCursor':'5','rawResultSha256':result_sha}}
 child={'state':'PRESTOP_IDENTITY_VERIFIED','generation':'3','sendRequestId':'send-id','beforeRawHighwaterRequestId':'before-id','terminalOutputRequestId':'output-id','beforeRawHighwater':'0','sourceEpoch':'epoch','terminalCursor':'5','vendorSessionId':'vendor','userUuid':'user-uuid','body':body,'cliVersion':'2.1.196','worktreePath':tree_path_text,'toolName':'Bash','exactCommand':command,'childNonce':'child-nonce','childPid':'4567','signedNodeSha256':'a'*64,'readyMarkerPath':str(marker_path),'readyMarkerSha256':hashlib.sha256(marker_raw).hexdigest(),'readyMarker':marker,'rawResultSha256':result_sha,'terminalReceipt':terminal_receipt,'processIdentity':{'processId':'4567','creationTime100ns':'100','imageSha256':'a'*64,'waitForSingleObjectResult':0x102,'aliveAtObservation':True,'observationTimeUtc':'2026-10-18T12:00:00+00:00'}}
 pre={'schema':'gogoke.37.operations.v1','family':'K-SESSION','operation':'output-stream','requestId':'before-id','domainId':'D','targetId':'session','expectedRevision':'0','payload':{'generation':'3','afterCursor':'0'}}
 sent={'schema':'gogoke.37.operations.v1','family':'K-SESSION','operation':'send','requestId':'send-id','domainId':'D','targetId':'session','expectedRevision':'0','payload':{'generation':'3','body':body}}
 output={'schema':'gogoke.37.operations.v1','family':'K-SESSION','operation':'output-stream','requestId':'output-id','domainId':'D','targetId':'session','expectedRevision':'0','payload':{'generation':'3','afterCursor':'4'}}
 more=[{'request':pre,'rawFrame':json.dumps(pre),'receipt':{'requestId':'before-id','operation':'output-stream','targetId':'session','status':'APPLIED','result':{'rawHighwater':'0'}}},
 {'request':sent,'rawFrame':json.dumps(sent),'receipt':terminal_receipt,'rawReceipt':json.dumps(terminal_receipt)},
 {'request':output,'rawFrame':json.dumps(output),'receipt':{'requestId':'output-id','operation':'output-stream','targetId':'session','status':'APPLIED','result':{'generation':'3','rawHighwater':'5','nativeInputReceipts':[{'requestId':'send-id','phase':'RECEIPTED','receipt':terminal_receipt}]}}}]
 denied={'schema':'gogoke.37.operations.v1','family':'K-WORKTREE','operation':'cleanup','requestId':'denied-id','domainId':'D','targetId':'tree','expectedRevision':'7','payload':{}}
 denied_entry={'request':denied,'rawFrame':json.dumps(denied),'receipt':{'requestId':'denied-id','operation':'cleanup','targetId':'tree','status':'DENIED','revision':'7'}}
 operations=[entries[0],entries[1],entries[2],*more,denied_entry,entries[3],entries[4]]; child['identityJournalOperationCount']=operations.index(denied_entry)
 case={'stateRoot':str(state),'domainId':'D','repositoryId':'repo','seatId':'seat','instanceId':'instance','worktreeId':'tree','graphRevision':7,'preStopCleanupRequestId':'denied-id','session':{'id':'session','generation':'3','reserveRequestId':ids['admission-reserve'],'commitRequestId':ids['admission-commit'],'openRequestId':ids['open'],'stopRequestId':ids['stop'],'releaseRequestId':ids['admission-release'],'stopFactId':stop_fact},'childCase':child}
 request_hex=json.dumps(sent,separators=(',',':')).encode().hex(); receipt_hex=json.dumps(terminal_receipt,separators=(',',':')).encode().hex()
 db.execute('INSERT INTO gogoke_v37_h_stdin_journal VALUES(?,?,?,?,?,?,?,?,?,?,?,?)',('D','session','send-id','send','RECEIPTED','APPLIED',request_hex,'3','original-ticket','process-op','original-nonce',receipt_hex))
 for req in (pre,sent,output): db.execute('INSERT INTO gogoke_v37_h_operation VALUES(?,?,?,?,?,?,?,?)',('D',req['requestId'],'session',req['operation'],json.dumps(req,separators=(',',':')).encode().hex(),'APPLIED','0','1'))
 journal={'operations':operations}; db.commit()
 disabled_case=dict(case); disabled_case.pop('childCase')
 disabled=reader.worktree_facts(db,journal,disabled_case,'stopped')
 assert disabled['childEvidence']=={'state':'NOT_RUN_OR_INCOMPLETE','acceptance':False}
 result=reader.worktree_facts(db,journal,case,'stopped')
 assert result['session']['stopFactId']==stop_fact and result['childEvidence']['toolEvidence']['toolUseId']=='tool-1'
 assert result['childEvidence']['acceptance'] is False and result['childEvidence']['toolEvidence']['rawResultSha256']==result_sha
 db.execute('UPDATE gogoke_v37_h_stdin_journal SET request_hex=? WHERE request_id=?',('zz','send-id'))
 try: reader.worktree_facts(db,journal,case,'stopped')
 except ValueError: pass
 else: raise AssertionError('invalid request hex was accepted')
 db.execute('UPDATE gogoke_v37_h_stdin_journal SET request_hex=? WHERE request_id=?',('ff','send-id'))
 try: reader.worktree_facts(db,journal,case,'stopped')
 except UnicodeDecodeError: pass
 else: raise AssertionError('invalid UTF-8 request was accepted')
 db.execute('UPDATE gogoke_v37_h_stdin_journal SET request_hex=? WHERE request_id=?',(json.dumps({**sent,'payload':{'generation':'3','body':'replacement'}},separators=(',',':')).encode().hex(),'send-id'))
 try: reader.worktree_facts(db,journal,case,'stopped')
 except RuntimeError: pass
 else: raise AssertionError('replacement original H request was accepted')
 db.execute('UPDATE gogoke_v37_h_stdin_journal SET request_hex=?,receipt_hex=? WHERE request_id=?',(request_hex,json.dumps({'requestId':'other-send','operation':'send','targetId':'session','status':'APPLIED','result':{}},separators=(',',':')).encode().hex(),'send-id'))
 try: reader.worktree_facts(db,journal,case,'stopped')
 except RuntimeError: pass
 else: raise AssertionError('replacement original H receipt was accepted')
`;
  const run = spawnSync(python, ['-c', source, stopReader], { encoding: 'utf8', windowsHide: true });
  assert.equal(run.status, 0, run.stderr || run.error?.message);
});
