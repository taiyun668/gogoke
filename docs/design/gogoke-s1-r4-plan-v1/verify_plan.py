#!/usr/bin/env python3
"""Offline plan verifier. This is NOT a product runtime, G0, or model-quality test."""
from __future__ import annotations
import argparse, copy, csv, hashlib, json, re, shutil, sys, tempfile
from pathlib import Path

FROZEN = {
 'inputs/LEGACY_TASKS.json':'29a32c6c4bd721fd19ed25df60670f173fbb32a8',
 'inputs/LEGACY_TEST_MATRIX.json':'f699418b93ebb156f7866a87c0594ee3cde45e95',
 'inputs/CAPABILITIES.tsv':'5f0140f31e91bad8ec8a61b9bfcdceb7a8cf92ac',
 'inputs/MASTER_PLAN.md':'a818790f2780e439096e1d246679fb9ed228d88a',
}
REQUIRED = ['PLAN.md','ARCHITECTURE.md','OPEN_ADAPTER.md','CONTEXT.md','DECISION.md',
 'DREAM_EVALUATION.md','RUNBOOK.md','CODEX_START.md','INPUTS.json','OBJECT_MODEL.json',
 'EXECUTION_PLAN.json','CHECKS.json','CAPABILITY_TASK_MAP.json','DECISION_FAMILIES.json',
 'QUALIFICATION_TARGETS.json','GATES_AUTHORIZATION.json','SOURCES.json','verify_plan.py']
SECTIONS = {'A':'ARCHITECTURE.md','O':'OPEN_ADAPTER.md','C':'CONTEXT.md','D':'DECISION.md',
 'E':'DREAM_EVALUATION.md','R':'RUNBOOK.md'}

def blob(data: bytes) -> str:
 return hashlib.sha1(b'blob '+str(len(data)).encode()+b'\0'+data).hexdigest()

def pairs(items):
 result={}
 for k,v in items:
  if k in result: raise ValueError('duplicate JSON key: '+k)
  result[k]=v
 return result

def load(path: Path):
 return json.loads(path.read_text(encoding='utf-8'),object_pairs_hook=pairs,
  parse_constant=lambda v: (_ for _ in ()).throw(ValueError('non-finite JSON: '+v)))

def overlaps(a: str,b: str) -> bool:
 return a==b or (a.endswith('/') and b.startswith(a)) or (b.endswith('/') and a.startswith(b))

