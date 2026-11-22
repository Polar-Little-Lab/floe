import {CopyTextButton} from './CopyTextButton';
import type {Commit} from './types';
export function CommitMetadata({commit,message,commits,busy,onParent}:{commit:Commit;message?:string;commits:Commit[];busy:boolean;onParent:(oid:string)=>void}){
 const date=new Date(commit.time*1000);const timestamp=Number.isFinite(date.getTime())?date.toISOString():String(commit.time);
 const loaded=new Map(commits.map(value=>[value.oid,value]));
 return <><dl><dt>SHA</dt><dd className="commit-sha">{commit.oid}<CopyTextButton text={commit.oid} label="Copy SHA"/></dd><dt>Parents</dt><dd>{commit.parents.length?commit.parents.map((oid,index)=><div className="commit-parent" key={oid}><button disabled={busy} title={loaded.get(oid)?.summary??oid} aria-label={`Open parent ${index+1}: ${oid}`} onClick={()=>onParent(oid)}>{commit.parents.length>1?`Parent ${index+1} · `:''}{oid.slice(0,7)}</button><CopyTextButton text={oid} label={`Copy parent ${index+1} SHA`}/>{loaded.get(oid)&&<small>{loaded.get(oid)!.summary}</small>}</div>):'Initial commit'}</dd></dl>{message!==undefined&&<div className="commit-message-tools"><CopyTextButton text={message} label="Copy commit message"/><CopyTextButton text={`${commit.oid}\n${commit.author}\n${timestamp}\n\n${message}`} label="Copy commit details"/></div>}</>;
}
