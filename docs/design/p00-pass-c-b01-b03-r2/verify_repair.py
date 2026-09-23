#!/usr/bin/env python3
"""Passive evidence/source validation for B01-B03. Never imports product code.
Usage: python verify_repair.py --source SOURCE --candidate CANDIDATE --out RESULT
"""
import argparse, copy, csv, hashlib, json, re
from collections import Counter
from pathlib import Path
P='gogoke-codex-decoupling-p00-'
E='docs/design/p00-pass-c-b01-b03-r2'
def sha(b): return hashlib.sha256(b).hexdigest()
def blob(b): return hashlib.sha1(f'blob {len(b)}\0'.encode()+b).hexdigest()
def table(p):
 with p.open(newline='',encoding='utf-8') as f:return list(csv.DictReader(f,delimiter='\t'))
def source_methods(src):
 # Pinned-source lexical inventory, not a general Rust parser/callgraph proof.
 # Restrict arms to the actual five dispatcher modules; resolve Git constants.
 d=src/'apps/desktop/src-tauri/src/bin/gogoke_daemon/rpc'
 dispatch=(d/'dispatcher.rs').read_text()
 handlers=re.findall(r'\b([a-z_]+)::try_handle\b',dispatch)
 assert handlers==['daemon','workspace','codex','git','prompts']
 ctext=(src/'apps/desktop/src-tauri/src/shared/git_rpc.rs').read_text()
 pairs=re.findall(r'\bconst\s+(METHOD_\w+)\s*:\s*&str\s*=\s*"([^\"]+)"',ctext)
 assert len(dict(pairs))==len(pairs),'duplicate Git constant definition'
 consts=dict(pairs); rows=[]
 for handler in handlers:
  text=(d/(handler+'.rs')).read_text()
  assert text.count('match method {')==1
  for m in re.finditer(r'^\s*("[a-z_]+"|git_rpc::METHOD_[A-Z_]+)\s*=>',text,re.M):
   tok=m.group(1);kind='literal' if tok.startswith('"') else 'constant'
   name=tok.strip('"') if kind=='literal' else consts[tok.split('::')[-1]]
   rows.append({'method':name,'handler':handler,'kind':kind,'path':str((d/(handler+'.rs')).relative_to(src)),'line':text.count('\n',0,m.start(1))+1})
 assert len(rows)==len({r['method'] for r in rows}),'duplicate methods'
 assert len(rows)==104 and sum(r['kind']=='literal' for r in rows)==77
 return rows
STAGES={}
for wp,nums in {'WP01':[5,61],'WP02':[10,39,55],'WP03':[21,31,32,41,50,51,52,60,63],'WP04':[11,13,14,15,16,17,18,19,20,24,49,53,67],'WP05':[7,8,9,12,54,66],'WP06':[28,29,65],'WP07':[25,26,27],'WP08':[3,22,30,33,34,35],'WP09a':[6,58],'WP09b':[23,36,40],'WP10a':[56,57],'WP10b':[43,44,45],'WP11a':[4],'WP11b':[1,2,47,62,64],'WP12':[37,38,42,46,59],'WP13':[48,68]}.items():
 for n in nums:STAGES[f'T{n:02}.L']=wp