def validate(root: Path, manifest: bool=True):
 errors=[]; checks=0
 def test(ok: bool, code: str, detail: str=''):
  nonlocal checks
  checks+=1
  if not ok: errors.append({'code':code,'detail':detail})
 for n in REQUIRED+list(FROZEN): test((root/n).is_file(),'MISSING_FILE',n)
 if errors:return {'status':'FAIL_PLAN','checks':checks,'errors':errors}
 try:
  for n,h in FROZEN.items():test(blob((root/n).read_bytes())==h,'FROZEN_INPUT',n)
  inp=load(root/'INPUTS.json'); old=load(root/'inputs/LEGACY_TASKS.json'); tm=load(root/'inputs/LEGACY_TEST_MATRIX.json')
  plan=load(root/'EXECUTION_PLAN.json'); matrix=load(root/'CHECKS.json'); caps=load(root/'CAPABILITY_TASK_MAP.json')
  targets=load(root/'QUALIFICATION_TARGETS.json'); gates=load(root/'GATES_AUTHORIZATION.json')
  obj=load(root/'OBJECT_MODEL.json'); families=load(root/'DECISION_FAMILIES.json')
  test(plan['status']=='DESIGN_FIXED_NOT_IMPLEMENTED','CLAIM_TIER')
  test(plan['source_head']==inp['source_head']=='88ef8e7dfbf5ba5aef58743dc45fa660f946276e','SOURCE_IDENTITY')
  test(inp['parent_plan']=='3b78e65361d0b75814c91d5a566d78a5e7fa013b','PARENT_IDENTITY')
  test(inp['donor']['commit']=='d6f291303ddc0c9a14f570266a4d9eff6d431593','DONOR_IDENTITY')
  test(plan['work_packages']==old['work_packages'],'WP_DEPENDENCIES_AND_DEADLINES')
  for f in inp['files']:
   test(f['path'] in FROZEN,'INPUT_UNEXPECTED',f['path'])
   test(f['git_blob']==FROZEN.get(f['path']),'INPUT_GIT_ANCHOR',f['path'])
   test(hashlib.sha256((root/f['path']).read_bytes()).hexdigest()==f['sha256'],'INPUT_SHA256',f['path'])
  # Parse actual frozen V3 requirement rows, not the prior summary.
  master={}; deadlines={}
  for line in (root/'inputs/MASTER_PLAN.md').read_text().splitlines():
   parts=[x.strip() for x in line.strip().split('|')[1:-1]]
   if len(parts)!=4:continue
   m=re.match(r'^(T\d\d)\s+(.+)$',parts[0])
   if not m or not parts[2].startswith('WP'):continue
   tid,req=m.groups();master[tid]={'requirement':req,'owner':parts[2],'capabilities':parts[1].split(',')}
   for k,wp in re.findall(r'(T\d\d\.[A-Z0-9]+)\s*→\s*(WP[0-9]+[ab]?)',parts[3]):
    test(k not in deadlines,'SOURCE_DUPLICATE_CHECK',k);deadlines[k]=wp
  test(len(master)==68,'MASTER_COUNT')
  test(deadlines==tm['checks'] and len(deadlines)==157,'DEADLINE_TUPLES')
  for tid,v in master.items():
   expected=tm['retained_master_rows'].get(tid,{})
   test(v['owner']==expected.get('maintenance_owner'),'MASTER_OWNER',tid)
   test(v['capabilities']==expected.get('capabilities'),'MASTER_CAPABILITIES',tid)
  tasks=plan['tasks']; byid={t['id']:t for t in tasks}; oldids=[t['id'] for t in old['tasks']]
  test(len(tasks)==len(byid)==32,'TASK_IDS')
  test(plan['legacy_task_ids']==oldids,'LEGACY_ID_ORDER')
  for t in old['tasks']:
   n=byid.get(t['id'])
   test(n is not None,'MISSING_LEGACY_TASK',t['id'])
   if n:
    test(n['wp']==t['wp'],'LEGACY_WP',t['id'])
    test(n['checks']==t['check_ids'],'LEGACY_CHECK_LINKS',t['id'])
  test(len(set(byid)-set(oldids))==13,'NEW_TASK_COUNT')
  ancestors={}
  def parents(k, stack):
   if k in ancestors:return ancestors[k]
   if k in stack: raise ValueError('DAG_CYCLE:'+k)
   if k not in byid:raise ValueError('UNKNOWN_DEPENDENCY:'+k)
   r=set()
   for d in byid[k]['depends_on']:r.add(d);r.update(parents(d,stack|{k}))
   ancestors[k]=r;return r
  for t in tasks:
   parents(t['id'],set())
   test(bool(t['acceptance']) and bool(t['write_scopes']) and bool(t['sections']),'TASK_INCOMPLETE',t['id'])
   for s in t['sections']:
    doc=SECTIONS.get(s[:1]);text=(root/doc).read_text() if doc else ''
    test(bool(re.search(r'^## '+re.escape(s)+r'\b',text,re.M)),'SECTION_JOIN',t['id']+':'+s)
   for p in t['write_scopes']:
    test(bool(p) and not p.startswith('/') and '..' not in Path(p).parts and '*' not in p and p not in ['.','apps/','third_party/','third_party/t3code/','tools/'],'OVERBROAD_PATH',t['id']+':'+p)
  for i,a in enumerate(tasks):
   for b in tasks[i+1:]:
    if a['id'] in ancestors[b['id']] or b['id'] in ancestors[a['id']]:continue
    for pa in a['write_scopes']:
     for pb in b['write_scopes']:
      test(not overlaps(pa,pb),'CONCURRENT_WRITER',a['id']+'/'+b['id']+':'+pa)
  due={r['id']:r for r in matrix['legacy_due']};new={r['id']:r for r in matrix['new_due']};allchecks={**due,**new}
  test(set(due)==set(tm['mandatory_s1_checks']) and len(due)==33,'LEGACY_DUE_SET')
  test(set(new)=={f'R4-{i:02d}' for i in range(1,27)},'NEW_CHECK_SET')
  for k,r in due.items():
   test(r['deadline']==tm['checks'][k],'DUE_DEADLINE',k)
   test(r['requirement']==master[k.split('.')[0]]['requirement'],'DUE_REQUIREMENT',k)
  allowed_groups={'qualification','sealing','codec','boundary','store','root','host','process','adapters','capabilities','delivery','continuation','events','policy','context','decision','evaluation','dream','vertical','upgrade'}
  runbook=(root/'RUNBOOK.md').read_text()
  for group in allowed_groups:test(group in runbook,'MISSING_COMMAND_GROUP',group)
  for k,r in new.items():
   n=int(k.split('-')[1]);due_gate='G2' if n<=5 else 'G3' if n<=15 or n in (23,25) else 'G5' if n==26 else 'G4'
   test(r['deadline']==due_gate,'NEW_CHECK_DEADLINE',k)
  for k,r in allchecks.items():
   test(set(r.get('groups',[r.get('group')])).issubset(allowed_groups),'CHECK_COMMAND_GROUP',k)
   expected=sorted(t['id'] for t in tasks if k in t['checks'])
   test(bool(expected) and sorted(r['owners'])==expected,'CHECK_TASK_JOIN',k)
   test(r['status']=='NOT_RUN','FALSE_RUNTIME_PASS',k)
   test(r.get('planned_test_tag')=='gogoke-s1-r4/'+k,'MISSING_TEST_TAG',k)
  for t in tasks:
   for k in t['checks']:test(k in allchecks,'UNMAPPED_TASK_CHECK',t['id']+':'+k)
  test(matrix['runner']['exists_at_plan'] is False and matrix['runner']['zero_test_policy']=='FAIL_INSTRUMENT','RUNNER_PROOF_BOUNDARY')
  original=list(csv.DictReader((root/'inputs/CAPABILITIES.tsv').open(encoding='utf-8'),delimiter='\t'))
  capmap={x['id']:x for x in caps['rows']}
  test(len(original)==len(capmap)==84,'CAPABILITY_COUNT')
  for r in original:
   row=capmap.get(r['capability_behavior'],{});first=r['first_validation_checks'].split(',')
   expected=sorted({t['id'] for t in tasks if set(t['checks'])&set(first)})
   test(row.get('owning_wp')==r['owning_wp'] and row.get('first_checks')==first and row.get('tasks')==expected,'CAPABILITY_JOIN',r['capability_behavior'])
  scenario=families['scenarios'];test(len(scenario)==18 and len({s['id'] for s in scenario})==18,'SCENARIO_SET')
  for s in scenario:test(s['live_status']=='NOT_QUALIFIED' and bool(s['ceiling']),'SCENARIO_AUTH',s['id'])
  test(targets['not_closed_architecture_enum'] is True,'CLOSED_DRIVER_ENUM')
  test([t['driver_id'] for t in targets['targets']]==['codex','claude-code','grok','opencode','antigravity','pi'],'QUALIFICATION_TARGET_SET')
  for t in targets['targets']:test(t['native_qualified'] is False,'FALSE_NATIVE_QUALIFICATION',t['driver_id'])
  test(targets['novel_control']['core_brand_switch_change_allowed'] is False and targets['novel_control']['schema_migration_allowed'] is False,'NOVEL_REQUIRES_CORE_REWRITE')
  for r in targets['cognitive_recipes']:test(r['driver_id']=='antigravity' and r['model_family']=='Gemini' and r['enabled'] is False,'GEMINI_NOT_RUNTIME')
  test(targets['jev']['kind']=='DecisionBackend' and not targets['jev']['enabled'] and not targets['jev']['weights_embedded'],'JEV_NOT_LOCAL_RUNTIME')
  for key in ['codex_dispatched','live_jev_enabled','live_generative_enabled','live_egress_allowed']:
   test(gates[key] is False,'UNAUTHORIZED_LIVE_FLAG',key)
  test(gates['live_budget']==0 and gates['test_only_profiles_cannot_activate_prod'] is True and gates['no_auto_account_rotation'] is True,'AUTH_BOUNDARY')
  test([g['id'] for g in gates['gates']]==['G0','G1','G2','G3','G4','G5','GN'],'GATE_SET')
  for g in gates['gates']:test(g['status'] in ['NOT_RUN','NOT_AUTHORIZED'],'FALSE_GATE_PASS',g['id'])
  test(len(obj['objects'])==16 and obj['status']=='NORMATIVE_DESIGN_NOT_EXISTING_API','OBJECT_DESIGN_TIER')
  test({'POSSIBLE','UNKNOWN','HOST_DELIVERED','NATIVE_ACKED'}.issubset(obj['states']['exposure']),'EXPOSURE_TRUTH')
  test('CENSORED' in obj['states']['outcome'],'MISSING_OUTCOME_CENSOR')
  test(plan['defaults']['worker_self_acceptance'] is False and plan['defaults']['worker_commit_push'] is False and plan['defaults']['author_plan_is_independent_acceptance'] is False,'REVIEW_BOUNDARY')
  if manifest and (root/'MANIFEST.json').exists():
   manifestdata=load(root/'MANIFEST.json')
   for n,h in manifestdata['sha256'].items():
    p=root/n
    safe=not Path(n).is_absolute() and '..' not in Path(n).parts and p.is_file() and not p.is_symlink()
    test(safe,'MANIFEST_PATH',n)
    if safe:test(hashlib.sha256(p.read_bytes()).hexdigest()==h,'MANIFEST_BYTES',n)
   actual={p.relative_to(root).as_posix() for p in root.rglob('*') if p.is_file() and '__pycache__' not in p.parts and p.name not in ['MANIFEST.json','VALIDATION.json']}
   test(actual==set(manifestdata['sha256']),'MANIFEST_COMPLETENESS')
 except (KeyError,ValueError,TypeError,OSError) as e:
  test(False,'INSTRUMENT_OR_PLAN_SCHEMA',str(e))
 return {'status':'PASS_PLAN_STRUCTURE_ONLY' if not errors else 'FAIL_PLAN','checks':checks,'errors':errors,
  'runtime_verified':False,'independent_architecture_review':False}

