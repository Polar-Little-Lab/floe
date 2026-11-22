import {api} from './api';
export type CommitTemplate={enabled:boolean;removeComments:boolean;summary:string;description:string};
export const emptyCommitTemplate:CommitTemplate={enabled:false,removeComments:false,summary:'',description:''};
const prefix='branchdesk.commit-template.v1:';
export async function repositoryTemplatePath(repoId:string){const paths=await api.openRepositoryPaths();const path=paths.find(item=>item.repoId===repoId)?.path;if(!path)throw Error('Open repository path is unavailable.');return path;}
export function readCommitTemplate(path:string):CommitTemplate{const raw=localStorage.getItem(prefix+path);if(!raw)return {...emptyCommitTemplate};const value=JSON.parse(raw);if(typeof value?.enabled!=='boolean'||typeof value.summary!=='string'||typeof value.description!=='string'||value.summary.length>1024||value.description.length>65536)throw Error('Saved commit template is invalid. Restore defaults to replace it.');return {...value,removeComments:value.removeComments===true};}
export function saveCommitTemplate(path:string,value:CommitTemplate){localStorage.setItem(prefix+path,JSON.stringify(value));}
export async function templateForRepository(repoId:string){return readCommitTemplate(await repositoryTemplatePath(repoId));}

/** App policy only: remove lines starting with #, never inline # or indented text. */
export function prepareCommitMessage(message:string,template:CommitTemplate){return template.removeComments?message.split(/\r?\n/).filter(line=>!line.startsWith('#')).join('\n'):message;}
