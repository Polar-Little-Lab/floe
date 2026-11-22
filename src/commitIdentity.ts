export interface CommitIdentity {name:string;email:string}
export interface IdentitySettings {identity:CommitIdentity|null;revision:string;warning:string|null}
// Never silently combine a partial per-operation override with another identity.
export function resolveCommitIdentity(name:string,email:string,fallback:CommitIdentity|null):CommitIdentity {
 return !name.trim()&&!email.trim()&&fallback?fallback:{name,email};
}

export interface RepositoryIdentity {identity:CommitIdentity|null;configured:boolean;warning:string|null}
export function repositoryDefaultIdentity(repository:RepositoryIdentity|undefined,app:CommitIdentity|null):CommitIdentity|null {
 return repository?.configured?repository.identity:app;
}