def self_test(root: Path):
 def mutation(file,fn):
  def apply(p):
   path=p/file; d=load(path);fn(d);path.write_text(json.dumps(d,ensure_ascii=False)+'\n')
  return apply
 variants=[
 ('missing_legacy_task',mutation('EXECUTION_PLAN.json',lambda d:d['tasks'].pop(0))),
 ('same_count_wrong_wp',mutation('EXECUTION_PLAN.json',lambda d:d['tasks'][0].update(wp='WP04'))),
 ('same_count_wrong_deadline',mutation('CHECKS.json',lambda d:d['legacy_due'][0].update(deadline='WP12'))),
 ('new_deadline_postponed',mutation('CHECKS.json',lambda d:d['new_due'][0].update(deadline='G5'))),
 ('same_count_wrong_requirement',mutation('CHECKS.json',lambda d:d['legacy_due'][0].update(requirement='ignore sealing'))),
 ('capability_first_check_changed',mutation('CAPABILITY_TASK_MAP.json',lambda d:d['rows'][0].update(first_checks=['T99.L']))),
 ('unknown_task_dependency',mutation('EXECUTION_PLAN.json',lambda d:d['tasks'][0]['depends_on'].append('NO_SUCH_TASK'))),
 ('dependency_cycle',mutation('EXECUTION_PLAN.json',lambda d:d['tasks'][19]['depends_on'].append('R4-V-FINAL'))),
 ('concurrent_writer',mutation('EXECUTION_PLAN.json',lambda d:d['tasks'][1]['write_scopes'].append(d['tasks'][2]['write_scopes'][0]))),
 ('broad_tree_edit',mutation('EXECUTION_PLAN.json',lambda d:d['tasks'][0]['write_scopes'].append('third_party/t3code/'))),
 ('nonexistent_section',mutation('EXECUTION_PLAN.json',lambda d:d['tasks'][0].update(sections=['A999']))),
 ('model_as_runtime',mutation('QUALIFICATION_TARGETS.json',lambda d:d['cognitive_recipes'][0].update(driver_id='Gemini'))),
 ('novel_requires_migration',mutation('QUALIFICATION_TARGETS.json',lambda d:d['novel_control'].update(schema_migration_allowed=True))),
 ('live_jev_unapproved',mutation('GATES_AUTHORIZATION.json',lambda d:d.update(live_jev_enabled=True))),
 ('live_egress_unapproved',mutation('GATES_AUTHORIZATION.json',lambda d:d.update(live_egress_allowed=True))),
 ('test_profile_to_production',mutation('GATES_AUTHORIZATION.json',lambda d:d.update(test_only_profiles_cannot_activate_prod=False))),
 ('pretend_product_pass',mutation('CHECKS.json',lambda d:d['new_due'][0].update(status='PASS'))),
 ('pretend_native_pass',mutation('QUALIFICATION_TARGETS.json',lambda d:d['targets'][0].update(native_qualified=True))),
 ('remove_final_gate',mutation('GATES_AUTHORIZATION.json',lambda d:d['gates'].pop(5))),
 ('self_acceptance',mutation('EXECUTION_PLAN.json',lambda d:d['defaults'].update(worker_self_acceptance=True))),
 ('erase_censored_state',mutation('OBJECT_MODEL.json',lambda d:d['states']['outcome'].remove('CENSORED'))),
 ]
 outcomes=[]
 for name,apply in variants:
  with tempfile.TemporaryDirectory(prefix='gogoke-plan-negative-') as tmp:
   p=Path(tmp)/'plan';shutil.copytree(root,p);apply(p)
   result=validate(p,manifest=False) # Must detect semantic corruption, not merely altered manifest.
   outcomes.append({'variant':name,'rejected':result['status']=='FAIL_PLAN','codes':sorted({e['code'] for e in result['errors']})})
 return {'total':len(outcomes),'rejected':sum(x['rejected'] for x in outcomes),'variants':outcomes,
  'meaning':'data/plan negative controls only, not production mutations'}

def main():
 ap=argparse.ArgumentParser();ap.add_argument('--root',type=Path,default=Path(__file__).parent);ap.add_argument('--self-test',action='store_true');ap.add_argument('--out',type=Path)
 args=ap.parse_args();result=validate(args.root.resolve())
 if args.self_test and result['status']=='PASS_PLAN_STRUCTURE_ONLY':
  result['negative_controls']=self_test(args.root.resolve())
  if result['negative_controls']['rejected']!=result['negative_controls']['total']:result['status']='FAIL_PLAN_NEGATIVES'
 text=json.dumps(result,ensure_ascii=False,indent=2)+'\n'
 if args.out:args.out.parent.mkdir(parents=True,exist_ok=True);args.out.write_text(text)
 print(text,end='');return 0 if result['status']=='PASS_PLAN_STRUCTURE_ONLY' else 2
if __name__=='__main__':raise SystemExit(main())
