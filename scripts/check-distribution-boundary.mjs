import {readFile,readdir} from 'node:fs/promises';
import {join} from 'node:path';
const forbidden=new Set(['design','reference','screenshots','inventory','reports','analysis','.agents','docs','tests','node_modules']);
async function walk(directory){
 for(const entry of await readdir(directory,{withFileTypes:true})){
  if(entry.isSymbolicLink())throw new Error(`Unexpected distribution symlink: ${entry.name}`);
  if(forbidden.has(entry.name.toLowerCase()))throw new Error(`Development directory in distribution: ${entry.name}`);
  if(entry.isDirectory())await walk(join(directory,entry.name));
 }
}
for(const name of ['tauri.conf.json','tauri.windows.conf.json','tauri.devtest.conf.json']){
 const config=JSON.parse(await readFile(`src-tauri/${name}`,'utf8'));
 if(config.bundle?.resources&&Object.keys(config.bundle.resources).length)throw new Error('Review bundle resources before shipping development references');
 if(config.bundle?.externalBin?.length)throw new Error('Review sidecars before shipping development tools');
 if(config.build?.frontendDist&&config.build.frontendDist!=='../dist')throw new Error('Unexpected frontend distribution');
}
await walk('dist');
console.log('Distribution boundary passed: frontend dist only, no development resource bundles or sidecars.');
