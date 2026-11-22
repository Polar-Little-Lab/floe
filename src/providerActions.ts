export type ProviderState='open'|'closed';
export type ProviderAction=
 |{kind:'changeReviewers';number:number;reviewers:string[];remove:boolean;expectedReviewers:string[]}
 |{kind:'createIssue';title:string;body:string}
 |{kind:'setAssignees';number:number;pull:boolean;assignees:string[];expectedAssignees:string[]}
 |{kind:'setLabels';number:number;pull:boolean;labels:string[];expectedLabels:string[]}
 |{kind:'setItemState';number:number;pull:boolean;state:ProviderState;expectedState:ProviderState}
 |{kind:'updateIssue'|'updatePull';number:number;title:string;body:string;state:ProviderState}
 |{kind:'createPull';title:string;body:string;head:string;base:string;draft:boolean}
 |{kind:'review';number:number;sha:string;body:string;event:'COMMENT'|'APPROVE'|'REQUEST_CHANGES'}
 |{kind:'deleteComment';number:number;pull:boolean;commentId:number;expectedBody:string}
 |{kind:'updateComment';number:number;pull:boolean;commentId:number;expectedBody:string;body:string}
 |{kind:'createComment';number:number;pull:boolean;body:string}
 |{kind:'replyReview';number:number;commentId:number;body:string}
 |{kind:'merge';number:number;sha:string;method:'merge'|'squash'|'rebase'};
export interface ProviderDetail{number:number;title:string;body:string;state:ProviderState;headSha:string|null;baseSha?:string|null;url:string;author?:string|null;headBranch?:string|null;baseBranch?:string|null;reviewers?:string[]|null;assignees?:string[]|null;labels?:string[]|null;milestone?:string|null;draft?:boolean|null;merged?:boolean|null;mergeable?:boolean|null;changedFiles?:number|null;additions?:number|null;deletions?:number|null;commits?:number|null}
export interface ProviderReceipt{number:number;url:string;merged:boolean}

export interface ProviderPullFile{path:string;previousPath:string|null;status:string;additions:number;deletions:number;patch:string|null}
export interface ProviderPullFiles{files:ProviderPullFile[];nextPage:number|null;limitReached:boolean}
