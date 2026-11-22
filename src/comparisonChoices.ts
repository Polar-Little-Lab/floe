import type {Commit,Snapshot} from './types';
export type ComparisonChoice=Pick<Commit,'oid'|'summary'|'refs'>;
export function comparisonChoices(snapshot:Snapshot):ComparisonChoice[]{
 const choices=new Map<string,ComparisonChoice>(snapshot.commits.map(commit=>[commit.oid,{oid:commit.oid,summary:commit.summary,refs:[...commit.refs]}]));
 const add=(oid:string|null|undefined,label:string)=>{if(!oid)return;let choice=choices.get(oid);if(!choice){choice={oid,summary:'Reference tip outside loaded graph',refs:[]};choices.set(oid,choice);}if(!choice.refs.includes(label))choice.refs.push(label);};
 add(snapshot.headOid,'HEAD');
 for(const branch of snapshot.branches)add(branch.oid,`local: ${branch.name}`);
 for(const remote of snapshot.remotes??[])for(const branch of remote.branches)add(branch.oid,`remote: ${remote.name}/${branch.name}`);
 for(const tag of snapshot.tags)add(tag.oid,`tag: ${tag.name}`);
 return [...choices.values()];
}
