const refreshMessages:Record<string,string>={
 PROVIDER_PULL_NOT_OPEN:'This pull request is no longer open. Refresh the item before continuing.',
 PROVIDER_PULL_DRAFT_OR_UNKNOWN:'This pull request is a draft or its draft status could not be confirmed. Refresh the item before merging.',
 PROVIDER_PULL_CHANGED:'The pull request head changed. Refresh and review the latest commits before merging.',
 PROVIDER_PULL_ALREADY_MERGED:'This pull request has already been merged. Refresh to see its current state.',
 PROVIDER_MERGE_CONFLICTS:'GitHub reports merge conflicts. Resolve them, then refresh this item.',
 PROVIDER_MERGEABILITY_PENDING:'GitHub is still calculating mergeability. Wait briefly, then refresh this item.',
 PROVIDER_REVIEWERS_CHANGED:'Requested reviewers changed on GitHub. Refresh before editing reviewers.',
 PROVIDER_ASSIGNEES_CHANGED:'Assignees changed on GitHub. Refresh before editing assignees.',
 PROVIDER_LABELS_CHANGED:'Labels changed on GitHub. Refresh before editing labels.',
 PROVIDER_ITEM_STATE_CHANGED:'The item state changed on GitHub. Refresh before closing or reopening it.',
 PROVIDER_COMMENT_CHANGED:'The comment changed on GitHub. Refresh before editing it.',
 PROVIDER_ITEM_TARGET_MISMATCH:'The returned item does not match this operation. Refresh before continuing.',
 PROVIDER_COMMENT_TARGET_MISMATCH:'The returned comment does not match this item. Refresh before continuing.',
};
export function providerWriteFailure(error:unknown){
 const text=String(error);
 const code=text.match(/\bPROVIDER_[A-Z_]+\b/)?.[0]??'';
 return {message:refreshMessages[code]??text,refreshRequired:Object.hasOwn(refreshMessages,code),outcomeUnknown:code==='PROVIDER_WRITE_OUTCOME_UNKNOWN'};
}