STAGES.update({'T68.P':'WP00','T68.D':'WP01','T55.H':'WP03','T60.M':'WP12'})
def validate(state,expected,tauri,retry,authorities):
 trace,caps,surfaces,meta=state;err=[]
 def ck(ok,k):
  if not ok:err.append(k)
 def idset(rows,key,want,k):
  v=[r[key] for r in rows];ck(len(set(v))==len(v),k+' duplicate');ck(set(v)==want,k+' set')
 rs={f'R{g}-{i:03}' for g,n in enumerate([55,44,50,55,47,52],1) for i in range(1,n+1)}
 cs={f'C{i:02}.{j}' for i in range(1,43) for j in (1,2)}
 idset(trace,'observation',rs,'observations');idset(caps,'capability_behavior',cs,'capabilities')
 idset(surfaces,'method',tauri|set(expected)|{'auth'},'methods')
 for rows in [trace,caps,surfaces]:
  for i,row in enumerate(rows):
   ck(all(isinstance(v,str) and v.strip() for v in row.values()),'blank:'+str(i))
   if 'capability_behavior' in row:ck(row['capability_behavior'] in cs,'Cref:'+str(i))
   for k in ['entry_family','entry_families','sink_family']:
    if k in row:ck(set(row[k].split(','))<=authorities,k+':'+str(i))
   for check in row.get('first_validation_checks',row.get('first_validation_check','')).split(','):ck(STAGES.get(check)==row['owning_wp'],'WP/check:'+str(i))
 ck(len({r['assertion'] for r in trace})==len(trace),'duplicate scenario')
 for k,v in [('runtime_verified',False),('code_entry_gate','CLOSED_PENDING_GPT_P00_REVIEW'),('global_orphan_sink_count',None),('p00_acceptance','NOT_ACCEPTED')]:ck(meta.get(k)==v,k)
 for row in surfaces:
  name=row['method'];want=expected.get(name,'transport-only' if name=='auth' else '-')
  ck(row['daemon_registry']==want,name+':handler:'+want)
  ck(row['tauri_registry']==('T' if name in tauri else '-'),name+':tauri')
  ck(row['disconnect_retry_allowlist']==('yes' if name in retry else 'no'),name+':retry')
  if name=='prompts_list':ck(row['desktop_route']=='local',name+':route')
 return sorted(set(err))
