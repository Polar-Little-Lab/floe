import type {Branch,Snapshot} from './types';
import {validPushDestination} from './PushDestinationDialog';
export type ToolbarPull = {kind:'pull';name:string}|{kind:'configure';branch:Branch}|{kind:'addRemote'}|{kind:'unavailable'};
export function toolbarPull(snapshot:Snapshot):ToolbarPull {
 if(snapshot.repoId==='demo'||snapshot.detached||!snapshot.headOid)return {kind:'unavailable'};
 const branch=snapshot.branches.find(b=>b.current&&b.name===snapshot.branch);
 if(!branch||!branch.oid)return {kind:'unavailable'};
 const remotes=snapshot.remotes??[];
 if(!remotes.length)return {kind:'addRemote'};
 const upstream=branch.upstream;
 const remote=upstream?.remotes.length===1?upstream.remotes[0]:null;
 const merge=upstream?.merges.length===1?upstream.merges[0]:null;
 if(remote&&remotes.some(r=>r.name===remote)&&merge?.startsWith('refs/heads/')&&validPushDestination(merge.slice(11)))return {kind:'pull',name:remote};
 return {kind:'configure',branch};
}
export function pushTargetFor(snapshot:Snapshot,name:string) {
 const remote=snapshot.remotes?.find(r=>r.name===name);
 if(!remote||snapshot.repoId==='demo'||snapshot.detached||!snapshot.headOid||snapshot.pushRecord||snapshot.pushRecordError)return null;
 return {repoId:snapshot.repoId,revision:snapshot.revision,name,source:snapshot.branch,sourceOid:snapshot.headOid,branches:remote.branches.filter(b=>!b.symbolicTarget&&b.name!=='HEAD').map(b=>b.name)};
}
