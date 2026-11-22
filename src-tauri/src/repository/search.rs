use super::*;
#[derive(Serialize)]
pub struct Hit { pub commit: Commit, pub offset: usize }
#[derive(Serialize)]
pub struct Results { pub query: String, pub hits: Vec<Hit>, pub scanned: usize, pub truncated: bool }

pub(super) fn walk(repo: &Repository) -> Result<git2::Revwalk<'_>> {
    let mut walk = repo.revwalk().map_err(err)?;
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME).map_err(err)?;
    if let Ok(head) = repo.head().and_then(|h| h.peel_to_commit()) { walk.push(head.id()).map_err(err)?; }
    let mut tips = Vec::new();
    for reference in repo.references().map_err(err)? {
        let reference = reference.map_err(err)?;
        let name = reference.name().unwrap_or("");
        if name.starts_with("refs/heads/") || name.starts_with("refs/remotes/") || name.starts_with("refs/tags/") {
            if let Ok(commit) = reference.peel_to_commit() { tips.push((name.to_owned(), commit.id())); }
        }
    }
    tips.sort_by(|a,b| a.0.cmp(&b.0));
    for (_, oid) in tips { walk.push(oid).map_err(err)?; }
    Ok(walk)
}

impl Session {
    pub fn search_commits(&mut self, revision: &str, query: &str) -> Result<Results> {
        let query = query.trim();
        if query.is_empty() || query.len() > 512 { return Err("INVALID_SEARCH_QUERY".into()); }
        self.fresh(revision)?;
        let repo = open(&self.root)?;
        let needle = query.to_lowercase();
        let mut result = Results { query: query.into(), hits: Vec::new(), scanned: 0, truncated: false };
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        for (offset, oid) in walk(&repo)?.enumerate() {
            if offset >= 20000 || result.hits.len() >= 200 || std::time::Instant::now() >= deadline {
                result.truncated = true; break;
            }
            let commit = repo.find_commit(oid.map_err(err)?).map_err(err)?;
            result.scanned += 1;
            let oid = commit.id().to_string();
            let author = commit.author();
            let message: String = String::from_utf8_lossy(commit.message_bytes()).chars().take(65536).collect();
            if !format!("{oid}\n{}\n{}\n{message}", author.name().unwrap_or(""), author.email().unwrap_or("")).to_lowercase().contains(&needle) { continue; }
            result.hits.push(Hit { offset, commit: Commit { oid, parents: commit.parent_ids().map(|id|id.to_string()).collect(), summary: commit.summary().ok().flatten().unwrap_or("(non UTF-8 message)").chars().take(4096).collect(), author: author.name().unwrap_or("Unknown").chars().take(256).collect(), time: commit.time().seconds(), refs: Vec::new() } });
        }
        self.fresh(revision)?;
        Ok(result)
    }
    pub fn graph_at(&mut self, revision: &str, offset: usize) -> Result<Snapshot> {
        if offset >= 20000 { return Err("GRAPH_SEARCH_LIMIT".into()); }
        self.fresh(revision)?;
        let previous = self.graph_offset;
        let previous_window=self.graph_window_size;
        self.graph_window_size=self.graph_page_size;
        self.graph_offset = offset / self.graph_page_size * self.graph_page_size;
        match self.snapshot() { Ok(value) => Ok(value), Err(error) => { self.graph_offset = previous; self.graph_window_size=previous_window; Err(error) } }
    }
}
