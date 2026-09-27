import test from 'node:test';
import assert from 'node:assert/strict';
import {mkdtempSync,writeFileSync,statSync,mkdirSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import path from 'node:path';
import {copyChanged} from './artifacts.mjs';
import {revisions} from './inputs.mjs';
test('worker rejects placeholders, preserves identical destination, replaces changed content',()=>{
 const dir=mkdtempSync(path.join(tmpdir(),'omniget worker '));
 try{const src=path.join(dir,'source'),dest=path.join(dir,'dest');writeFileSync(src,'');assert.throws(()=>copyChanged(src,dest),/EMPTY_ARTIFACT/);
 writeFileSync(src,'one');assert.equal(copyChanged(src,dest).copied,true);const time=statSync(dest).mtimeMs;
 assert.equal(copyChanged(src,dest).copied,false);assert.equal(statSync(dest).mtimeMs,time);
 writeFileSync(src,'two');assert.equal(copyChanged(src,dest).copied,true);
 }finally{rmSync(dir,{recursive:true});}
});
test('revision includes untracked inputs and separates frontend, Rust and output',()=>{
 const dir=mkdtempSync(path.join(tmpdir(),'omniget inputs '));
 try{mkdirSync(path.join(dir,'src'));mkdirSync(path.join(dir,'src-tauri/target'),{recursive:true});
 const a=revisions(dir);writeFileSync(path.join(dir,'src/new.svelte'),'hello');const b=revisions(dir);
 assert.notEqual(a.frontendRevision,b.frontendRevision);assert.equal(a.backendRevision,b.backendRevision);
 writeFileSync(path.join(dir,'src-tauri/main.rs'),'fn main() {}');const c=revisions(dir);assert.notEqual(b.backendRevision,c.backendRevision);
 writeFileSync(path.join(dir,'src-tauri/target/output'),'ignored');assert.deepEqual(revisions(dir),c);
 }finally{rmSync(dir,{recursive:true});}
});

test('embedded frontend inputs also invalidate Rust and custom Cargo output is ignored',()=>{
 const dir=mkdtempSync(path.join(tmpdir(),'omniget embedded '));const previous=process.env.CARGO_TARGET_DIR;
 try{mkdirSync(path.join(dir,'src/lib/native-agent'),{recursive:true});mkdirSync(path.join(dir,'src-tauri/custom-output'),{recursive:true});
 process.env.CARGO_TARGET_DIR='custom-output';const before=revisions(dir);
 writeFileSync(path.join(dir,'src/lib/native-agent/catalog.json'),'[]');const after=revisions(dir);
 assert.notEqual(after.backendRevision,before.backendRevision);assert.notEqual(after.frontendRevision,before.frontendRevision);
 writeFileSync(path.join(dir,'src-tauri/custom-output/binary'),'output');assert.deepEqual(revisions(dir),after);
 }finally{if(previous===undefined)delete process.env.CARGO_TARGET_DIR;else process.env.CARGO_TARGET_DIR=previous;rmSync(dir,{recursive:true});}
});
