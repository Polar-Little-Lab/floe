import {createHash} from 'node:crypto';
import {createReadStream} from 'node:fs';
import {readFile,readdir,writeFile} from 'node:fs/promises';
import {join} from 'node:path';
const directory='preview-artifacts';
const filenames=(await readdir(directory)).filter(name=>/\.(zip|exe)$/.test(name)).sort();
if(filenames.length!==1)throw new Error('Expected exactly one packaged application');
const artifacts=[];
for(const name of filenames){
 const hash=createHash('sha256');let bytes=0;
 for await(const chunk of createReadStream(join(directory,name))){hash.update(chunk);bytes+=chunk.length;}
 if(!bytes)throw new Error(`Empty artifact: ${name}`);
 artifacts.push({name,bytes,sha256:hash.digest('hex')});
}
const pkg=JSON.parse(await readFile('package.json','utf8'));
const manifest={product:process.env.PREVIEW_PRODUCT||'FLOE',version:pkg.version,channel:process.env.PREVIEW_CHANNEL||'internal-preview',platform:process.env.PREVIEW_PLATFORM,commit:process.env.GITHUB_SHA,runId:process.env.GITHUB_RUN_ID,createdAt:new Date().toISOString(),verification:'Compilation/package only; no runtime acceptance tests in this workflow.',distribution:'macOS CI requires Developer ID signing, notarization, stapling and Gatekeeper verification. Windows code signing and automatic update are not configured.',artifacts};
await writeFile(join(directory,'manifest.json'),JSON.stringify(manifest,null,2)+'\n');
await writeFile(join(directory,'SHA256SUMS'),artifacts.map(item=>`${item.sha256}  ${item.name}`).join('\n')+'\n');
