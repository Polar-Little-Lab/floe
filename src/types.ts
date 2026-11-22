export interface Commit { oid: string; parents: string[]; summary: string; author: string; time: number; refs: string[] }
export interface Change { id: string; path: string; staged: boolean; unstaged: boolean; status: string }
export interface Branch { name: string; oid: string; current: boolean; upstream?: {remotes:string[];merges:string[]}|null }
export interface TagRef { name: string; oid: string | null; annotated: boolean }
export type MergeResolution={kind:'ours'|'theirs'|'delete'}|{kind:'text';text:string};
export interface MergeDraft {name:string;branch:string;head:string;target:string;resolutions:Record<string,MergeResolution>}
export interface ExternalMerge {heads:string[];conflicts:MergeConflict[]}
export interface SequenceProgress {aborting?:boolean;current?:ReplaySelection|null;resolutions?:Record<string,MergeResolution>;id:string;branch:string;originalHead:string;expectedHead:string;total:number;completed:number;pending:boolean}
export interface RemoteInfo {name:string;tagPolicy?:string;fetchUrls:string[];pushUrls:string[];fetchRefspecs:string[];branches:{name:string;oid:string|null;symbolicTarget:string|null}[]}
export interface Snapshot {graphPageSize?:number;graphWindowSize?:number;linkedLocalWrites?:boolean;repositoryIdentity?:import("./commitIdentity").RepositoryIdentity;pushRecord?:PushRecord|null;pushRecordError?:string|null;rebaseDraft?:{id:string;branch:string;head:string;upstream:string}|null;rebaseDraftError?:string|null;graphOffset?:number;detached?:boolean;remotes?:RemoteInfo[]; sequenceProgress?:SequenceProgress|null;sequenceError?:string|null; replayCompletionReady?:boolean;replayDraft?:ReplayDraft|null;replayDraftError?:string|null;headOid?:string|null;externalMerge?:ExternalMerge|null;mergeDraftError?:string|null;mergeDraft?:MergeDraft|null; repoId: string; name: string; branch: string; revision: string; commits: Commit[]; branches: Branch[]; tags: TagRef[]; stashes: Stash[]; changes: Change[]; hasMore: boolean; writeBlock: string | null; recoveryPending: boolean }
export interface SelectedDiffLine {hunk:number;line:number;oldLine:number|null;newLine:number|null}
export interface DiffResult { selectableLines?:SelectedDiffLine[]; text: string; truncated: boolean; hunks?:{index:number;header:string}[] }
export interface CommitDetail { oid: string; message: string; parentOid: string | null; files: Change[]; diff: DiffResult | null }
export type Action = {kind:"setGitflowConfiguration";configuration:{main:string;develop:string;feature:string;release:string;hotfix:string;versionTag:string}} | {kind:'stageLines';file_id:string;lines:{hunk:number;line:number}[];unstage:boolean}| {kind:"applyPatchIndex";text:string}| {kind:"pruneWorktree";name:string;expected_metadata:string}| {kind:"setWorktreeLock";name:string;expected_locked:boolean;locked:boolean}| {kind:"createWorktree";name:string;path:string;branch:string|null}| {kind:"createRecoveryBranch";reference:string;oid:string;name:string}| {kind:"ignoreFile";file_id:string}| {kind:"softReset";oid:string}| {kind:"setRepositoryIdentity";identity:import("./commitIdentity").CommitIdentity|null}| {kind:"setUpstream";name:string;target:{remote:string;branch:string}|null}| {kind:"startRebase";upstream:string;author:string;email:string}|{kind:"resolveRebase";draft_id:string;source:string;file_id:string;resolution:MergeResolution}|{kind:"skipRebase";draft_id:string;source:string}|{kind:"continueRebase"|"abortRebase"}| {kind:"rebase";upstream:string;author:string;email:string}| {kind:"setRemoteTagPolicy";name:string;policy:"auto"|"all"|"none"}| {kind:"createBranchAt";name:string;oid:string}| {kind:"checkoutCommit";oid:string}| {kind:"renameRemote";name:string;new_name:string}| {kind:"removeRemote";name:string}| {kind:"setRemote";name:string;url:string;push_url:string|null;create:boolean}| {kind:"abortSequence"}| {kind:"resolveSequence";sequence_id:string;ordinal:number;file_id:string;resolution:MergeResolution}| {kind:"skipSequence";sequence_id:string;ordinal:number}| {kind:"startSequence";selections:ReplaySelection[]}|{kind:"continueSequence";message:string;author:string;email:string;allow_empty?:boolean}|{kind:"stopSequence"}| {kind:'startReplay';oid:string;operation:ReplayKind;mainline:number|null}|{kind:'resolveReplay';file_id:string;resolution:MergeResolution}|{kind:'abortReplay'}|{kind:'skipReplay'}|{kind:'continueReplay';allow_empty?:boolean;message:string;author:string;email:string}| {kind:'replayCommit';allow_empty?:boolean;oid:string;operation:ReplayKind;mainline:number|null;message:string;author:string;email:string} | {kind:'renameBranch';name:string;new_name:string}| {kind:'stageHunk';file_id:string;hunk:number;unstage:boolean}| {kind:'abortExternalMerge'}| {kind:'continueExternalMerge';message:string;author:string;email:string}| {kind:'stageExternalResolution';file_id:string}| {kind:'startMerge';name:string}|{kind:'resolveMerge';file_id:string;resolution:MergeResolution}|{kind:'abortMerge'}|{kind:'continueMerge';message:string;author:string;email:string}| {kind:'mergeCommit';name:string;message:string;author:string;email:string} | {kind:'createAnnotatedTag';name:string;oid:string;message:string;author:string;email:string} | {kind:'saveStash'|'saveStashKeepingIndex';message:string;include_untracked:boolean} | {kind:'applyStash'|'popStash';oid:string;restore_index:boolean} | {kind: 'dropStash'; oid: string} | {kind:'stageFiles'|'unstageFiles';file_ids:string[]}| { kind: 'stage' | 'unstage'; file_id: string } | { kind: 'stageAll' | 'unstageAll' | 'recoverCheckout' } | { kind: 'commit' | 'amendCommit'; message: string; author: string; email: string } | { kind: 'createBranch' | 'deleteBranch' | 'deleteTag' | 'checkout' | 'fastForwardMerge'; name: string } | { kind: 'createTag'; name: string; oid: string };

