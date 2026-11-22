export interface TabDraft {message:string;description:string;author:string;email:string;amend:boolean;headOid?:string|null}
export function restoreDraft(draft:TabDraft|undefined,headOid:string|null|undefined):TabDraft {
 return draft?{...draft,amend:draft.amend&&!!headOid&&headOid===draft.headOid}:{message:'',description:'',author:'',email:'',amend:false,headOid};
}
export function hasCommitDraft(draft:TabDraft|undefined){return !!(draft?.message.trim()||draft?.description.trim());}
