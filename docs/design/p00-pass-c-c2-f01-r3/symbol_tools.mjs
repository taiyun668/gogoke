/** Pinned TypeScript AST support. Reads source; does not import or execute product modules. */
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {createRequire} from 'node:module';
const require=createRequire(import.meta.url);
const candidates=[process.env.P00_TYPESCRIPT_PATH,'/opt/nvm/versions/node/v22.16.0/lib/node_modules/typescript/lib/typescript.js'];
let ts;
for(const p of candidates.filter(Boolean)){try{ts=require(p);break;}catch{}}
if(!ts){try{ts=require('typescript');}catch{throw Error('TypeScript 5.8.3 required; set P00_TYPESCRIPT_PATH to its local typescript.js. No automatic install.');}}
if(ts.version!=='5.8.3')throw Error('Pinned parser version mismatch: '+ts.version);
export {ts};
export const sha=b=>crypto.createHash('sha256').update(b).digest('hex');
export const blob=b=>crypto.createHash('sha1').update(Buffer.concat([Buffer.from(`blob ${b.length}\0`),b])).digest('hex');
export function parseFile(root,p){
 if(path.isAbsolute(p)||p.split(/[\\/]/).includes('..'))throw Error('Non-local source path: '+p);
 const bytes=fs.readFileSync(path.join(root,p)),text=bytes.toString('utf8');
 const sf=ts.createSourceFile(p,text,ts.ScriptTarget.Latest,true,ts.ScriptKind.TS);
 if(sf.parseDiagnostics.length)throw Error('TypeScript parse errors: '+p);
 const declarations=[];
 function walk(n){
  const top=n.parent===sf||(ts.isVariableDeclaration(n)&&n.parent.parent.parent===sf);
  const member=ts.isMethodDeclaration(n)&&ts.isClassDeclaration(n.parent);
  if(n.name&&((top&&(ts.isFunctionDeclaration(n)||ts.isClassDeclaration(n)||ts.isVariableDeclaration(n)||ts.isInterfaceDeclaration(n)||ts.isTypeAliasDeclaration(n)))||member)){
   let symbol=n.name.getText(sf);
   if(member)symbol=n.parent.name.text+'.'+symbol;
   declarations.push({symbol,kind:ts.SyntaxKind[n.kind],line:sf.getLineAndCharacterOfPosition(n.getStart(sf)).line+1,end_line:sf.getLineAndCharacterOfPosition(n.end).line+1,declaration_sha256:sha(Buffer.from(n.getText(sf))),node:n});
  }
  ts.forEachChild(n,walk);
 }
 walk(sf);
 return {path:p,bytes,text,sf,declarations,git_blob:blob(bytes),sha256:sha(bytes)};
}
export function anchor(parsed,symbol){
 const matches=parsed.declarations.filter(x=>x.symbol===symbol);
 if(matches.length!==1)throw Error(`Expected one declared symbol: ${parsed.path}:${symbol}; found ${matches.length}`);
 const {node,...decl}=matches[0];
 return {path:parsed.path,...decl,git_blob:parsed.git_blob,sha256:parsed.sha256};
}
export function body(parsed,symbol){
 anchor(parsed,symbol);const n=parsed.declarations.find(x=>x.symbol===symbol).node;
 if(!n.body)throw Error('No executable body: '+symbol);
 return n.body.getText(parsed.sf);
}
