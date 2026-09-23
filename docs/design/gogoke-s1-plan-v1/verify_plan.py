#!/usr/bin/env python3
"""Read-only S1 plan checks. No product build, test, network, or native execution.
Usage: python verify_plan.py --source FIXED_CHECKOUT --plan THIS_DIR --out RESULT
"""
from __future__ import annotations
import argparse, copy, hashlib, json, re
from pathlib import Path, PurePosixPath

BASIS = 'bc665a852833952b76d9508401193bedd2198436'
PARENT = 'a06619719bbac73a892d9e7b27b911cac77d8aa8'
DEPS = {'WP10a':['WP00'],'WP01':['WP00'],'WP02':['WP01'],
        'WP03':['WP02','WP10a'],'WP09a':['WP03'],'WP04':['WP03','WP09a']}
ROLES = {'Sol/high','Luna/high','construction Luna/max'}
REQUIRED_DOCS = ['PLAN.md','CONTRACTS.md','CODEX_HANDOFF.md']

def blob(b: bytes) -> str:
    return hashlib.sha1(b'blob '+str(len(b)).encode()+b'\0'+b).hexdigest()

def read_master(source: Path) -> tuple[dict, dict, str]:
    p = source/'docs/design/gogoke-codex-decoupling-plan-v3.md'
    raw = p.read_bytes(); lines = raw.decode().splitlines()
    start = next(i for i,x in enumerate(lines,1) if x.startswith('## 9. 测试'))
    end = next(i for i,x in enumerate(lines,1) if x.startswith('### 9.1 '))
    rows, checks = {}, {}
    for n, line in enumerate(lines,1):
        if not start <= n < end or not re.match(r'^\| T\d\d ', line):
            continue
        cells = [x.strip() for x in line.strip('|').split('|')]
        tid = re.match(r'T\d+',cells[0]).group()
        pairs = re.findall(r'(T\d+\.[A-Z0-9]+)\s*→\s*(WP\d+[ab]?)',cells[3])
        rows[tid] = {'source_line':n,'maintenance_owner':cells[2],
                     'capabilities':cells[1].split(','),'checks':[x[0] for x in pairs]}
        for cid, wp in pairs:
            if cid in checks: raise ValueError('duplicate master check '+cid)
            checks[cid] = wp
    return rows, checks, blob(raw)

