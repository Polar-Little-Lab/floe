//! Bounded read-only snapshot summary; callers authorize the repository path.
use super::*;
#[derive(Serialize)]
#[serde(rename_all="camelCase")]
pub struct Checkout {pub path:String,pub head_oid:Option<String>,pub branch:String,pub detached:bool,pub staged:usize,pub unstaged:usize,pub changed:usize,pub write_block:Option<String>}
pub(super) fn read(root:PathBuf,label:&str)->Result<Checkout>{
    let mut session=Session::new(root)?;
    let snapshot=session.snapshot()?;
    Ok(Checkout{path:label.into(),head_oid:snapshot.head_oid,branch:snapshot.branch,detached:snapshot.detached,
        staged:snapshot.changes.iter().filter(|file|file.staged).count(),
        unstaged:snapshot.changes.iter().filter(|file|file.unstaged).count(),
        changed:snapshot.changes.len(),write_block:snapshot.write_block})
}
