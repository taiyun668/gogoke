/** C2-F01 passive AST/source-symbol check. Never executes product modules. */
import fs from 'node:fs';import path from 'node:path';import assert from 'node:assert/strict';
import {parseFile,anchor,ts} from './symbol_tools.mjs';
const [source,candidate,out]=process.argv.slice(2);
if(!source||!candidate||!out)throw Error('usage: node verify_symbols.mjs SOURCE CANDIDATE OUT');
const E='docs/design/p00-pass-c-b01-b03-r2',N='docs/design/p00-pass-c-c2-f01-r3';
const contract=JSON.parse(fs.readFileSync(path.join(candidate,E,'reuse-contracts.json')));
const binding=JSON.parse(fs.readFileSync(path.join(candidate,N,'symbol-bindings.json')));
const cache=new Map();const parsed=p=>{if(!cache.has(p))cache.set(p,parseFile(source,p));return cache.get(p);};
const semantic={
 'packages/seat-runtime/src/seat.ts':['input validators'],
 'packages/seat-runtime/src/seat-runtime.ts':['parsers'],
 'packages/seat-runtime/src/claude-seat.ts':['parsers'],
 'packages/seat-runtime/src/grok-acp-seat.ts':['parsers'],
 'packages/seat-runtime/src/index.ts':['barrel exports'],
 'packages/seat-runtime/src/demo.ts':['run','cleanup demonstration'],
 'packages/room/src/accounts.ts':['login','logout']
};
const requiredEdges=[
 ['ensureSeat','ensureSeatInner'],['ensureSeat','instanceStore.releaseAllFor'],
 ['ensureSeatInner','instanceStore.claim'],['ensureSeatInner','seat.start'],['ensureSeatInner','instanceStore.noteProcess'],['ensureSeatInner','seat.probeEffectiveHome'],
 ['InstanceStore.noteProcess','this.get'],['InstanceStore.noteProcess','processStartedAt'],['InstanceStore.noteProcess','this.save'],
 ['InstanceStore.claim','this.save'],['InstanceStore.releaseAllFor','this.save'],
 ['InstanceStore.save','mkdirSync'],['InstanceStore.save','writeFileSync'],['InstanceStore.save','renameSync'],['processStartedAt','execFileSync']
];
function validate(c,b){
 const errors=[];const ck=(ok,m)=>{if(!ok)errors.push(m);};
 const checkAnchor=(a,where)=>{try{const actual=anchor(parsed(a.path),a.symbol);ck(JSON.stringify(a)===JSON.stringify(actual),'anchor mismatch:'+where+':'+a.symbol);}catch(e){errors.push('unresolved:'+where+':'+String(e.message));}};
 ck(c.source_basis==='bc665a852833952b76d9508401193bedd2198436'&&b.source_basis===c.source_basis,'source identity');
 ck(b.rows.length===c.selection_matrix.length,'selection row count');
 ck(c.source_symbol_binding_manifest===N+'/symbol-bindings.json','binding pointer');
 ck(c.ownership_handoff_addendum===N+'/ownership-contract.json','ownership pointer');
 const seen=new Set();
 for(const r of b.rows){
  ck(!seen.has(r.index),'duplicate row:'+r.index);seen.add(r.index);
  const matrix=c.selection_matrix[r.index];if(!matrix){errors.push('missing matrix row');continue;}
  ck(r.path===matrix[0]&&r.operation_label===matrix[1]&&r.disposition===matrix[2],'matrix join:'+r.index);
  if(!r.path.endsWith('.ts')){ck(r.path==='crates/gogo-{core,store,protocol}/'&&r.classification==='NON_ADOPTED_DIRECTORY_SCOPE'&&r.tokens.length===0,'non-symbol scope');continue;}
  const labels=matrix[1].split('/');ck(JSON.stringify(r.tokens.map(x=>x.label))===JSON.stringify(labels),'token coverage:'+r.index);
  for(const t of r.tokens){
   const descriptor=semantic[r.path]?.includes(t.label)??false;
   ck(t.classification===(descriptor?'SEMANTIC_LABEL':'SOURCE_SYMBOL'),'label classification:'+t.label);
   if(descriptor)ck(typeof t.meaning==='string'&&t.meaning.length>15,'missing label explanation:'+t.label);
   else ck(t.source_symbols.length===1,'concrete binding count:'+t.label);
   for(const a of t.source_symbols){
    ck(a.path===r.path,'source file join:'+t.label);checkAnchor(a,t.label);
    if(!descriptor)ck(t.label===a.symbol||a.symbol===`InstanceStore.${t.label}`,'illegal symbol alias:'+t.label);
   }
   if(t.label==='login')ck(t.source_symbols[0]?.symbol==='AccountStore.openLoginTerminal','login behavior mapping');
   if(t.label==='logout')ck(t.source_symbols[0]?.symbol==='runLogout','logout behavior mapping');
  }
 }
 ck(JSON.stringify(b.caller_edges.map(e=>[e.caller.symbol,e.callee_expression]))===JSON.stringify(requiredEdges),'required caller-edge set/order');
 for(const e of b.caller_edges){
  checkAnchor(e.caller,e.id);if(e.target)checkAnchor(e.target,e.id+' target');
  try{
   const p=parsed(e.caller.path),d=p.declarations.find(d=>d.symbol===e.caller.symbol),calls=[];
   function walk(n){if(ts.isCallExpression(n)&&n.expression.getText(p.sf)===e.callee_expression)calls.push({line:p.sf.getLineAndCharacterOfPosition(n.getStart(p.sf)).line+1,expression:n.expression.getText(p.sf)});ts.forEachChild(n,walk);}walk(d.node);
   ck(calls.length>0&&JSON.stringify(calls)===JSON.stringify(e.call_sites),'caller join:'+e.id);
   if(e.target)ck(e.callee_expression.split('.').at(-1)===e.target.symbol.split('.').at(-1),'callee identity:'+e.id);
   else ck(['seat.start','seat.probeEffectiveHome','mkdirSync','writeFileSync','renameSync','execFileSync'].includes(e.callee_expression),'unbound target:'+e.id);
  }catch(err){errors.push('caller parse:'+e.id+':'+err.message);}
 }
 return errors;
}
const baseline=validate(contract,binding),negatives=[];
if(!baseline.length){
 const probe=(id,edit)=>{const c=structuredClone(contract),b=structuredClone(binding);edit(c,b);const errors=validate(c,b);assert(errors.length>0,id+' was not rejected');assert.equal(c.transitive_imports.length,40);assert.equal(c.persistent_call_sites.length,37);negatives.push({id,status:'REJECTED',errors,import_count_unchanged:true,writer_count_unchanged:true});};
 const row=(b,s)=>b.rows.find(x=>x.path.endsWith(s)),tok=(r,s)=>r.tokens.find(x=>x.label===s);
 probe('restore-confirmOwner',(c)=>{const r=c.selection_matrix.find(x=>x[0].endsWith('/instances.ts'));r[1]=r[1].replace('noteProcess','confirmOwner');});
 probe('restore-startSeat',(c)=>{const r=c.selection_matrix.find(x=>x[0].endsWith('/server.ts'));r[1]=r[1].replace('ensureSeat/ensureSeatInner','startSeat');});
 probe('nonexistent-bound-method',(c,b)=>{tok(row(b,'/instances.ts'),'noteProcess').source_symbols[0].symbol='InstanceStore.confirmOwner';});
 probe('synthetic-missing-symbol',(c,b)=>{tok(row(b,'/server.ts'),'ensureSeat').source_symbols[0].symbol='ensureSeatR3Missing';});
 probe('wrong-class',(c,b)=>{tok(row(b,'/instances.ts'),'noteProcess').source_symbols[0].symbol='AccountStore.noteProcess';});
 probe('wrong-source-file',(c,b)=>{tok(row(b,'/instances.ts'),'noteProcess').source_symbols[0].path='packages/room/src/server.ts';});
 probe('wrong-line',(c,b)=>{tok(row(b,'/instances.ts'),'noteProcess').source_symbols[0].line++;});
 probe('wrong-blob',(c,b)=>{tok(row(b,'/instances.ts'),'noteProcess').source_symbols[0].git_blob='0'.repeat(40);});
 probe('wrong-declaration-kind',(c,b)=>{tok(row(b,'/instances.ts'),'noteProcess').source_symbols[0].kind='FunctionDeclaration';});
 probe('deleted-binding-row',(c,b)=>{b.rows.pop();});
 probe('deleted-token',(c,b)=>{row(b,'/instances.ts').tokens.pop();});
 probe('reclassify-missing-as-prose',(c,b)=>{const r=row(b,'/instances.ts'),t=tok(r,'noteProcess');t.label='confirmOwner';t.classification='SEMANTIC_LABEL';t.meaning='An arbitrary prose excuse must not bypass the symbol check';t.source_symbols=[];r.operation_label=r.operation_label.replace('noteProcess','confirmOwner');c.selection_matrix[r.index][1]=r.operation_label;});
 probe('delete-release-caller',(c,b)=>{b.caller_edges.splice(1,1);});
 probe('nonexistent-call-expression',(c,b)=>{b.caller_edges[0].callee_expression='startSeat';});
}
const result={schema:'gogoke.p00.c2-f01.symbol-validation.r3',role:'AUTHOR_SIDE_NOT_INDEPENDENT_REVIEW',status:baseline.length?'CHANGES_REQUIRED':'PASS_PINNED_SYMBOL_JOINS',parser:ts.version,source_files_parsed:cache.size,parse_errors:0,selection_rows:binding.rows.length,concrete_tokens:binding.rows.flatMap(x=>x.tokens).filter(x=>x.classification==='SOURCE_SYMBOL').length,semantic_labels:binding.rows.flatMap(x=>x.tokens).filter(x=>x.classification==='SEMANTIC_LABEL').length,caller_edges:binding.caller_edges.length,baseline_errors:baseline,negative_cases:negatives,scope:'AST declarations, exact source ranges/blob/hashes and lexical call sites, not whole-program binding, runtime execution or ownership proof',runtime_verified:false};
fs.writeFileSync(out,JSON.stringify(result)+'\n');console.log(JSON.stringify({status:result.status,errors:baseline,rows:result.selection_rows,concrete_tokens:result.concrete_tokens,negative_cases:negatives.length}));process.exitCode=baseline.length?2:0;