export interface Stash { oid: string; index: number; message: string }
export type StashArea = 'staged' | 'unstaged' | 'untracked';
export interface StashDetail { oid: string; message: string; baseOid: string; groups: {area:StashArea;files:Change[]}[]; diff: DiffResult | null }

export interface HistoryEntry {oid:string;summary:string;author:string;time:number;path:string;previousPath:string|null;status:string}
export interface FileHistory {path:string;entries:HistoryEntry[];offset:number;scanned:number;nextOffset:number|null;limitReached:boolean;diff:DiffResult|null}

export interface BlameHunk {start:number;lines:number;oid:string;author:string;time:number;originalPath:string;originalStart:number;boundary:boolean}
export interface FileBlame {oid:string;path:string;lines:string[];hunks:BlameHunk[];historyLimited:boolean}

export type ContentSource={kind:'comparison';base:string;target:string;before:boolean}|{kind:'treeHistory';anchor:string;path:string;offset:number;selected:string;before:boolean}|{kind:'treeFile';oid:string;path:string}|{kind:'replayConflict';oid:string;operation:ReplayKind;mainline:number|null;conflict:number;side:'ancestor'|'ours'|'theirs'}|{kind:'indexConflict';side:'ancestor'|'ours'|'theirs'}|{kind:'mergeConflict';name:string;conflict:number;side:'ancestor'|'ours'|'theirs'}|{kind:'worktree'|'index'|'head'}|{kind:'commit'|'commitBefore';oid:string}|{kind:'history'|'historyBefore';anchor:string|null;offset:number;selected:string}|{kind:'stash'|'stashBefore';oid:string;area:StashArea};
export interface FileContent {lfsUploadRecord?:import("./lfsTransfer").LfsUploadRecord;lfsUploadError?:string;lfsObject?:{state:string;message:string};image?:{mime:string;data:string;width:number;height:number};imageError?:string;path:string;source:string;state:'text'|'binary'|'unsupportedEncoding'|'tooLarge'|'deleted';size:number;text:string;truncated:boolean;endsWithNewline:boolean}

