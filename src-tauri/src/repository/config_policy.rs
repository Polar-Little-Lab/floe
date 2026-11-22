//! Bounded config preflight, before any parser can follow external includes.
use super::*;

pub(super) fn preflight(bytes: &[u8]) -> Result<()> {
    if bytes.len() > 65536 {
        return Err("CONFIG_LIMIT".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "INVALID_REMOTE_CONFIG")?;
    // libgit2 accepts multiple headers on one line and legacy dotted sections.
    // Scan every opening bracket, including continuation lines, before opening a
    // private copy. Header-shaped values/comments are conservatively refused;
    // ordinary values containing these words remain supported. No path is read.
    for tail in text.split('[').skip(1) {
        let section: String = tail
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect::<String>()
            .to_ascii_lowercase();
        if matches!(section.as_str(), "include" | "includeif" | "extensions") {
            return Err("UNSUPPORTED_CONFIG".into());
        }
    }
    Ok(())
}

pub(super) fn requires_read_only(entries: &[(String, String)]) -> bool {
    entries.iter().any(|(key, _)| {
        matches!(
            key.as_str(),
            "core.autocrlf"
                | "core.eol"
                | "core.hookspath"
                | "commit.gpgsign"
                | "tag.gpgsign"
                | "tag.forcesignannotated"
                | "user.signingkey"
        ) || key.starts_with("filter.")
            || key.starts_with("gpg.")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn config_policy_rejects_indirection_before_parse() {
        for text in [
            "[include]\npath=/outside",
            "\u{feff}[InClUdE] path=/outside",
            "[core][include]path=/outside",
            "[includeIf \"gitdir:/tmp/\"]\npath=/outside",
            "[include.path]\nx=y",
            "[extensions]\nworktreeConfig=true",
        ] {
            assert!(preflight(text.as_bytes()).is_err(), "{text}");
        }
    }
    #[test]
    fn config_policy_values_are_not_keys() {
        let text = b"# include worktree extensions sign filter\n[remote \"design-filter\"]\nurl=https://example.invalid/include/worktree/extensions.git\n[branch \"sign\"]\nremote=design-filter\nmerge=refs/heads/filter\n";
        let entries = remote_remove::entries(text).unwrap();
        assert!(!requires_read_only(&entries));
        for text in ["[core]\nworktree=/outside", "[core]\nworktree = /tmp"] {
            assert!(remote_remove::entries(text.as_bytes()).is_err());
        }
        for key in [
            "core.autocrlf",
            "core.hookspath",
            "commit.gpgsign",
            "filter.example.clean",
            "gpg.ssh.program",
        ] {
            assert!(requires_read_only(&[(key.into(), "false".into())]));
        }
    }
}
