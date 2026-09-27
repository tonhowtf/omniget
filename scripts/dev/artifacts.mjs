import {createHash} from 'node:crypto';
import {readFileSync,existsSync,copyFileSync,renameSync,mkdirSync,statSync} from 'node:fs';
import path from 'node:path';
export function fileDigest(file) {return createHash('sha256').update(readFileSync(file)).digest('hex');}
export function copyChanged(source,dest) {
 if(statSync(source).size===0)throw Error('EMPTY_ARTIFACT: '+source);
 const digest=fileDigest(source);
 if(existsSync(dest)&&fileDigest(dest)===digest)return {digest,copied:false};
 mkdirSync(path.dirname(dest),{recursive:true});
 const pending=dest+'.'+process.pid+'.pending';copyFileSync(source,pending);
 if(fileDigest(pending)!==digest)throw Error('ARTIFACT_CHANGED_DURING_COPY');
 renameSync(pending,dest);
 return {digest,copied:true};
}