export interface ConflictVersion {id:string;path:string;oid:string}
export interface MergeConflict {ancestor:ConflictVersion|null;ours:ConflictVersion|null;theirs:ConflictVersion|null}
export interface MergePlan {revision:string;currentBranch:string;targetBranch:string;headOid:string;targetOid:string;baseOid:string|null;kind:'upToDate'|'fastForward'|'mergeCommit'|'conflicts'|'unrelated'|'multipleBases';blockers:string[];files:Change[];conflicts:MergeConflict[];diff:DiffResult|null}

export type ReplayKind = 'cherryPick'|'revert';
export interface ReplayPlan { originallyEmpty?:boolean; resolvedEmpty?:boolean|null; revision:string; head:string; target:string; parent:string|null; kind:ReplayKind; resultTree:string|null; conflicts:MergeConflict[]; blockers:string[]; empty:boolean; diff:DiffResult }

export interface ReplayDraft { version:number; branch:string; head:string; source:string; kind:ReplayKind; mainline:number|null; resolutions:Record<string,MergeResolution> }

export interface ReplaySelection {oid:string;kind:ReplayKind;mainline:number|null}
export interface ReplaySequenceStep {selection:ReplaySelection;beforeTree:string;resultTree:string|null;parent:string|null;originallyEmpty:boolean;empty:boolean;conflicts:MergeConflict[];diff:DiffResult|null}
export interface ReplaySequencePlan {revision:string;head:string;requested:number;steps:ReplaySequenceStep[];stoppedAt:number|null;resultTree:string|null;blockers:string[]}

export interface RemoteRemovePreview {name:string;removedRefs:string[];preservedRefs:string[];removedConfigKeys:string[]}

export interface CommitComparison {afterOnly?:number;beforeOnly?:number;countsLimited?:boolean;baseOid:string;targetOid:string;files:Change[];diff:DiffResult|null}

/** Read-only analysis for the forthcoming journaled rebase execution flow. */
export type RebasePlan = {
 revision:string;branch:string;upstream:string;headOid:string;upstreamOid:string;baseOid:string|null;
 kind:'replay'|'upToDate'|'fastForward'|'unrelated';selections:ReplaySelection[];
 skippedEquivalent:string[];flattenedMerges:string[];replay:ReplaySequencePlan|null;
 resultTree:string|null;blockers:string[];
};

export type RebaseDraftView={id:string;branch:string;upstream:string;revision:string;resultOid:string;completed:boolean;skipped:string[];pending:{source:string;ordinal:number;conflicts:{id:string;path:string;ancestor:string|null;ours:string|null;theirs:string|null;binary:boolean;present:[boolean,boolean,boolean]}[];resolutions:Record<string,MergeResolution>}|null};

export type PreparedPull={snapshot:Snapshot;remote:string;source:string}&({mode:'merge';plan:MergePlan}|{mode:'rebase';plan:RebasePlan});

export type PushPlan={revision:string;remote:string;endpoint:string;source:string;destination:string;sourceOid:string;remoteOid:string|null;kind:string;outgoing:string[];outgoingTruncated:boolean;blockers:string[];packBytes:number|null;packObjects:number|null};

export type PushRecord={id:string;remote:string;endpoint:string;source:string;destination:string;oldOid:string|null;newOid:string;status:"pending"|"accepted"|"rejected"|"notSent";detail:string|null;observation?:{kind:"targetPresent"|"previousTarget"|"differentTarget"|"missing";oid:string|null;checkedAt:number}|null};