def check(source: Path, plan: dict, matrix: dict, manifest: dict) -> tuple[list[str],int]:
    errors: list[str] = []; count = 0
    def expect(value: bool, label: str) -> None:
        nonlocal count
        count += 1
        if not value: errors.append(label)
    expect(plan.get('source_basis') == BASIS, 'source_basis')
    expect(plan.get('planning_base') == PARENT, 'planning_base')
    expect(plan.get('authorized_production_work_packages') == [], 'authorization')
    expect(plan.get('authorization_status') == 'PENDING_OWNER', 'owner_pending')
    expect(plan.get('runtime_verified') is False, 'runtime_not_executed')
    expect(plan.get('independent_plan_review_completed') is False, 'no_self_acceptance')
    expect(plan.get('p00') == 'NOT_ACCEPTED', 'p00_closed')
    expect(plan.get('code_entry_gate') == 'CLOSED_PENDING_GPT_P00_REVIEW', 'gate_closed')
    expect(plan.get('design_platform') == 'GPT' and plan.get('execution_platform') == 'Codex', 'platforms')
    wps = plan.get('work_packages',[])
    expect(len(wps)==6 and {x['id'] for x in wps}==set(DEPS), 'wp_set')
    for w in wps: expect(w.get('depends_on')==DEPS.get(w['id']), 'wp_dependency:'+w['id'])
    original, deadlines, master_blob = read_master(source)
    expect(len(original)==68 and len(deadlines)==157, 'master_count')
    expect(matrix.get('retained_master_rows')==original, 'master_rows')
    expect(matrix.get('source_plan',{}).get('ref')==BASIS, 'master_ref')
    expect(matrix.get('source_plan',{}).get('git_blob')==master_blob, 'master_blob')
    actual = [{'id':cid,'test_id':cid.split('.')[0],'deadline_wp':wp,'status':matrix.get('check_status'),'in_s1':wp in DEPS} for cid,wp in matrix.get('checks',{}).items()]
    expect(len(actual)==len(deadlines) and len({x['id'] for x in actual})==len(actual), 'check_uniqueness')
    expect({x['id']:x['deadline_wp'] for x in actual}==deadlines, 'check_deadlines')
    for c in actual:
        expect(c.get('test_id')==c['id'].split('.')[0], 'check_parent:'+c['id'])
        expect(c.get('status')=='PLANNED_NOT_EXECUTED', 'check_status:'+c['id'])
        expect(c.get('in_s1')==(c['deadline_wp'] in DEPS), 'check_phase:'+c['id'])
    due = {k:v for k,v in deadlines.items() if v in DEPS}
    expect(len(due)==33 and set(matrix.get('mandatory_s1_checks',[]))==set(due), 's1_due_set')
    expected_by_wp = {w:[c for c,d in deadlines.items() if d==w] for w in DEPS}
    expect(matrix.get('by_wp')==expected_by_wp, 'deadline_groups')
    for w in wps: expect(w.get('mandatory_checks')==expected_by_wp[w['id']], 'wp_checks:'+w['id'])
    tasks = plan.get('tasks',[]); ids = [t['id'] for t in tasks]
    expect(len(tasks)==19 and len(set(ids))==19, 'task_set')
    defaults = plan.get('task_defaults',{})
    expect(defaults.get('scope_state')=='PLANNED_NOT_AUTHORIZED', 'task_not_authorized')
    expect(defaults.get('commit_push')=='CONTROLLER_ONLY', 'worker_no_push')
    expect(defaults.get('reviewer')=='fresh Sol/high' and defaults.get('risk_review')=='fresh Astra/xhigh', 'review_roles')
    expect('self_acceptance' in defaults.get('forbidden',[]) and 'test_expectation_weakening' in defaults.get('forbidden',[]), 'task_limits')
    paths, covered = [], set()
    shared = plan.get('controller_shared_write_scopes',[])
    for t in tasks:
        expect(t.get('executor') in ROLES, 'executor:'+t['id'])
        expect(t['wp'] in DEPS, 'task_wp:'+t['id'])
        expect(t.get('input_products')==['P'+x[2:] for x in DEPS[t['wp']]], 'products:'+t['id'])
        expect(bool(t.get('acceptance')) and bool(t.get('contract_sections')), 'task_contract:'+t['id'])
        expect(all(re.fullmatch(r'C(0[1-9]|1[012])',x) for x in t['contract_sections']), 'contract_ids:'+t['id'])
        expect(all(x in ids and x!=t['id'] for x in t['work_depends_on']), 'task_deps:'+t['id'])
        for p in t['read_source_paths']:
            expect(not PurePosixPath(p).is_absolute() and '..' not in PurePosixPath(p).parts and (source/p).is_file(), 'read_path:'+p)
        for p in t['write_scopes']:
            expect(p.startswith(('apps/desktop/','tools/gogoke-s1/')) and '..' not in PurePosixPath(p).parts, 'write_scope:'+p)
            expect(p not in shared, 'hotspot:'+p)
            paths.append((p,t['id']))
        for c in t['check_ids']:
            expect(due.get(c)==t['wp'], 'task_check_deadline:'+t['id']+':'+c); covered.add(c)
    expect(covered==set(due), 'task_check_coverage')
    for i,(a,ta) in enumerate(paths):
        for b,tb in paths[i+1:]:
            if ta!=tb:
                expect(not (a==b or (a.endswith('/') and b.startswith(a)) or (b.endswith('/') and a.startswith(b))), 'write_conflict:'+ta+':'+tb)
    visiting, done = set(),set(); graph={t['id']:t['work_depends_on'] for t in tasks}
    def visit(n: str) -> None:
        if n in visiting: raise ValueError('cycle')
        if n in done or n not in graph: return
        visiting.add(n)
        for d in graph[n]: visit(d)
        visiting.remove(n); done.add(n)
    try:
        for n in graph: visit(n)
        expect(True,'task_dag')
    except ValueError: expect(False,'task_dag')
    expect(manifest.get('source_basis')==BASIS, 'manifest_basis')
    src = manifest.get('files',[])
    expect(len(src)==56 and len({x['path'] for x in src})==56, 'source_manifest_set')
    for f in src:
        p = source/f['path']
        expect(p.is_file() and blob(p.read_bytes())==f['git_blob'], 'source_blob:'+f['path'])
    declared = {x['path'] for x in src}
    expect(all(p in declared for t in tasks for p in t['read_source_paths']), 'read_sources_bound')
    pkg=json.loads((source/'apps/desktop/package.json').read_text())
    for c in matrix['command_registry']:
        if c['exists_at_source'] and c['command'].startswith('npm'):
            script = 'test' if c['command']=='npm test' else c['command'].split()[-1]
            expect(script in pkg['scripts'], 'existing_command:'+c['id'])
        if not c['exists_at_source']:
            expect(c.get('planned_owner') in ids, 'future_command_owner:'+c['id'])
    expect(plan['capacity_policy'].get('actual_slots')=='UNKNOWN_UNTIL_CODEX_INTAKE', 'no_invented_slots')
    expect(plan['capacity_policy'].get('extra_sessions_to_bypass_limit') is False and plan['capacity_policy'].get('grok_required') is False, 'capacity_limits')
    expect(all(plan['invariants'].values()), 'plan_invariants')
    return errors,count

