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
