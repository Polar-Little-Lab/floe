export interface WorkspaceWipEntry {repository:string;localPath:string|null;state:'unmapped'|'notOpen'|'error'|'clean'|'changes';repoId:string|null;branch:string|null;changed:number|null;staged:number|null;unstaged:number|null;warning:string|null}
export interface WorkspaceWipListing {workspaceId:string;entries:WorkspaceWipEntry[]}
export interface OpenRepositoryPath {repoId:string;path:string}