def main() -> int:
    ap=argparse.ArgumentParser(); ap.add_argument('--source',type=Path,required=True)
    ap.add_argument('--plan',type=Path,required=True); ap.add_argument('--out',type=Path,required=True)
    args=ap.parse_args()
    p,m,s=[json.loads((args.plan/n).read_text()) for n in ('TASKS.json','TEST_MATRIX.json','SOURCE_MANIFEST.json')]
    errors, count=check(args.source,p,m,s)
    for n in REQUIRED_DOCS:
        count+=1
        if not (args.plan/n).is_file() or not (args.plan/n).read_text().strip(): errors.append('document:'+n)
    def mutate(which: str):
        a,b,c=copy.deepcopy((p,m,s))
        if which=='drop_wp': a['work_packages'].pop()
        elif which=='wp_dependency': a['work_packages'][3]['depends_on']=[]
        elif which=='cycle': a['tasks'][0]['work_depends_on']=[a['tasks'][1]['id']]; a['tasks'][1]['work_depends_on']=[a['tasks'][0]['id']]
        elif which=='duplicate_task': a['tasks'][-1]['id']=a['tasks'][0]['id']
        elif which=='drop_check': b['checks'].pop(next(iter(b['checks'])))
        elif which=='postpone_check': b['checks']['T55.H']='WP11b'
        elif which=='runtime_pass': a['runtime_verified']=True
        elif which=='auto_authorize': a['authorized_production_work_packages']=list(DEPS)
        elif which=='grok_worker': a['tasks'][0]['executor']='Grok'
        elif which=='self_review': a['task_defaults']['reviewer']='IMPLEMENTER'
        elif which=='worker_push': a['task_defaults']['commit_push']='WORKER'
        elif which=='shared_hotspot': a['tasks'][0]['write_scopes'].append(a['controller_shared_write_scopes'][0])
        elif which=='overlap': a['tasks'][1]['write_scopes'].append(a['tasks'][0]['write_scopes'][0])
        elif which=='root_kernel_write': a['tasks'][0]['write_scopes'].append('crates/core/src/lib.rs')
        elif which=='missing_source': a['tasks'][0]['read_source_paths'].append('not-a-real-source.ts')
        elif which=='wrong_blob': c['files'][0]['git_blob']='0'*40
        elif which=='fake_command': b['command_registry'][0]['command']='npm run nonexistent-s1-task'
        elif which=='omit_task_check': a['tasks'][5]['check_ids']=[]
        else: raise ValueError(which)
        return check(args.source,a,b,c)[0]
    variants=['drop_wp','wp_dependency','cycle','duplicate_task','drop_check','postpone_check','runtime_pass','auto_authorize','grok_worker','self_review','worker_push','shared_hotspot','overlap','root_kernel_write','missing_source','wrong_blob','fake_command','omit_task_check']
    results=[{'case':x,'rejected':bool(e:=mutate(x)),'errors':e} for x in variants]
    ok=not errors and all(x['rejected'] for x in results)
    result={'schema':'gogoke.s1.plan-validation.v1','status':'PASS_PLAN_STRUCTURE_ONLY' if ok else 'FAIL',
      'source_basis':BASIS,'planning_base':PARENT,'baseline_checks':count,'baseline_errors':errors,
      'tasks':len(p['tasks']),'work_packages':len(p['work_packages']),'retained_tests':len(m['retained_master_rows']),
      'retained_check_deadlines':len(m['checks']),'due_s1':len(m['mandatory_s1_checks']),
      'negative_cases':results,'runtime_verified':False,'semantic_plan_independently_approved':False,
      'product_tests_executed':False,'source_files_written':False,
      'limitations':['Structure checks are not proof of semantic design completeness or product correctness.','Source bytes checked against published blob list; whole checkout/tree verification is a separate recorded identity check.','No test command in TEST_MATRIX was executed by this validator.']}
    args.out.parent.mkdir(parents=True,exist_ok=True); args.out.write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n')
    print(json.dumps({k:result[k] for k in ('status','baseline_checks','baseline_errors','tasks','retained_tests','retained_check_deadlines','due_s1')}))
    return 0 if ok else 1
if __name__=='__main__': raise SystemExit(main())
