import type {WorkspaceIconName} from './ui/WorkspaceIcon';
import type {TaskSnooze,TaskIdentity} from './taskSnoozes';
import type {WorkspaceTaskFilters} from './workspaceTaskFilters';
// These groups currently scope the GitHub.com provider only. Local mappings do not grant access.
export interface NamedRepository {owner:string;repo:string;localPath:string|null}
export interface SavedWorkspaceView {id:string;name:string;query:string;kind:'all'|'pulls'|'issues'|'wips'|'snoozed';filters:WorkspaceTaskFilters}
export interface NamedWorkspaceGroup {description?:string;color?:number;icon?:WorkspaceIconName;id:string;name:string;repositories:NamedRepository[];views?:SavedWorkspaceView[]}
export interface NamedWorkspace {snoozes?:TaskSnooze[];pins?:TaskIdentity[];groups:NamedWorkspaceGroup[];selected:string|null}
export interface NamedWorkspaceState {workspace:NamedWorkspace|null;revision:string;warning:string|null}

import {repositoryScope} from './providerPullRequests';
export function workspaceDraft(name:string,text:string,previous:NamedRepository[]=[]):{name:string;repositories:NamedRepository[]}{
 const cleaned=name.trim();if(!cleaned||new TextEncoder().encode(cleaned).length>256||/[\u0000-\u001f\u007f]/.test(cleaned))throw Error('Enter a workspace name up to 256 bytes.');
 const lines=text.split(/\r?\n/).map(line=>line.trim()).filter(Boolean);if(lines.length>100)throw Error('A workspace supports up to 100 repositories.');
 const seen=new Set<string>();const repositories=lines.map(line=>{const [owner,repo]=repositoryScope(line),key=(owner+'/'+repo).toLowerCase();if(seen.has(key))throw Error('Duplicate workspace repository.');seen.add(key);const old=previous.find(item=>(item.owner+'/'+item.repo).toLowerCase()===key);return {owner,repo,localPath:old?.localPath??null};});
 return {name:cleaned,repositories};
}

export const workspaceColors=['#15a0bf','#0669f7','#8e00c2','#c517b6','#d90171','#cd0101'] as const;
export function workspaceAppearance(description:string,color:number,icon:WorkspaceIconName=''){
 if(new TextEncoder().encode(description).length>4096||/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/.test(description))throw Error('Description must be at most 4096 bytes without control characters.');
 if(!Number.isInteger(color)||color<0||color>=workspaceColors.length)throw Error('Choose a workspace color.');
 if(!['','folder','branch','rocket','tag'].includes(icon))throw Error('Choose a workspace icon.');
 return {description,color,icon};
}
