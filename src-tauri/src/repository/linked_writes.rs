use super::*;
pub(super) fn supports(action:&Action)->bool {matches!(action,Action::StageHunk{..}|Action::StageLines{..}|Action::Stage{..}|Action::StageAll|Action::StageFiles{..}|Action::Unstage{..}|Action::UnstageAll|Action::UnstageFiles{..}|Action::Commit{..}|Action::RecoverCheckout)}
pub(super) fn pending(root:&Path)->bool {commit_write::pending(root)||external_merge::pending(root)}
pub(super) fn policy(root:&Path)->Result<Option<String>> {
    let layout=layout::Layout::read(root)?;
    if !layout.linked||layout.submodule{return Ok(Some("Linked local writes are unavailable for this repository layout.".into()));}
    if let Some(reason)=validate(layout.common.parent().ok_or("INVALID_WORKTREE_LINK")?)? {return Ok(Some(reason));}
    let mut count=0;let mut attrs=false;check_tree(root,&mut count,&mut attrs,0)?;
    if attrs||["config.worktree","info/attributes","info/sparse-checkout"].iter().any(|p|fs::symlink_metadata(layout.private.join(p)).is_ok()) {return Ok(Some("Linked worktree attributes or per-worktree configuration require read-only mode.".into()));}
    for name in ["branchdesk-sequence.json","branchdesk-rebase-draft.json","branchdesk-replay-draft.json","branchdesk-merge-draft.json","branchdesk-checkout","branchdesk-fetch","branchdesk-stash-save","branchdesk-stash-apply"] {
        if fs::symlink_metadata(layout.private.join(name)).is_ok(){return Ok(Some("Unsupported pending operation in linked worktree metadata.".into()));}
    }
    let repo=open(root)?;
    if repo.state()!=git2::RepositoryState::Clean {return Ok(Some("Linked merge/rebase operations are not supported yet.".into()));}
    if commit_write::pending(root)&&external_merge::pending(root){return Ok(Some("Multiple linked recovery journals require manual inspection.".into()));}
    // A forced shared checkout would make committing here move another checkout's HEAD.
    let head=external_merge::head_lock_target(&repo)?;
    if head!="HEAD" {
        let expected=format!("ref: {head}");
        let shared=bounded(&layout.common.join("HEAD"),4096)?;
        if String::from_utf8_lossy(&shared).trim()==expected {return Ok(Some("Current branch is checked out in the main worktree.".into()));}
        let entries=fs::read_dir(layout.common.join("worktrees")).map_err(err)?;
        for (i,entry) in entries.enumerate(){if i>=200{return Ok(Some("Too many worktrees to verify branch ownership.".into()));}let path=entry.map_err(err)?.path();if path==layout.private{continue;}
            let meta=fs::symlink_metadata(&path).map_err(err)?;if !meta.is_dir()||is_link(&meta){return Err("UNSUPPORTED_WORKTREE_METADATA".into());}
            if String::from_utf8_lossy(&bounded(&path.join("HEAD"),4096)?).trim()==expected{return Ok(Some("Current branch is checked out in another worktree.".into()));}
        }
    }
    Ok(None)
}
