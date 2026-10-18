import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { closeFixture } from './causal-close-fixture.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));

test('Grok Host-MCP immutable reader joins original H, A and C facts and rejects detached evidence',
  { skip: process.platform !== 'win32' }, () => {
    const tempRoot = process.env.GOGOKE_E2E_TEST_TEMP_ROOT;
    assert.ok(tempRoot && path.isAbsolute(tempRoot), 'Explicit private test scratch directory required');
    const dir = fs.mkdtempSync(path.join(tempRoot, 'grok-host-mcp-'));
    try {
      const python = process.env.GOGOKE_TEST_PYTHON ?? 'python';
      const reader = path.resolve(here, '..', 'm2-provider-capture-readback.py');
      const causalCloseFixture = Buffer.from(JSON.stringify(
        closeFixture(123, 'SPAWNED_NODE_CHILD'))).toString('base64');
      const source = String.raw`import ast, hashlib, json, re, sqlite3, subprocess, sys
import base64, os
from pathlib import Path

reader_path, temp_root = Path(sys.argv[1]), Path(sys.argv[2])
source = reader_path.read_text(encoding='utf-8')
tree = ast.parse(source)
needed = {'sha', 'fail', 'exact_one', 'local_spelling', 'same_local_path',
          'grok_host_mcp_question_evidence', 'grok_host_mcp_causal_close_evidence',
          'provider_normal_close_readback_entry'}
pieces = [ast.get_source_segment(source, node) for node in tree.body
          if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name in needed]
if {node.name for node in tree.body if isinstance(node, ast.FunctionDef) and node.name in needed} != needed:
    raise AssertionError('actual immutable readback helper definitions are incomplete')
namespace = {'hashlib': hashlib, 'json': json, 're': re, 'os': os, 'Path': Path,
             '__file__': str(reader_path)}
exec('\n\n'.join(pieces), namespace)
readback = namespace['grok_host_mcp_question_evidence']
normal_close_entry = namespace['provider_normal_close_readback_entry']

close_base = json.loads(base64.b64decode('${causalCloseFixture}'))
close_base['causalCloseAttempts'][0]['schema'] = 'gogoke.37.causal-close-attempt.v1'
def close_entry_journal(case_mode='grokHostMcpQuestion'):
    value = json.loads(json.dumps(close_base))
    value.update({'sourceCommit':'a'*40,'installedVersion':'1.0.41',
        'installedPath':close_base['boundInstalledIdentity']['installed'],
        'installedSha256':{**close_base['boundInstalledIdentity']['installedSha256'],
                           'resource-index.json':'3'*64},
        'launches':[{'pid':123,'sourceCommit':'a'*40,'bootstrap':{'version':'1.0.41'}}],
        'productExits':[{'pid':123,'code':0}],
        'cases':[{'driverId':'grok','caseMode':case_mode}]})
    return value

actual_close = normal_close_entry(close_entry_journal())
assert actual_close['mode'] == 'SPAWNED_NODE_CHILD'
assert actual_close['originalChildExitCode'] == 0
for name, mutate in [
    ('missing-causal-proof', lambda value: value.update({'causalCloseAttempts':[]})),
    ('source-pin-mismatch', lambda value: value['causalCloseAttempts'][0]['source']['module'].__setitem__('sha256','0'*64)),
    ('caption-binding-mismatch', lambda value: value['closes'][0]['causalProof'].__setitem__('resumedOriginalCall',False)),
    ('writer-only-close', lambda value: value['closes'][0].update({'observationMode':'NORMAL_CLOSE_ONLY'})),
]:
    malformed = close_entry_journal()
    mutate(malformed)
    try: normal_close_entry(malformed)
    except RuntimeError: pass
    else: raise AssertionError(name+' causal normal-close evidence was accepted')
ordinary = close_entry_journal('singleAnswer')
ordinary['causalCloseAttempts'] = []
assert normal_close_entry(ordinary) is None

domain, sid, vendor, seat, instance, worktree = 'test-domain', 'session-original', 'grok-thread', 'seat-grok', 'grokTestM2', 'tree-original'
generation, process_op, ticket, nonce, open_id = '19', 'process-original', 'ticket-original', 'nonce-original', 'open-request'
server = 'srv_0123456789abcdef'
fixed = [
    {'header':'Format','question':'Which non-secret format should this marker use?',
     'options':[{'label':'JSON','description':'Use the JSON format.'},{'label':'Plain','description':'Use plain text.'}],
     'recommendedIndex':1,'answerMode':'RECOMMENDED_OPTION'},
    {'header':'Label','question':'What exact non-secret label should this marker use?',
     'options':[{'label':'Alpha','description':'Use the first fixed label.'},{'label':'Beta','description':'Use the second fixed label.'}],
     'recommendedIndex':0,'answerMode':'FREE_TEXT'},
]
free_answer = 'a1b2c3d4e5f60718293a4b5c6d7e8f90123456789abcdef0'
canonical = lambda value: json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(',',':'))
digest = lambda value: hashlib.sha256(value).hexdigest()

def make_operation(family, operation, request_id, target, payload, revision='0'):
    request = {'schema':'gogoke.37.operations.v1','family':family,'operation':operation,
        'requestId':request_id,'domainId':domain,'targetId':target,'expectedRevision':revision,
        'payload':payload}
    return {'request':request,'rawFrame':json.dumps(request,ensure_ascii=False,separators=(',',':'))}

def add_raw(db, cursor, raw, state='NO_EVENT', reason='ACP_RPC_RESPONSE', event=None):
    db.execute('INSERT INTO v37_ledger_raw_source(operation_id,process_ticket,custodian_nonce,domain_id,session_id,'
        'generation,source_epoch,source_cursor,raw_bytes,state,resolved_event_id,no_event_reason) '
        'VALUES(?,?,?,?,?,?,?,?,?,?,?,?)',
        (process_op,ticket,nonce,domain,sid,generation,nonce,str(cursor),raw,state,event,reason))

repo = reader_path.parents[2]
def rust_const(relative, name):
    text=(repo/relative).read_text(encoding='utf-8')
    match=re.search(r'const '+re.escape(name)+r'\s*:\s*&str\s*=\s*"([\s\S]*?)";',text)
    if not match: raise AssertionError('missing production Rust DDL constant '+name)
    return match.group(1)
def inbox_table(name):
    text=(repo/'apps/desktop/native-host/src/store/inbox/mod.rs').read_text(encoding='utf-8')
    match=re.search(r'\(\s*"'+re.escape(name)+r'",\s*"(CREATE TABLE [^"]+)"\)',text)
    if not match: raise AssertionError('missing production inbox DDL '+name)
    return match.group(1)
def instance_table(name):
    text=(repo/'apps/desktop/native-host/src/store/instance/mod.rs').read_text(encoding='utf-8')
    marker='"'+name+'"'
    if marker not in text: raise AssertionError('missing production instance DDL '+name)
    row=text.split(marker,1)[1]
    start=row.index('"CREATE TABLE')+1
    end=row.index('"',start)
    return row[start:end]
def schema_table(name):
    text=(repo/'apps/desktop/native-host/src/store/ledger/schema.sql').read_text(encoding='utf-8')
    match=re.search(r'CREATE TABLE IF NOT EXISTS '+re.escape(name)+r'\s*\([\s\S]*?\) STRICT;',text)
    if not match: raise AssertionError('missing production ledger DDL '+name)
    return match.group(0)

def add_step(db, step_id, command, phase, requires, source_epoch=None, source_cursor=None):
    db.execute('INSERT INTO gogoke_v37_rpc_steps(domain_id,session_id,open_request_id,step_id,'
        'process_operation_id,ticket,custodian_nonce,pid,creation_time,image_path,binary_digest,profile_id,'
        'generation,command_hex,requires_response,phase,source_epoch,source_cursor) '
        'VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)',
        (domain,sid,open_id,step_id,process_op,ticket,nonce,'123','100000','D:/test/grok.exe',
         'sha256:fixed','grokTestM2',generation,command.hex(),requires,phase,source_epoch,source_cursor))

def fixture(name):
    db_path = temp_root / (name + '.sqlite')
    db = sqlite3.connect(db_path)
    production_ddl = [
        rust_const('apps/desktop/native-host/src/store/ledger/mod.rs','RAW_SOURCE_SCHEMA'),
        schema_table('v37_ledger_index'), schema_table('v37_ledger_receipt'),
        rust_const('apps/desktop/native-host/src/store/session_transport/rpc_journal.rs','SCHEMA'),
        rust_const('apps/desktop/native-host/src/store/session_transport/episodes.rs','PROCESS_SCHEMA'),
        instance_table('gogoke_v37_instances'),
        inbox_table('gogoke_v37_qcard_native'), inbox_table('gogoke_v37_qcard_native_options'),
        inbox_table('gogoke_v37_qcard_native_operations'),
        re.search(r'\("gogoke_v37_h_stdin_journal",\s*"(CREATE TABLE [^"]+)"\)',
            (repo/'apps/desktop/native-host/src/store/session_transport/admission.rs').read_text(encoding='utf-8')).group(1),
        re.search(r'const SEATS:\s*&str\s*=\s*"([^"]+)";',
            (repo/'apps/desktop/native-host/src/store/seat/mod.rs').read_text(encoding='utf-8')).group(1),
        re.search(r'const SETTINGS:\s*&str\s*=\s*"([^"]+)";',
            (repo/'apps/desktop/native-host/src/store/seat/mod.rs').read_text(encoding='utf-8')).group(1),
    ]
    raw_ddl = production_ddl[0]
    producer_source=(repo/'apps/desktop/native-host/src/store/session_transport/rpc_journal.rs').read_text(encoding='utf-8')
    assert 'HOST_GROK_MCP_QUESTION_CARD' in producer_source and 'ACP_RPC_RESPONSE' in producer_source
    normalized_raw_ddl = ' '.join(raw_ddl.split())
    assert "source_cursor <> '0'" in normalized_raw_ddl
    assert "substr(raw_bytes, -1, 1) = X'0A'" in normalized_raw_ddl
    assert "state = 'PENDING' AND resolved_event_id IS NULL AND no_event_reason IS NULL" in normalized_raw_ddl
    assert "state = 'NO_EVENT' AND resolved_event_id IS NULL AND no_event_reason IS NOT NULL" in normalized_raw_ddl
    for statement in production_ddl:
        db.execute(statement)
    db.execute('PRAGMA foreign_keys=ON')
    for bad_cursor,bad_raw,bad_state,bad_event,bad_reason in [
        ('0',b'{}\n','NO_EVENT',None,'ACP_RPC_RESPONSE'),
        ('1',b'{}','NO_EVENT',None,'ACP_RPC_RESPONSE'),
        ('1',b'{}\n','PENDING',None,'ACP_RPC_RESPONSE'),
        ('1',b'{}\n','NO_EVENT','event-invalid',None),
    ]:
        try:
            db.execute('INSERT INTO v37_ledger_raw_source(operation_id,process_ticket,custodian_nonce,domain_id,session_id,'
                'generation,source_epoch,source_cursor,raw_bytes,state,resolved_event_id,no_event_reason) '
                'VALUES(?,?,?,?,?,?,?,?,?,?,?,?)',
                ('bad-op','bad-ticket','bad-nonce',domain,'bad-session',generation,'bad-nonce',bad_cursor,
                 bad_raw,bad_state,bad_event,bad_reason))
        except sqlite3.IntegrityError:
            db.rollback()
        else:
            raise AssertionError('production raw-source DDL accepted an impossible state/cursor/frame')
    open_request = make_operation('K-SESSION','open',open_id,sid,
        {'generation':generation,'seatId':seat,'repositoryId':'gogokeSeatTestbed','worktreeId':worktree})
    open_request['receipt'] = {'requestId':open_id,'operation':'open','targetId':sid,'status':'APPLIED',
        'previousRevision':'0','revision':'1','result':{'threadId':vendor}}
    stop_fact='sha256:'+'1'*64
    db.execute('INSERT INTO gogoke_v37_h_process_episode(domain_id,request_id,session_id,generation,old_generation,'
        'raw_hex,previous_revision,result_revision,process_operation_id,instance_id,home_id,binding_id,seat_id,'
        'seat_incarnation,stop_request_id,phase,stop_fact_id) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)',
        (domain,open_id,sid,generation,None,open_request['rawFrame'].encode().hex(),0,1,process_op,
         instance,'home-original','binding-original',seat,'incarnation-1','stop-request','STOPPED',stop_fact))
    db.execute('INSERT INTO gogoke_v37_instances(instance_id,driver_id,home_ref,home_identity,program_digest,'
        'version,install_state,login_state,revision) VALUES(?,?,?,?,?,?,?,?,?)',
        (instance,'grok','provider-home-ref','provider-home-identity',
         'sha256:ab5d2a424f08281798acbdbb06076166fe000d7995ede94a673417b805210a25','1.0.41',
         'INSTALLED','LOGGED_IN',1))
    db.execute('INSERT INTO gogoke_v37_seats(domain_id,seat_id,incarnation,layer,parent_seat_id,kind,instance_id,'
        'state,generation,revision) VALUES(?,?,?,?,?,?,?,?,?,?)',
        (domain,seat,'incarnation-1','USER',None,'LONG',instance,'IDLE',19,1))
    db.execute('INSERT INTO gogoke_v37_seat_settings(domain_id,seat_id,template_id,settings_json) VALUES(?,?,?,?)',
        (domain,seat,'template-grok',json.dumps({'model':'grok-4.6','effort':'high','permissionTier':'READ_ONLY'})))

    reg_frame = {'jsonrpc':'2.0','id':3,'method':'session/new','params':{'cwd':'D:/test/tree',
        'mcpServers':[],'_meta':{'yoloMode':False,'x.ai/mcp/servers':[{'name':'gogoke_qcard','serverId':server}]}}}
    reg_bytes = (canonical(reg_frame)+'\n').encode()
    add_step(db,'thread-start',reg_bytes,'OBSERVED',1,nonce,'1')
    reg_ack = (json.dumps({'jsonrpc':'2.0','id':3,'result':{'sessionId':vendor}},separators=(',',':'))+'\n').encode()
    add_raw(db,1,reg_ack)

    journal = {'operations':[open_request], 'sessions':[{'id':sid,'threadId':vendor,'seatId':seat,
        'instanceId':instance,'worktreeId':worktree}]}
    case_turns=[]
    for idx,spec in enumerate(fixed):
        send_id = f'send-{idx+1}'
        prompt = 'Use the registered ask_user tool exactly once with this one question, and no other tools: ' + \
            json.dumps({key:spec[key] for key in ('header','question','options','recommendedIndex')},ensure_ascii=False,separators=(',',':')) + \
            '. After the User answers, repeat that answer exactly in your final response.'
        send_entry = make_operation('K-SESSION','send',send_id,sid,{'generation':generation,'body':prompt},'1')
        journal['operations'].append(send_entry)
        h_bytes = send_entry['rawFrame'].encode()
        h_digest = digest(h_bytes)
        prompt_step_id = 'acp-send-'+h_digest[:40]
        prompt_rpc_id = 'gogoke-acp-send-'+h_digest[:40]
        outer = 'outer-1' if idx == 0 else 9
        inner = 7 if idx == 0 else 'inner-2'
        args = {key:spec[key] for key in ('header','question','options','recommendedIndex')}
        question_frame = {'jsonrpc':'2.0','id':outer,'method':'x.ai/mcp/sdk_call','params':{
            'serverId':server,'message':{'jsonrpc':'2.0','id':inner,'method':'tools/call',
                'params':{'name':'ask_user','arguments':args}}}}
        question_raw = (json.dumps(question_frame,ensure_ascii=False,separators=(',',':'))+'\n').encode()
        question_cursor = str(2 + idx*3)
        add_raw(db,int(question_cursor),question_raw,'NO_EVENT','HOST_GROK_MCP_QUESTION_CARD')
        source = {'operationId':process_op,'sourceEpoch':nonce,'sourceCursor':question_cursor,
            'frameSha256':digest(question_raw)}
        binding = {'serverId':server,'registrationStepId':'thread-start',
            'registrationCommandSha256':digest(reg_bytes),'hostSessionId':sid,'openRequestId':open_id,
            'generation':generation,'processOperationId':process_op,'hostPromptRequestId':send_id,
            'hostPromptRequestSha256':h_digest,'promptStepId':prompt_step_id,'promptRpcId':prompt_rpc_id,
            'outerRpcId':outer,'innerRpcId':inner,'sourceOperationId':process_op,'sourceEpoch':nonce,
            'sourceCursor':question_cursor,'sourceFrameSha256':digest(question_raw),'bindingScope':'INITIAL_H_PROMPT'}
        payload = {'provider':'grok','cardOrigin':'host-mcp','bindingVersion':1,'recommendedIndex':spec['recommendedIndex'],
            'allowFreeAnswer':True,'questions':[{'id':'host0','header':spec['header'],'question':spec['question'],
                'options':spec['options']}],'binding':binding}
        card_basis = f'{domain}\n{sid}\n{process_op}\n{nonce}\n{question_cursor}'.encode()
        card_id = 'card'+digest(card_basis)
        composite = canonical([server,outer,inner]).encode()
        request_ref = 'gmpc_'+digest(composite)
        answer = spec['options'][spec['recommendedIndex']]['label'] if idx == 0 else free_answer
        answer_json = canonical({'answer':answer})
        answer_frame = {'jsonrpc':'2.0','id':outer,'result':{'jsonrpc':'2.0','id':inner,
            'result':{'content':[{'type':'text','text':answer_json}],'isError':False}}}
        answer_wire = canonical(answer_frame)
        db.execute('INSERT INTO gogoke_v37_qcard_native VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)',
            (domain,card_id,'2','ANSWERED',json.dumps(request_ref),vendor,request_ref,'',canonical(payload),
             'host0',spec['header'],spec['question'],'OPTIONS_OR_FREE',seat,send_id,generation,'WIRE',answer_wire))
        for option_index,option in enumerate(spec['options']):
            db.execute('INSERT INTO gogoke_v37_qcard_native_options VALUES(?,?,?,?,?,?)',
                (domain,card_id,f'option{option_index}',str(option_index),option['label'],option['description']))
        descriptor = {'operationId':process_op,'sourceEpoch':nonce,'sourceCursor':question_cursor,
            'frameSha256':digest(question_raw)}
        raise_id='raise'+digest(card_basis)
        db.execute('INSERT INTO gogoke_v37_qcard_native_operations VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)',
            (domain,raise_id,canonical(descriptor).encode().hex(),card_id,'0','1','RAISED',json.dumps(request_ref),
             seat,send_id,generation,'NONE','','',''))

        recover_id=f'recover-{idx+1}'
        recover_req=make_operation('K-QCARD','recover',recover_id,card_id,{},'1')
        recover_result={'state':'OPEN','seatId':seat,'turnId':send_id,'generation':generation,
            'requestRef':request_ref,'nativeQuestion':payload,'availableForAnswer':True}
        recover_receipt={'requestId':recover_id,'operation':'recover','targetId':card_id,'status':'APPLIED',
            'previousRevision':'1','revision':'1','result':recover_result}
        recover_entry={**recover_req,'receipt':recover_receipt}
        journal['operations'].append(recover_entry)
        db.execute('INSERT INTO v37_ledger_receipt VALUES(?,?,?,?,?)',
            ('K-QCARD',domain,recover_id,recover_req['rawFrame'].encode(),json.dumps(recover_receipt,separators=(',',':')).encode()))

        answer_id=f'answer-{idx+1}'
        answer_req=make_operation('K-QCARD','answer',answer_id,card_id,
            {'generation':generation,'answers':{'host0':[answer]}},'1')
        answer_receipt={'requestId':answer_id,'operation':'answer','targetId':card_id,'status':'APPLIED',
            'previousRevision':'1','revision':'2','result':{'state':'ANSWERED',
                'deliveryBasis':'NATIVE_EXACT_WRITE_RECEIPT','vendorConsumptionConfirmed':False}}
        answer_entry={**answer_req,'receipt':answer_receipt}
        journal['operations'].append(answer_entry)
        db.execute('INSERT INTO v37_ledger_receipt VALUES(?,?,?,?,?)',
            ('K-QCARD',domain,answer_id,answer_req['rawFrame'].encode(),json.dumps(answer_receipt,separators=(',',':')).encode()))
        step_id='qanswer'+digest(f'{domain}\n{sid}\n{answer_id}'.encode())
        db.execute('INSERT INTO gogoke_v37_qcard_native_operations VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)',
            (domain,answer_id,answer_req['rawFrame'].encode().hex(),card_id,'1','2','ANSWERED',json.dumps(request_ref),
             seat,send_id,generation,'WIRE',answer_wire,'','native-exact-receipt'))
        add_step(db,step_id,(answer_wire+'\n').encode(),'WRITTEN',0)

        terminal_cursor=str(int(question_cursor)+2)
        text='The requested answer is '+answer+'.'
        update={'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':text}}
        update_frame={'jsonrpc':'2.0','method':'session/update','params':{'sessionId':vendor,'update':update}}
        update_raw=(json.dumps(update_frame,ensure_ascii=False,separators=(',',':'))+'\n').encode()
        update_cursor=str(int(question_cursor)+1)
        event_id=f'event-{idx+1}'
        add_raw(db,update_cursor,update_raw,'RESOLVED',None,event_id)
        normalized={**update,'_meta':{'threadId':vendor,'rawSourceCursor':update_cursor}}
        db.execute('INSERT INTO v37_ledger_index(cursor,source_event_id,source_kind,source_cursor,source_epoch,'
            'domain_id,seat_id,session_id,tier,side_id,occurred_at,update_json) '
            'VALUES(?,?,?,?,?,?,?,?,?,?,?,?)',
            (idx+1,event_id,'v37',update_cursor,nonce,domain,seat,sid,'SESSION',None,
             '2026-10-18T00:00:00.000Z',canonical(normalized)))
        terminal_frame={'jsonrpc':'2.0','id':prompt_rpc_id,'result':{'stopReason':'end_turn'}}
        terminal_raw=(json.dumps(terminal_frame,separators=(',',':'))+'\n').encode()
        add_raw(db,terminal_cursor,terminal_raw)
        prompt_command=(canonical({'jsonrpc':'2.0','id':prompt_rpc_id,'method':'session/prompt',
            'params':{'sessionId':vendor,'prompt':[{'type':'text','text':prompt}]}})+'\n').encode()
        add_step(db,prompt_step_id,prompt_command,'OBSERVED',1,nonce,terminal_cursor)
        terminal_result={'createdTurn':True,'stopReason':'end_turn','sourceEpoch':nonce,'sourceCursor':terminal_cursor}
        terminal_receipt={'requestId':send_id,'operation':'send','targetId':sid,'status':'APPLIED','result':terminal_result}
        h_receipt=json.dumps(terminal_receipt,separators=(',',':')).encode()
        db.execute('INSERT INTO gogoke_v37_h_stdin_journal(domain_id,request_id,operation,ticket,process_operation_id,'
            'custodian_nonce,session_id,generation,request_hex,phase,receipt_hex,receipt_status,expected_revision,'
            'receipt_previous_revision,receipt_revision) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)',
            (domain,send_id,'send',ticket,process_op,nonce,sid,generation,send_entry['rawFrame'].encode().hex(),
             'RECEIPTED',h_receipt.hex(),'APPLIED','1','1','2'))
        turn={'ordinal':idx+1,'question':spec,'answerMode':spec['answerMode'],'state':'ORIGINAL_H_TERMINAL_OBSERVED',
            'sendRequestId':send_id,'sendRawFrame':send_entry['rawFrame'],'prompt':prompt,
            'beforeSendRawHighwater':str(int(question_cursor)-1),'cardId':card_id,'cardRevision':'1',
            'recoverRequestIds':[recover_id],'answerRequestId':answer_id,'answerWriteRequestId':answer_id,
            'answerRequestRawFrame':answer_req['rawFrame'],'privateAnswer':answer,
            'answerGeneratedAfterCardObservation':True,'terminalOutputPage':{'rawHighwater':terminal_cursor,
                'ledgerCursor':str(idx+1),'ledgerHighwater':str(idx+1)},'terminalReceipt':terminal_receipt,
            'card':{'cardId':card_id,'requestRef':request_ref,'turnId':send_id,'source':source,'binding':binding}}
        case_turns.append(turn)
    # C/A/H records above are the same rows the immutable reader consumes.
    physical_sources=db.execute('SELECT source_cursor,raw_bytes,state,resolved_event_id,no_event_reason '
        'FROM v37_ledger_raw_source ORDER BY CAST(source_cursor AS INTEGER)').fetchall()
    assert [row[0] for row in physical_sources] == [str(value) for value in range(1,8)]
    assert all(bytes(row[1]).endswith(b'\n') for row in physical_sources)
    assert all((row[2]=='NO_EVENT' and row[3] is None and row[4]) or
        (row[2]=='RESOLVED' and row[3] and row[4] is None) for row in physical_sources)
    db.commit()
    case={'driverId':'grok','caseMode':'grokHostMcpQuestion','fixedVersion':'1.0.41',
        'fixedSha256':'ab5d2a424f08281798acbdbb06076166fe000d7995ede94a673417b805210a25',
        'result':'DIRECT_H_RECEIPT_A_READBACK_REQUIRED','hostMcpQuestionObserved':True,
        'openRequestId':open_id,'seatId':seat,'instanceId':instance,'worktreeId':worktree,
        'seatSettings':{'model':'grok-4.6','effort':'high'},'sendRequestId':case_turns[-1]['sendRequestId'],
        'prompt':case_turns[-1]['prompt'],'hostMcpQuestion':{'state':'TWO_ORIGINAL_TURNS_ANSWERED_AND_TERMINAL',
            'turns':case_turns}}
    return db,case,journal,{'id':sid,'threadId':vendor,'seatId':seat,'instanceId':instance,'worktreeId':worktree}

def run_case(name, mutation=None, expect_fail=False):
    db,case,journal,session=fixture(name)
    try:
        if mutation:
            mutation(db,case,journal)
        if expect_fail:
            try: readback(db,domain,case,session,journal)
            except RuntimeError as error:
                print(name+': rejected: '+str(error))
                return
            raise AssertionError(name+' malformed evidence was accepted')
        result=readback(db,domain,case,session,journal)
        assert result['hostMcpQuestionObserved'] and result['allAnswersConsumedInOriginalTurns']
        assert [row['answerMode'] for row in result['turns']]==['RECOMMENDED_OPTION','FREE_TEXT']
        print(name+': positive branch read actual SQLite H/A/C rows')
    finally:
        db.close()

run_case('positive')
run_case('mixed-h',lambda db,case,journal: db.execute(
    'UPDATE gogoke_v37_h_stdin_journal SET process_operation_id=? WHERE request_id=?',
    ('other-process',case['hostMcpQuestion']['turns'][1]['sendRequestId'])),True)
run_case('source-mismatch',lambda db,case,journal: db.execute(
    'UPDATE gogoke_v37_qcard_native_operations SET request_hex=? WHERE card_id=? AND state="RAISED"',
    (canonical({'operationId':process_op,'sourceEpoch':nonce,'sourceCursor':'99',
       'frameSha256':case['hostMcpQuestion']['turns'][0]['card']['source']['frameSha256']}).encode().hex(),
     case['hostMcpQuestion']['turns'][0]['cardId'])),True)
def typed_id_mismatch(db,case,journal):
    raw=db.execute('SELECT raw_bytes FROM v37_ledger_raw_source WHERE source_cursor=? AND no_event_reason=?',
        ('2','HOST_GROK_MCP_QUESTION_CARD')).fetchone()[0]
    frame=json.loads(bytes(raw).decode('utf-8'))
    frame['id']=1
    mutated=(json.dumps(frame,ensure_ascii=False,separators=(',',':'))+'\n').encode()
    db.execute('UPDATE v37_ledger_raw_source SET raw_bytes=? WHERE source_cursor=? AND no_event_reason=?',
        (mutated,'2','HOST_GROK_MCP_QUESTION_CARD'))
run_case('typed-id-mismatch',typed_id_mismatch,True)
run_case('free-answer-leak',lambda db,case,journal: case['hostMcpQuestion']['turns'][1].update(
    {'prompt':case['hostMcpQuestion']['turns'][1]['prompt']+case['hostMcpQuestion']['turns'][1]['privateAnswer']}),True)
def writer_only(db,case,journal):
    answer=case['hostMcpQuestion']['turns'][1]['privateAnswer']
    update={'sessionUpdate':'agent_message_chunk','content':{'type':'text','text':'The answer was recorded.'}}
    frame={'jsonrpc':'2.0','method':'session/update','params':{'sessionId':vendor,'update':update}}
    raw=(json.dumps(frame,separators=(',',':'))+'\n').encode()
    db.execute('UPDATE v37_ledger_raw_source SET raw_bytes=? WHERE source_cursor=? AND state="RESOLVED"',(raw,'6'))
    normalized={**update,'_meta':{'threadId':vendor,'rawSourceCursor':'6'}}
    db.execute('UPDATE v37_ledger_index SET update_json=? WHERE source_cursor=?',(canonical(normalized),'6'))
run_case('writer-only-no-vendor-consumption',writer_only,True)

def table_from(relative,name):
    text=(repo/relative).read_text(encoding='utf-8')
    match=re.search(r'"(CREATE TABLE (?:IF NOT EXISTS )?'+re.escape(name)+r'\s*\([^\"]*\) STRICT)"',text,re.S)
    if not match: raise AssertionError('missing production table DDL '+name)
    return match.group(1)

def full_main_close_controls():
    # Exercise the actual readback CLI over the production DDL; close facts remain synthetic.
    db,case,source_journal,session=fixture('state')
    admission='apps/desktop/native-host/src/store/session_transport/admission.rs'
    binding_source='apps/desktop/native-host/src/store/session_transport/session_binding.rs'
    worktree_source='apps/desktop/native-host/src/store/worktree/mod.rs'
    for name in ['gogoke_v37_h_owner_binding','gogoke_v37_h_claim','gogoke_v37_h_operation',
                 'gogoke_v37_h_seat_binding']:
        db.execute(table_from(admission,name))
    db.execute(rust_const('apps/desktop/native-host/src/store/authority/process_custody.rs','SCHEMA'))
    for name in ['SCHEMA','PENDING_SCHEMA','EFFECTIVE_SEAT_SCHEMA']:
        db.execute(rust_const(binding_source,name))
    for name in ['gogoke_v37_worktree_sources','gogoke_v37_worktrees',
                 'gogoke_v37_worktree_operations','gogoke_v37_worktree_lifecycle']:
        db.execute(table_from(worktree_source,name))
    db.execute(schema_table('v37_ledger_meta'))
    db.execute('INSERT INTO v37_ledger_meta VALUES(1,?)',(nonce,))
    stop_fact='sha256:'+'1'*64
    db.execute('INSERT INTO gogoke_v37_h_owner_binding VALUES(?,?,?,?,?,?,?)',
        ('binding-original',instance,domain,'SESSION',sid,generation,'ACTIVE'))
    db.execute('INSERT INTO gogoke_v37_h_claim VALUES(?,?,?,?,?,?,?,?,?,?)',
        (domain,sid,instance,'home-original','binding-original',generation,'RELEASED',5,process_op,stop_fact))
    bind=(domain,sid,seat,'incarnation-1',19,instance)
    db.execute('INSERT INTO gogoke_v37_native_selection VALUES(?,?,?,?,?,?)',bind)
    db.execute('INSERT INTO gogoke_v37_session_binding_v2 VALUES(?,?,?,?,?,?,?)',(*bind,'NATIVE_V2'))
    fixed_sha=case['fixedSha256']
    db.execute('INSERT INTO gogoke_coordination_process_custody VALUES(?,?,?,?,?,?,?,?,?,?,?,?)',
        (process_op,ticket,nonce,'1234','100000','D:/synthetic/grok.exe','sha256:'+fixed_sha,
         instance,domain,generation,'STOPPED',stop_fact))
    physical=temp_root/'v37-worktrees'/'single'/'wtAudit'
    physical.mkdir(parents=True,exist_ok=True)
    identity=subprocess.check_output([sys.executable,str(reader_path),'--directory-identity',str(physical)],
        text=True).strip()
    db.execute('INSERT INTO gogoke_v37_worktree_sources VALUES(?,?,?,?,?,?,?,?,?,?)',
        ('gogokeSeatTestbed',str(temp_root/'source'),'source-identity',str(temp_root/'source/.git'),
         'common-identity','HTTPS','a'*40,'sha256:'+'b'*64,'2.0',1))
    db.execute('INSERT INTO gogoke_v37_worktrees VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)',
        (worktree,'wtAudit','gogokeSeatTestbed',domain,seat,'incarnation-1',18,1,'ReadOnly',instance,1,
         str(physical),identity,'b'*64,20,'pointer-id','common-id','a'*40,'REGISTERED',1))
    db.execute('INSERT INTO gogoke_v37_worktree_operations VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)',
        ('create-original','c'*64,'gogokeSeatTestbed',domain,seat,worktree,'wtAudit','incarnation-1',
         18,1,instance,'ReadOnly','REGISTERED',''))
    db.execute('INSERT INTO gogoke_v37_worktree_lifecycle(worktree_id,state,revision) VALUES(?,?,?)',
        (worktree,'REGISTERED',2))
    case.update({'sessionId':sid,'stopFact':stop_fact,
        'loggedInInstance':{'instanceId':instance,'driverId':'grok','version':'1.0.41','state':'LOGGED_IN'},
        'processIdentity':{'processId':'1234','creationTime100ns':'100000',
            'basis':'ACTUAL_PRODUCT_DESCENDANT_PROCESS_COMMAND_LINE_FILTERED','imageSha256':fixed_sha,
            'productRootPid':'123'}})
    case['modelEffortEvidence']={'basis':'REQUIRES_CAPTURED_ACTUAL_PROCESS_ARGV','argv':
        {'basis':'ACTUAL_PRODUCT_DESCENDANT_PROCESS_COMMAND_LINE_FILTERED','imageSha256':fixed_sha,
         'processId':'1234','modelArgCount':1,'effortArgCount':1,'commandLineSha256':'e'*64,
         'modelFlag':'--model','effortFlag':'--reasoning-effort','model':'grok-4.6','effort':'high',
         'productRootPid':'123'}}
    session['generation']=generation
    for op,rid,rev in [('stop','stop-request','3'),('admission-release','release-request','4'),
                       ('capability-probe','capability-request','3')]:
        entry=make_operation('K-SESSION',op,rid,sid,{'generation':generation,'seatId':seat},rev)
        result={'stopFact':stop_fact} if op=='stop' else {}
        if op=='capability-probe':
            result={'driverId':'grok','version':'1.0.41','binaryDigest':'sha256:'+fixed_sha,
                    'evidenceBasis':'NATIVE_ACP_INITIALIZE_DECLARATION'}
        entry['receipt']={'requestId':rid,'operation':op,'targetId':sid,'status':'APPLIED',
            'previousRevision':rev,'revision':str(int(rev)+1),'result':result}
        source_journal['operations'].append(entry)
        if op=='capability-probe':
            db.execute('INSERT INTO v37_ledger_receipt VALUES(?,?,?,?,?)',
                ('K-SESSION',domain,rid,entry['rawFrame'].encode(),json.dumps(entry['receipt']).encode()))
    for entry in source_journal['operations']:
        request=entry['request']
        if request['family']=='K-SESSION' and request['operation'] in ('open','stop','admission-release'):
            db.execute('INSERT INTO gogoke_v37_h_operation VALUES(?,?,?,?,?,?,?,?)',
                (domain,request['requestId'],entry['rawFrame'].encode().hex(),request['operation'],sid,'APPLIED',
                 int(request['expectedRevision']),int(entry['receipt']['revision'])))
    close_journal=close_entry_journal()
    close_journal.update({'schema':'gogoke.37.m2-provider-win11-e2e.v1',
        'state':'DIRECT_PROVIDER_H_RECEIPTS_A_READBACK_REQUIRED','acceptance':False,
        'observerDatabaseWrites':False,'hostOperationsWriteCandidateDatabase':True,'credentialReads':False,
        'caseId':'private-full-reader','domainId':domain,'repositoryId':'gogokeSeatTestbed',
        'currentEndpoint':{'pid':123},'cases':[case],'operations':source_journal['operations'],
        'sessions':[session]})
    assert db.execute('PRAGMA foreign_key_check').fetchall()==[]
    db.commit()
    db.close()
    journal_path=temp_root/'synthetic-journal.json'
    journal_path.write_text(json.dumps(close_journal,separators=(',',':')),encoding='utf-8')
    variants=[('coherent-positive',None),('missing-image','missing-image'),
        ('wrong-image-sha','wrong-image-sha'),('invalid-physical-fileid','invalid-physical-fileid'),
        ('missing-ancestry','missing-ancestry')]
    for label,mutation in variants:
        value=json.loads(journal_path.read_text(encoding='utf-8'))
        if mutation=='missing-image':
            def mutate_images(node):
                if isinstance(node,dict):
                    node.pop('image',None)
                    for child in node.values(): mutate_images(child)
                elif isinstance(node,list):
                    for child in node: mutate_images(child)
            mutate_images(value['causalCloseAttempts'])
        elif mutation=='wrong-image-sha':
            def mutate_hashes(node):
                if isinstance(node,dict):
                    if isinstance(node.get('image'),dict): node['image']['sha256']='0'*64
                    for child in node.values(): mutate_hashes(child)
                elif isinstance(node,list):
                    for child in node: mutate_hashes(child)
            mutate_hashes(value['causalCloseAttempts'])
        elif mutation=='invalid-physical-fileid':
            def mutate_fileids(node):
                if isinstance(node,dict):
                    if isinstance(node.get('image'),dict): node['image']['fileId']='invalid-file-id'
                    for child in node.values(): mutate_fileids(child)
                elif isinstance(node,list):
                    for child in node: mutate_fileids(child)
            mutate_fileids(value['causalCloseAttempts'])
        elif mutation=='missing-ancestry':
            value['causalCloseAttempts'][0]['ready']['nativeAncestorChain']=[]
        changed_journal=temp_root/(label+'-journal.json')
        changed_journal.write_text(json.dumps(value,separators=(',',':')),encoding='utf-8')
        out=temp_root/(label+'-result.json')
        run=subprocess.run([sys.executable,str(reader_path),str(temp_root),str(out),str(changed_journal)],
            capture_output=True,text=True,encoding='utf-8')
        if label=='coherent-positive':
            if run.returncode!=0: raise AssertionError('complete reader positive failed: '+run.stderr[-2500:])
            result=json.loads(out.read_text(encoding='utf-8'))
            assert result['directProviderEvidence'] is True and result['acceptance'] is False
            assert result['measurementPreservedDatabaseBytes'] is True
            assert result['sessions'][0]['causalNormalCloseEvidence']['mode']=='SPAWNED_NODE_CHILD'
        elif run.returncode==0 or out.exists():
            raise AssertionError(label+' malformed close was accepted by complete reader')
    print('full-main-coherent-positive: accepted complete production-DDL SQLite entry')
    print('full-main-missing-image: rejected')
    print('full-main-wrong-image-sha: rejected')
    print('full-main-invalid-physical-fileid: rejected')
    print('full-main-missing-ancestry: rejected')

full_main_close_controls()
`;
      const fixtureScript = path.join(dir, 'grok-host-mcp-fixture.py');
      fs.writeFileSync(fixtureScript, source, 'utf8');
      const run = spawnSync(python, [fixtureScript, reader, dir], {
        encoding: 'utf8', windowsHide: true, maxBuffer: 4 * 1024 * 1024,
      });
      assert.equal(run.status, 0, run.stderr || run.error?.message || run.stdout);
      for (const label of ['positive branch read actual SQLite', 'mixed-h: rejected', 'source-mismatch: rejected',
        'typed-id-mismatch: rejected', 'free-answer-leak: rejected', 'writer-only-no-vendor-consumption: rejected']) {
        assert.ok(run.stdout.includes(label), `missing fixture outcome: ${label}\n${run.stdout}`);
      }
      for (const label of ['full-main-coherent-positive: accepted complete production-DDL SQLite entry',
        'full-main-missing-image: rejected', 'full-main-wrong-image-sha: rejected',
        'full-main-invalid-physical-fileid: rejected', 'full-main-missing-ancestry: rejected']) {
        assert.ok(run.stdout.includes(label), `missing full-main close outcome: ${label}\n${run.stdout}`);
      }
      assert.ok(run.status === 0, 'actual provider normal-close reader entry accepts only shared causal Grok evidence');
    } finally {
      assert.equal(path.dirname(dir), tempRoot);
      fs.rmSync(dir, { recursive: true, force: true });
    }
  });