def main():
 ap=argparse.ArgumentParser();ap.add_argument('--source',type=Path,required=True);ap.add_argument('--candidate',type=Path,required=True);ap.add_argument('--out',type=Path,required=True);a=ap.parse_args()
 src=a.source.resolve();c=a.candidate.resolve();d=c/'docs/design';e=c/E
 sources=json.loads((e/'source-identities.json').read_text())
 for x in sources:
  b=(src/x['path']).read_bytes();assert sha(b)==x['sha256'] and blob(b)==x['git_blob'],x['path']
 methodrows=source_methods(src);expected={r['method']:r['handler']+':'+r['kind'] for r in methodrows}
 lib=(src/'apps/desktop/src-tauri/src/lib.rs').read_text(); reg=re.search(r'generate_handler!\[(.*?)\]',lib,re.S).group(1)
 tauri={m.split('::')[-1] for m in re.findall(r'\b(?:\w+::)*\w+\b',reg)};assert len(tauri)==129
 txt=(src/'apps/desktop/src-tauri/src/remote_backend/mod.rs').read_text();retry=set(re.findall(r'"([a-z_]+)"',re.search(r'fn can_retry_after_disconnect[\s\S]*?\n}',txt).group()))
 auth=(d/(P+'pass-a-authorities-v3.md')).read_text();ids=set(re.findall(r'\b(?:RT|EB|F|S|W|P|N|Q|O)\d{2}\b',auth))
 trace=sum([table(d/(P+f'pass-a-traceability-r{i}-v3.tsv')) for i in range(1,7)],[]);caps=table(d/(P+'pass-a-capabilities-v3.tsv'));surfaces=table(d/(P+'pass-a-surfaces-v3.tsv'));meta=json.loads((d/(P+'gpt-pass-a-v3.json')).read_text())
 state=(trace,caps,surfaces,meta);errs=validate(state,expected,tauri,retry,ids);assert not errs,errs
 # Demonstrate that the old two bad labels are detected, not used as negative-test baseline.
 old=copy.deepcopy(state)
 for r in old[2]:
  if r['method']=='local_usage_snapshot':r['daemon_registry']='codex:literal'
  if r['method']=='menu_set_accelerators':r['daemon_registry']='workspace:literal'
 olderrors=validate(old,expected,tauri,retry,ids);assert len(olderrors)==2,olderrors
 negatives=[]
 def mutate(n,fn,keep_totals=False,keep_partitions=False):
  x=copy.deepcopy(state);fn(x);es=validate(x,expected,tauri,retry,ids);assert es,n
  if keep_totals:assert len(x[2])==len(state[2])
  if keep_partitions:assert Counter(r['daemon_registry'] for r in x[2])==Counter(r['daemon_registry'] for r in state[2])
  negatives.append({'id':n,'status':'REJECTED','errors':es,'same_method_count':keep_totals,'same_handler_distribution':keep_partitions})
 mutate('N01',lambda x:x[0].pop(0));mutate('N02',lambda x:x[0][1].update(observation=x[0][0]['observation']))
 mutate('N03',lambda x:x[0][0].update(capability_behavior='C99.1'));mutate('N04',lambda x:x[0][0].update(entry_family='F99'));mutate('N05',lambda x:x[0][0].update(sink_family='S99'))
 mutate('N06',lambda x:x[0][0].update(owning_wp='WP13'));mutate('N07',lambda x:x[3].update(runtime_verified=True));mutate('N08',lambda x:x[3].update(code_entry_gate='OPEN'));mutate('N09',lambda x:x[3].update(global_orphan_sink_count=0))
 mutate('N10',lambda x:x[2].pop(0));mutate('N11',lambda x:next(r for r in x[2] if r['method']=='prompts_list').update(desktop_route='remote-when-enabled'))
 mutate('N12',lambda x:next(r for r in x[2] if r['method']=='send_user_message').update(disconnect_retry_allowlist='yes'));mutate('N13',lambda x:x[2].remove(next(r for r in x[2] if r['method']=='auth')))
 mutate('N14',lambda x:next(r for r in x[2] if r['daemon_registry']=='git:constant').update(daemon_registry='git:literal'))
 mutate('N15',lambda x:x[1].remove(next(r for r in x[1] if r['capability_behavior']=='C42.2')));mutate('N16',lambda x:x[0][1].update(assertion=x[0][0]['assertion']))
 mutate('B01-usage-wrong-handler',lambda x:next(r for r in x[2] if r['method']=='local_usage_snapshot').update(daemon_registry='codex:literal'),True)
 mutate('B01-menu-wrong-handler',lambda x:next(r for r in x[2] if r['method']=='menu_set_accelerators').update(daemon_registry='workspace:literal'),True)
 def swap(x):
  r=next(r for r in x[2] if r['method']=='local_usage_snapshot');s=next(r for r in x[2] if r['method']=='menu_set_accelerators');r['daemon_registry'],s['daemon_registry']=s['daemon_registry'],r['daemon_registry']
 mutate('B01-swap-keeps-all-totals-and-partitions',swap,True,True)
 contracts=json.loads((e/'reuse-contracts.json').read_text())
 assert contracts['lifecycle']['current_host_deadline_ms']==8000 and contracts['lifecycle']['current_grace_ms']==10000
 assert len(contracts['persistent_call_sites'])==37 and len(contracts['transitive_imports'])==40
 actual_imports=[];actual_writes=[]
 for source in [x['path'] for x in sources if x['path'].endswith('.ts')]:
  text=(src/source).read_text()
  for match in re.finditer(r'\b(?:import|export)\s+(?:type\s+)?[^;]*?\bfrom\s*["\'](\.[^"\']+)["\']',text):
   target=(Path(source).parent/match.group(1)).as_posix()
   actual_imports.append({'source':source,'import_line':text.count('\n',0,match.start())+1,'specifier':match.group(1),'target':target,'target_tracked':(src/target).is_file()})
  if source in [x['path'] for x in sources[:4]]:
   lines=text.splitlines()
   for line,code in enumerate(lines,1):
    if re.search(r'\b(?:appendJsonLineAtomic|atomicWriteJson|atomicWriteFile|mkdirSync)\s*\(',code):
     actual_writes.append({'path':source,'line':line,'call':code.strip(),'following_lines':'\n'.join(lines[line:line+7])})
 def graph_errors(value):
  out=[]
  if value['transitive_imports']!=actual_imports:out.append('import/source tuple mismatch')
  if value['persistent_call_sites']!=actual_writes:out.append('write/source tuple mismatch')
  return out
 assert not graph_errors(contracts),graph_errors(contracts)
 for x in actual_imports:assert x['target_tracked'],x
 graph_negatives=[]
 for field,key,bad_value in [('transitive_imports','target','wrong.ts'),('persistent_call_sites','call','undeclared_write()')]:
  bad=copy.deepcopy(contracts);bad[field][0][key]=bad_value
  errors=graph_errors(bad);assert len(errors)==1
  assert len(bad[field])==len(contracts[field])
  graph_negatives.append({'field':field,'status':'REJECTED','same_count':True,'errors':errors})
 assert {x[0] for x in contracts['roots_writers']}=={f'RT{i}' for i in range(26,33)}
 assert {x[0] for x in contracts['writer_contracts']}=={f'W{i}' for i in range(21,31)}
 assert {x[0] for x in contracts['permission_contracts']}=={f'Q{i}' for i in range(13,16)}
 # Adjacent K16/K17 fixed-source order/path/content checks, not Rust runtime tests.
 def body(text,name,next_name):
  start=text.index('fn '+name+'(');end=text.index('fn '+next_name+'(',start)
  return text[start:end]
 pt=(src/'apps/desktop/src-tauri/src/shared/prompts_core.rs').read_text()
 b=body(pt,'prompts_update_core','prompts_delete_core')
 assert b.index('fs::write(&next_path')<b.index('fs::remove_file(&target_path')
 at=(src/'apps/desktop/src-tauri/src/shared/agents_config_core.rs').read_text()
 b=body(at,'delete_agent_core','read_agent_config_toml_core')
 assert b.index('std::fs::remove_file(&target)')<b.index('persist_global_config_document')
 b=body(at,'read_agent_config_toml_core','write_agent_config_toml_core')
 assert 'resolve_managed_agent_config_relative_path' in b and 'resolve_safe_managed_abs_path_for_read' in b
 b=body(at,'write_agent_config_toml_core','resolve_codex_home')
 assert 'resolve_managed_agent_config_relative_path' in b and 'std::fs::write(path, content)' in b and 'parse_agent_config_document' not in b
 # Every declared object identity is checked when present; index excludes itself.
 checked=[]
 for obj in meta.get('published_evidence_objects',[]):
  data=(c/obj['path']).read_bytes();assert blob(data)==obj['git_blob'],obj['path']
  if 'sha256' in obj:assert sha(data)==obj['sha256'],obj['path']
  checked.append(obj['path'])
 result={'schema':'gogoke.p00.pass-c.repair-check.r2','mode':'AUTHOR_PASS_C_NOT_ISOLATED_REVIEW','status':'PASS_PASSIVE_TUPLE_IDENTITY_AND_CONTRACT_SCHEMA_ONLY','source_files_bound':len(sources),'method_count':len(methodrows),'literal_handler_counts':dict(sorted(Counter(r['handler'] for r in methodrows if r['kind']=='literal').items())),'git_constant_arms':sum(r['kind']=='constant' for r in methodrows),'corrected_candidate_errors':errs,'original_bad_labels_detected':olderrors,'observations':len(trace),'capability_behaviors':len(caps),'surfaces':len(surfaces),'negative_cases':negatives,'published_objects_checked':checked,'source_import_tuples_recomputed':len(actual_imports),'source_writer_tuples_recomputed':len(actual_writes),'contract_source_tuple_negative_cases':graph_negatives,'method_tuples':methodrows,'product_runtime_verified':False,'source_semantic_coverage_not_proven_by_schema':True}
 a.out.parent.mkdir(parents=True,exist_ok=True);a.out.write_text(json.dumps(result,indent=2,ensure_ascii=False)+'\n');print(json.dumps({'status':result['status'],'negatives':len(negatives),'old_errors':len(olderrors),'corrected_errors':len(errs)}))
if __name__=='__main__':main()
