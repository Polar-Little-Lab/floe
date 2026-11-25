//! Single-ref receive-pack protocol, validated against Git before transport wiring.
use git2::Oid;
const MAX_PACK: usize = 128 * 1024 * 1024;
const MAX_REPORT: usize = 64 * 1024;
#[derive(Debug, PartialEq, Eq)]
pub enum Report {
    Accepted,
    Rejected(String),
}
fn valid_ref(reference: &str) -> bool {
    reference.starts_with("refs/heads/")
        && reference.len() <= 1024
        && git2::Reference::is_valid_name(reference)
}
fn valid_oid(value: &str) -> bool {
    value.len() == 40 && Oid::from_str(value).is_ok()
}
pub fn request(
    old: &str,
    new: &str,
    reference: &str,
    capabilities: &[String],
    pack: &[u8],
) -> Result<Vec<u8>, String> {
    if !valid_ref(reference)
        || !valid_oid(old)
        || !valid_oid(new)
        || Oid::from_str(new).unwrap().is_zero()
        || old == new
    {
        return Err("PUSH_COMMAND_INVALID".into());
    }
    if !capabilities.iter().any(|c| c == "report-status") {
        return Err("PUSH_REPORT_STATUS_REQUIRED".into());
    }
    if pack.len() < 32 || pack.len() > MAX_PACK || !pack.starts_with(b"PACK") {
        return Err("PUSH_PACK_INVALID".into());
    }
    let line = format!("{old} {new} {reference}\0report-status\n");
    let mut output = format!("{:04x}{line}0000", line.len() + 4).into_bytes();
    output.extend_from_slice(pack);
    Ok(output)
}
fn packet<'a>(input: &mut &'a [u8]) -> Result<Option<&'a [u8]>, String> {
    if input.len() < 4 {
        return Err("PUSH_OUTCOME_UNKNOWN".into());
    }
    let prefix = std::str::from_utf8(&input[..4]).map_err(|_| "PUSH_OUTCOME_UNKNOWN")?;
    if !prefix.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("PUSH_OUTCOME_UNKNOWN".into());
    }
    let size = usize::from_str_radix(prefix, 16).map_err(|_| "PUSH_OUTCOME_UNKNOWN")?;
    *input = &input[4..];
    if size == 0 {
        return Ok(None);
    }
    if !(4..=65520).contains(&size) || input.len() < size - 4 {
        return Err("PUSH_OUTCOME_UNKNOWN".into());
    }
    let value = &input[..size - 4];
    *input = &input[size - 4..];
    Ok(Some(value))
}
fn text(bytes: &[u8]) -> Result<&str, String> {
    let value = std::str::from_utf8(bytes).map_err(|_| "PUSH_OUTCOME_UNKNOWN")?;
    let value = value.strip_suffix('\n').unwrap_or(value);
    if value.is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err("PUSH_OUTCOME_UNKNOWN".into());
    }
    Ok(value)
}
pub fn report(bytes: &[u8], reference: &str) -> Result<Report, String> {
    if bytes.len() > MAX_REPORT || !valid_ref(reference) {
        return Err("PUSH_OUTCOME_UNKNOWN".into());
    }
    let mut remaining = bytes;
    let unpack = text(packet(&mut remaining)?.ok_or("PUSH_OUTCOME_UNKNOWN")?)?;
    let unpack = unpack
        .strip_prefix("unpack ")
        .filter(|v| !v.is_empty())
        .ok_or("PUSH_OUTCOME_UNKNOWN")?;
    let result = text(packet(&mut remaining)?.ok_or("PUSH_OUTCOME_UNKNOWN")?)?;
    // Exactly one requested ref must be acknowledged, followed by the terminal flush.
    if packet(&mut remaining)?.is_some() || !remaining.is_empty() {
        return Err("PUSH_OUTCOME_UNKNOWN".into());
    }
    if result == format!("ok {reference}") {
        if unpack != "ok" {
            return Err("PUSH_OUTCOME_UNKNOWN".into());
        }
        Ok(Report::Accepted)
    } else if let Some(reason) = result
        .strip_prefix(&format!("ng {reference} "))
        .filter(|r| !r.is_empty())
    {
        Ok(Report::Rejected(reason.to_owned()))
    } else {
        Err("PUSH_OUTCOME_UNKNOWN".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        io::Write,
        path::Path,
        process::{Command, Stdio},
    };
    fn run(root: &Path, args: &[&str], input: &[u8]) -> std::process::Output {
        let mut child = Command::new(if cfg!(windows) { "git.exe" } else { "/usr/bin/git" })
            .arg("-C")
            .arg(root)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(input).unwrap();
        child.wait_with_output().unwrap()
    }
    fn git(root: &Path, args: &[&str]) -> String {
        let out = run(root, args, b"");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }
    fn wire(lines: &[&str]) -> Vec<u8> {
        let mut result = Vec::new();
        for line in lines {
            result.extend_from_slice(format!("{:04x}{line}", line.len() + 4).as_bytes());
        }
        result.extend_from_slice(b"0000");
        result
    }
    #[test]
    fn report_requires_complete_exact_unambiguous_single_ref_acknowledgement() {
        let reference = "refs/heads/main";
        let success = wire(&["unpack ok\n", "ok refs/heads/main\n"]);
        assert_eq!(report(&success, reference).unwrap(), Report::Accepted);
        assert_eq!(
            report(
                &wire(&["unpack ok", "ng refs/heads/main rejected"]),
                reference
            )
            .unwrap(),
            Report::Rejected("rejected".into())
        );
        for end in 0..success.len() {
            assert!(report(&success[..end], reference).is_err());
        }
        for lines in [
            vec!["unpack bad", "ok refs/heads/main"],
            vec!["unpack ok", "ok refs/heads/other"],
            vec!["unpack ok"],
            vec!["unpack ok", "ok refs/heads/main", "ok refs/heads/main"],
            vec!["unpack ok", "ng refs/heads/main "],
            vec!["unpack ok", "ok refs/heads/main\0"],
            vec!["unpack ok", "ok refs/heads/main", "option forced-update"],
        ] {
            assert!(report(&wire(&lines), reference).is_err());
        }
        let mut trailing = success.clone();
        trailing.push(0);
        assert!(report(&trailing, reference).is_err());
    }
    #[test]
    fn actual_receive_pack_accepts_creates_updates_and_rejects_races_and_hooks() {
        for mode in ["create", "update", "stale", "hook", "corrupt"] {
            let d = tempfile::tempdir().unwrap();
            let local = d.path().join("local");
            let server = d.path().join("server");
            git(
                d.path(),
                &["init", "--template=", "-b", "main", local.to_str().unwrap()],
            );
            git(&local, &["config", "user.name", "Push fixture"]);
            git(&local, &["config", "user.email", "push@invalid"]);
            fs::write(local.join("file"), "base").unwrap();
            git(&local, &["add", "file"]);
            git(&local, &["commit", "-m", "base"]);
            let old = git(&local, &["rev-parse", "HEAD"]).trim().to_owned();
            git(
                d.path(),
                &[
                    "clone",
                    "--template=",
                    "--bare",
                    local.to_str().unwrap(),
                    server.to_str().unwrap(),
                ],
            );
            fs::write(local.join("file"), "next").unwrap();
            git(&local, &["commit", "-am", "next"]);
            let new = git(&local, &["rev-parse", "HEAD"]).trim().to_owned();
            let expected = if mode == "create" {
                git(&server, &["update-ref", "-d", "refs/heads/main"]);
                "0".repeat(40)
            } else if mode == "stale" {
                "1".repeat(40)
            } else {
                old.clone()
            };
            #[cfg(unix)]
            if mode == "hook" {
                use std::os::unix::fs::PermissionsExt;
                fs::create_dir_all(server.join("hooks")).unwrap();
                let hook = server.join("hooks/pre-receive");
                fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
                fs::set_permissions(hook, fs::Permissions::from_mode(0o700)).unwrap();
            }
            let pack = run(
                &local,
                &["pack-objects", "--stdout", "--revs"],
                format!("{new}\n").as_bytes(),
            );
            assert!(pack.status.success());
            let mut packed = pack.stdout;
            if mode == "corrupt" {
                let end = packed.len() - 1;
                packed[end] ^= 1;
            }
            let body = request(
                &expected,
                &new,
                "refs/heads/main",
                &["report-status".into()],
                &packed,
            )
            .unwrap();
            let received = run(
                d.path(),
                &["receive-pack", "--stateless-rpc", server.to_str().unwrap()],
                &body,
            );
            let outcome = report(&received.stdout, "refs/heads/main").unwrap();
            if matches!(mode, "create" | "update") {
                assert_eq!(outcome, Report::Accepted);
                // A lost acknowledgement after publication is uncertain, not a
                // rejection. Automatic resend must not be inferred from EOF.
                for end in 0..received.stdout.len() {
                    assert_eq!(
                        report(&received.stdout[..end], "refs/heads/main").unwrap_err(),
                        "PUSH_OUTCOME_UNKNOWN"
                    );
                }

                assert_eq!(git(&server, &["rev-parse", "refs/heads/main"]).trim(), new);
            } else {
                assert!(matches!(outcome, Report::Rejected(_)));
                assert_eq!(git(&server, &["rev-parse", "refs/heads/main"]).trim(), old);
            }
            git(&server, &["fsck", "--strict"]);
        }
    }
    #[test]
    fn command_requires_report_status_valid_branch_and_nonzero_new_commit() {
        let pack = [b"PACK".as_slice(), &[0; 28]].concat();
        let old = "0".repeat(40);
        let new = "1".repeat(40);
        assert!(request(&old, &new, "refs/heads/main", &[], &pack).is_err());
        for r in ["refs/tags/v1", "refs/heads/a\nb", "refs/heads/a.lock"] {
            assert!(request(&old, &new, r, &["report-status".into()], &pack).is_err());
        }
        assert!(request(
            &old,
            &old,
            "refs/heads/main",
            &["report-status".into()],
            &pack
        )
        .is_err());
    }
}
