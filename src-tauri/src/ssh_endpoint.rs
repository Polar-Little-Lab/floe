//! SSH destination parsing only. No DNS, process launch, trust grant or authentication.
//! URI paths are absolute; scp-style paths retain relative/absolute semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Endpoint {
    pub user: Option<String>,
    pub host: String,
    pub port: u16,
    pub path: String,
}
fn invalid() -> String {
    "INVALID_SSH_REMOTE".into()
}
fn decode(value: &str) -> Result<String, String> {
    let mut output = Vec::with_capacity(value.len());
    let mut bytes = value.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let a = bytes
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or_else(invalid)?;
            let c = bytes
                .next()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or_else(invalid)?;
            output.push((a * 16 + c) as u8);
        } else {
            output.push(b);
        }
    }
    String::from_utf8(output).map_err(|_| invalid())
}
fn host(value: &str) -> Result<String, String> {
    if value.starts_with('[') && value.ends_with(']') {
        return value[1..value.len() - 1]
            .parse::<std::net::Ipv6Addr>()
            .map(|ip| ip.to_string())
            .map_err(|_| invalid());
    }
    if value.is_empty()
        || value.len() > 253
        || !value.is_ascii()
        || value.ends_with('.')
        || value.split('.').any(|part| {
            part.is_empty()
                || part.len() > 63
                || part.starts_with('-')
                || part.ends_with('-')
                || !part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
        })
    {
        return Err(invalid());
    }
    // Do not let URL/DNS libraries reinterpret numeric shorthand as a different host.
    let parsed = tauri::Url::parse(&format!("https://{value}/")).map_err(|_| invalid())?;
    if parsed
        .host_str()
        .is_some_and(|h| h.parse::<std::net::Ipv4Addr>().is_ok() && h != value)
    {
        return Err(invalid());
    }
    Ok(value.to_ascii_lowercase())
}
pub(crate) fn parse(value: &str) -> Result<Endpoint, String> {
    if value.is_empty()
        || value.len() > 4096
        || value.chars().any(char::is_control)
        || value.contains(['\\', '?', '#'])
    {
        return Err(invalid());
    }
    let uri = value.starts_with("ssh://");
    let (authority, raw_path) = if uri {
        let rest = &value[6..];
        let (a, p) = rest.split_once('/').ok_or_else(invalid)?;
        (a, format!("/{p}"))
    } else {
        if value.contains("://") || value.starts_with('/') {
            return Err(invalid());
        }
        // A colon inside a bracketed IPv6 host is not the scp path separator.
        let mut bracket = false;
        let mut separator = None;
        for (i, c) in value.char_indices() {
            match c {
                '[' => {
                    if bracket {
                        return Err(invalid());
                    }
                    bracket = true;
                }
                ']' => bracket = false,
                ':' if !bracket => {
                    separator = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let index = separator.ok_or_else(invalid)?;
        (&value[..index], value[index + 1..].to_string())
    };
    let (user, authority) = match authority.split_once('@') {
        Some((u, h)) => {
            if u.is_empty()
                || u.len() > 128
                || u.starts_with('-')
                || !u
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
            {
                return Err(invalid());
            }
            (Some(u.to_string()), h)
        }
        None => (None, authority),
    };
    let (raw_host, port) = if uri {
        let split = if authority.starts_with('[') {
            authority.find(']').map(|i| i + 1).ok_or_else(invalid)?
        } else {
            authority.find(':').unwrap_or(authority.len())
        };
        let tail = &authority[split..];
        let port = if tail.is_empty() {
            22
        } else {
            let digits = tail.strip_prefix(':').ok_or_else(invalid)?;
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return Err(invalid());
            }
            digits.parse::<u16>().map_err(|_| invalid())?
        };
        (&authority[..split], port)
    } else {
        (authority, 22)
    };
    if port == 0 {
        return Err(invalid());
    }
    let host = host(raw_host)?;
    let path = if uri { decode(&raw_path)? } else { raw_path };
    if path.is_empty()
        || path == "/"
        || path.len() > 4096
        || path.chars().any(char::is_control)
        || path.contains('\\')
        || path.starts_with('-')
        || (!uri && path.starts_with(':'))
        || path.trim_start_matches('/').starts_with('~')
        || path.split('/').any(|p| matches!(p, "." | ".."))
    {
        return Err(invalid());
    }
    Ok(Endpoint {
        user,
        host,
        port,
        path,
    })
}
#[cfg(test)]
impl Endpoint {
    // Prototype for the fixed service command; never accepts an executable from config.
    pub(crate) fn upload_command(&self) -> String {
        format!("git-upload-pack '{}'", self.path.replace('\'', "'\\''"))
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uri_and_scp_keep_path_scope_and_canonical_host_port() {
        let relative = parse("git@EXAMPLE.invalid:team/repo.git").unwrap();
        assert_eq!(
            relative,
            Endpoint {
                user: Some("git".into()),
                host: "example.invalid".into(),
                port: 22,
                path: "team/repo.git".into()
            }
        );
        let absolute = parse("ssh://git@example.invalid:2222/team/repo.git").unwrap();
        assert_eq!(absolute.path, "/team/repo.git");
        assert_eq!(absolute.port, 2222);
        assert_ne!(relative, absolute);
        assert_eq!(
            parse("ssh://git@example.invalid/team/a%20b.git")
                .unwrap()
                .path,
            "/team/a b.git"
        );
        assert_eq!(
            parse("git@example.invalid:team/a%20b.git").unwrap().path,
            "team/a%20b.git"
        );
        assert_eq!(
            parse("ssh://git@[2001:0db8::1]:2222/r").unwrap().host,
            "2001:db8::1"
        );
        assert_eq!(parse("git@[2001:db8::1]:/r").unwrap().path, "/r");
        assert_eq!(parse("example.invalid:relative").unwrap().user, None);
    }
    #[test]
    fn reject_secrets_ambiguous_hosts_options_and_control_escapes() {
        for value in [
            "ssh://user:secret@example.invalid/r",
            "ssh://@host/r",
            "ssh://host:0/r",
            "ssh://host:65536/r",
            "ssh://host:/r",
            "ssh://host/r%",
            "ssh://host/r%ff",
            "ssh://host/%00r",
            "ssh://host/%0ar",
            "ssh://host/r?token=secret",
            "ssh://host/r#secret",
            "ssh://host/a/../r",
            "ssh://host/%2e%2e/r",
            "ssh://127.1/r",
            "ssh://0x7f000001/r",
            "ssh://2130706433/r",
            "ssh://host./r",
            "ssh://-oProxyCommand=evil/r",
            "-u@host:r",
            "git@host:-option",
            "ssh://host/~user/r",
            "git@host:~user/r",
            "ext::evil",
            "/tmp/local:r",
            "git@host:",
            "ssh://host/",
            "ssh://host/r\\n",
            "ssh://host/r\\evil",
            "ssh://[fe80::1%lo0]/r",
            "git@@host:r",
        ] {
            let e = parse(value).unwrap_err();
            assert_eq!(e, "INVALID_SSH_REMOTE");
            assert!(!e.contains("secret"));
        }
    }
    #[test]
    #[cfg(unix)]
    fn fixed_service_quote_keeps_shell_metacharacters_in_one_path_argument() {
        let dir = tempfile::tempdir().unwrap();
        let helper = dir.path().join("git-upload-pack");
        std::fs::write(&helper, "#!/bin/sh\nprintf '%s\\0' \"$@\"\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        for path in [
            "team/a b.git",
            "team/a'b.git",
            "team/$(/usr/bin/touch SENTINEL).git",
            "team/`/usr/bin/touch SENTINEL`.git",
            "team/a;/usr/bin/touch SENTINEL",
            "team/a%20b",
        ] {
            let endpoint = parse(&format!("git@example.invalid:{path}")).unwrap();
            let out = std::process::Command::new("/bin/sh")
                .args(["-c", &endpoint.upload_command()])
                .env_clear()
                .env("PATH", dir.path())
                .current_dir(dir.path())
                .output()
                .unwrap();
            assert!(out.status.success());
            assert_eq!(out.stdout, [path.as_bytes(), b"\0"].concat());
            assert!(!dir.path().join("SENTINEL").exists());
        }
    }
    #[test]
    #[cfg(unix)]
    fn parsed_targets_match_actual_git_ssh_invocation_without_network() {
        let dir = tempfile::tempdir().unwrap();
        let helper = dir.path().join("ssh-probe");
        let record = dir.path().join("args");
        std::fs::write(
            &helper,
            "#!/bin/sh\nprintf '%s\\0' \"$@\" > \"$BRANCHDESK_SSH_ARGS\"\nexit 1\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&helper, std::fs::Permissions::from_mode(0o700)).unwrap();
        for value in [
            "git@example.invalid:team/repo.git",
            "ssh://git@example.invalid:2222/team/repo.git",
            "ssh://git@example.invalid/team/a%20b.git",
            "git@example.invalid:team/a'b.git",
            "git@[2001:db8::1]:/r",
            "ssh://git@[2001:db8::1]:2222/r",
            "ssh://example.invalid/r",
            "example.invalid:team/repo",
            "git@example.invalid:team/$(echo BAD).git",
        ] {
            let endpoint = parse(value).unwrap();
            let out = std::process::Command::new("git")
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", dir.path())
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_SSH", &helper)
                .env("GIT_SSH_VARIANT", "ssh")
                .env("BRANCHDESK_SSH_ARGS", &record)
                .current_dir(dir.path())
                .args(["-c", "protocol.version=0", "ls-remote", value])
                .output()
                .unwrap();
            assert!(!out.status.success());
            let bytes = std::fs::read(&record).unwrap();
            let args = bytes
                .split(|b| *b == 0)
                .filter(|b| !b.is_empty())
                .map(|b| std::str::from_utf8(b).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                args.last().copied(),
                Some(endpoint.upload_command().as_str()),
                "{value}: {args:?}"
            );
            let destination = args[args.len() - 2];
            let expected = match endpoint.user {
                Some(user) => format!("{user}@{}", endpoint.host),
                None => endpoint.host.clone(),
            };
            assert_eq!(destination, expected, "{value}");
            if endpoint.port != 22 {
                assert!(args
                    .windows(2)
                    .any(|w| w == ["-p", endpoint.port.to_string().as_str()]));
            }
        }
    }
}
