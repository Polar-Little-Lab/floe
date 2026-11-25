//! Isolated HTTPS worker with explicit endpoint-bound optional credentials.
// Protocol engine is exercised against synthetic receive-pack before enabling remote writes.
mod artifact;
pub(crate) mod auth;
mod object_import;
mod pack;
mod pack_pin;
pub(crate) mod push_pack;
mod push_transport;
mod push_wire;
mod selection;
pub(crate) use pack_pin::PackPin;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
const INPUT_LIMIT: usize = 64 * 1024;
const OUTPUT_LIMIT: u64 = 4 * 1024 * 1024;
const WORKER_ARG: &str = "--branchdesk-remote-worker";
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    url: String,
    #[serde(default)]
    download: bool,
    #[serde(default)]
    push_preview: bool,
    #[serde(default)]
    push: Option<PushRequest>,
    #[serde(default)]
    credential: Option<auth::TransportCredential>,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PushRequest {
    old: String,
    new: String,
    reference: String,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(
    tag = "status",
    content = "detail",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum PushOutcome {
    Accepted,
    Rejected(String),
    BeforeSend(String),
    Unknown,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Advertisement {
    pub endpoint: String,
    pub refs: Vec<RemoteRef>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ReceiveAdvertisement {
    pub advertisement: Advertisement,
    pub capabilities: Vec<String>,
}
impl ReceiveAdvertisement {
    pub fn supports_report_status(&self) -> bool {
        self.capabilities.iter().any(|c| c == "report-status")
    }
}
fn valid_capabilities(values: &[String]) -> bool {
    let mut seen = std::collections::HashSet::new();
    values.len() <= 256
        && values.iter().map(String::len).sum::<usize>() <= 16384
        && values.iter().all(|v| {
            !v.is_empty()
                && v.len() <= 1024
                && !v.chars().any(|c| c.is_control() || c.is_whitespace())
                && seen.insert(v)
        })
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteRef {
    pub name: String,
    pub oid: String,
    pub symbolic_target: Option<String>,
}
#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
enum Response {
    Download { value: pack::Download },
    Success { value: Advertisement },
    PushPreview { value: ReceiveAdvertisement },
    PushResult { value: PushOutcome },
    Failure { code: String },
}
pub(crate) fn valid_advertisement(value: &Advertisement, url: &str) -> bool {
    artifact::valid_advertisement(value, url)
}
fn client_builder() -> reqwest::blocking::ClientBuilder {
    reqwest::blocking::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .no_proxy()
        .referer(false)
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(12))
}
fn inspect_with_client(
    url: &str,
    client: &reqwest::blocking::Client,
) -> Result<Advertisement, String> {
    inspect_service(url, client, false)
}
fn inspect_service(
    url: &str,
    client: &reqwest::blocking::Client,
    push: bool,
) -> Result<Advertisement, String> {
    Ok(inspect_service_details(url, client, push)?.advertisement)
}
fn inspect_service_details(
    url: &str,
    client: &reqwest::blocking::Client,
    push: bool,
) -> Result<ReceiveAdvertisement, String> {
    let service = if push {
        "git-receive-pack"
    } else {
        "git-upload-pack"
    };
    let media = format!("application/x-{service}-advertisement");
    let endpoint = crate::repository::remote_endpoint::https(url)?;
    let request_url = format!(
        "{}/info/refs?service={service}",
        endpoint.as_str().trim_end_matches('/')
    );
    let response = client
        .get(request_url)
        .header("Accept", &media)
        .send()
        .map_err(|error| {
            if error.is_timeout() {
                "REMOTE_TIMEOUT"
            } else {
                "REMOTE_CONNECTION_FAILED"
            }
        })?;
    if response.status().is_redirection() {
        return Err("REMOTE_REDIRECT_REJECTED".into());
    }
    if response.status().as_u16() == 401 || response.status().as_u16() == 403 {
        return Err("REMOTE_AUTH_REQUIRED".into());
    }
    if !response.status().is_success() {
        return Err("REMOTE_CONNECTION_FAILED".into());
    }
    if response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or("").trim())
        != Some(media.as_str())
    {
        return Err("REMOTE_ADVERTISEMENT_INVALID".into());
    }
    if response
        .content_length()
        .is_some_and(|length| length > OUTPUT_LIMIT)
    {
        return Err("REMOTE_OUTPUT_LIMIT".into());
    }
    let mut bytes = Vec::new();
    response
        .take(OUTPUT_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "REMOTE_CONNECTION_FAILED")?;
    if bytes.len() as u64 > OUTPUT_LIMIT {
        return Err("REMOTE_OUTPUT_LIMIT".into());
    }
    parse_service_details(endpoint.as_str(), &bytes, push)
}
#[cfg(test)]
fn parse_advertisement(endpoint: &str, bytes: &[u8]) -> Result<Advertisement, String> {
    parse_service_advertisement(endpoint, bytes, false)
}
#[cfg(test)]
fn parse_service_advertisement(
    endpoint: &str,
    bytes: &[u8],
    push: bool,
) -> Result<Advertisement, String> {
    Ok(parse_service_details(endpoint, bytes, push)?.advertisement)
}
fn parse_service_details(
    endpoint: &str,
    bytes: &[u8],
    push: bool,
) -> Result<ReceiveAdvertisement, String> {
    fn packet<'a>(remaining: &mut &'a [u8]) -> Result<Option<&'a [u8]>, String> {
        if remaining.len() < 4 {
            return Err("REMOTE_ADVERTISEMENT_INVALID".into());
        }
        let prefix =
            std::str::from_utf8(&remaining[..4]).map_err(|_| "REMOTE_ADVERTISEMENT_INVALID")?;
        if !prefix.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("REMOTE_ADVERTISEMENT_INVALID".into());
        }
        let length =
            usize::from_str_radix(prefix, 16).map_err(|_| "REMOTE_ADVERTISEMENT_INVALID")?;
        *remaining = &remaining[4..];
        if length == 0 {
            return Ok(None);
        }
        if !(4..=65520).contains(&length) || length - 4 > remaining.len() {
            return Err("REMOTE_ADVERTISEMENT_INVALID".into());
        }
        let value = &remaining[..length - 4];
        *remaining = &remaining[length - 4..];
        Ok(Some(value))
    }
    fn valid_name(name: &str) -> bool {
        let plain = name.strip_suffix("^{}").unwrap_or(name);
        name.len() <= 1024
            && !name.chars().any(char::is_control)
            && (plain == "HEAD"
                || (plain.starts_with("refs/") && git2::Reference::is_valid_name(plain)))
    }
    let mut remaining = bytes;
    if packet(&mut remaining)?.map(|line| line.strip_suffix(b"\n").unwrap_or(line))
        != Some(if push {
            b"# service=git-receive-pack".as_slice()
        } else {
            b"# service=git-upload-pack".as_slice()
        })
        || packet(&mut remaining)?.is_some()
    {
        return Err("REMOTE_ADVERTISEMENT_INVALID".into());
    }
    let mut refs = Vec::new();
    let mut symbolic = Vec::new();
    let mut retained_capabilities = Vec::new();
    let mut first = true;
    let mut names = std::collections::HashSet::new();
    while let Some(line) = packet(&mut remaining)? {
        let line = std::str::from_utf8(line).map_err(|_| "INVALID_REMOTE_REF")?;
        let line = line.strip_suffix('\n').unwrap_or(line);
        let (reference, capabilities) = line
            .split_once('\0')
            .map_or((line, None), |(r, c)| (r, Some(c)));
        if let Some(capabilities) = capabilities {
            if !first {
                return Err("REMOTE_ADVERTISEMENT_INVALID".into());
            }
            retained_capabilities = capabilities
                .split(' ')
                .filter(|c| !c.is_empty())
                .map(str::to_owned)
                .collect();
            if !valid_capabilities(&retained_capabilities) {
                return Err("REMOTE_ADVERTISEMENT_INVALID".into());
            }
            for capability in &retained_capabilities {
                if let Some(value) = capability.strip_prefix("symref=") {
                    let (name, target) = value.split_once(':').ok_or("INVALID_REMOTE_REF")?;
                    if !valid_name(name) || !valid_name(target) {
                        return Err("INVALID_REMOTE_REF".into());
                    }
                    symbolic.push((name.to_owned(), target.to_owned()));
                }
                if capability.starts_with("object-format=") && capability != "object-format=sha1" {
                    return Err("REMOTE_OBJECT_FORMAT_UNSUPPORTED".into());
                }
            }
        }
        let is_first = first;
        first = false;
        let (oid, name) = reference.split_once(' ').ok_or("INVALID_REMOTE_REF")?;
        if oid.len() != 40 || !oid.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("INVALID_REMOTE_REF".into());
        }
        if oid == "0000000000000000000000000000000000000000" {
            if is_first && name == "capabilities^{}" && remaining == b"0000" {
                continue;
            }
            return Err("INVALID_REMOTE_REF".into());
        }
        if !valid_name(name) || !names.insert(name.to_owned()) {
            return Err("INVALID_REMOTE_REF".into());
        }
        refs.push(RemoteRef {
            name: name.into(),
            oid: oid.to_ascii_lowercase(),
            symbolic_target: None,
        });
        if refs.len() > 2000 {
            return Err("REMOTE_REF_LIMIT".into());
        }
    }
    if !remaining.is_empty() {
        return Err("REMOTE_ADVERTISEMENT_INVALID".into());
    }
    for reference in &mut refs {
        reference.symbolic_target = symbolic
            .iter()
            .find(|(name, _)| name == &reference.name)
            .map(|(_, target)| target.clone());
    }
    Ok(ReceiveAdvertisement {
        advertisement: Advertisement {
            endpoint: endpoint.into(),
            refs,
        },
        capabilities: retained_capabilities,
    })
}
// Authenticated server errors may reflect the Authorization value. Do not
// persist that server-controlled text in a Push journal or renderer error.
fn push_rejection_detail(reason: String, authenticated: bool) -> String {
    if authenticated {
        "REMOTE_REJECTED_AUTHENTICATED_PUSH".into()
    } else {
        reason
    }
}
fn execute_worker() -> i32 {
    let outcome = std::panic::catch_unwind(|| -> Result<Response, String> {
        let mut bytes = zeroize::Zeroizing::new(Vec::with_capacity(INPUT_LIMIT + 1024));
        std::io::stdin()
            .take((INPUT_LIMIT + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| "REMOTE_INPUT_FAILED")?;
        if bytes.len() > INPUT_LIMIT {
            return Err("REMOTE_INPUT_LIMIT".into());
        }
        let mut request: Request =
            serde_json::from_slice(&bytes).map_err(|_| "REMOTE_INPUT_INVALID")?;
        if (request.download as u8 + request.push_preview as u8 + request.push.is_some() as u8) > 1
        {
            return Err("REMOTE_INPUT_INVALID".into());
        }
        let authenticated = request.credential.is_some();
        let client = auth::client_builder(&request.url, request.credential.as_ref())?
            .build()
            .map_err(|_| "REMOTE_SETUP_FAILED")?;
        drop(bytes);
        drop(request.credential.take());
        if let Some(push) = request.push {
            let work = std::env::current_dir().map_err(|_| "REMOTE_QUARANTINE_FAILED")?;
            let result = (|| {
                let prepared = push_pack::PreparedPack::load_staged(&work.join("outbound"))
                    .map_err(push_transport::Failure::BeforeSend)?;
                push_transport::transfer(
                    &client,
                    &request.url,
                    &push.old,
                    &push.new,
                    &push.reference,
                    &prepared,
                )
            })();
            let value = match result {
                Ok(push_wire::Report::Accepted) => PushOutcome::Accepted,
                Ok(push_wire::Report::Rejected(reason)) => {
                    PushOutcome::Rejected(push_rejection_detail(reason, authenticated))
                }
                Err(push_transport::Failure::BeforeSend(code)) => PushOutcome::BeforeSend(code),
                Err(push_transport::Failure::OutcomeUnknown) => PushOutcome::Unknown,
            };
            Ok(Response::PushResult { value })
        } else if request.push_preview {
            Ok(Response::PushPreview {
                value: inspect_service_details(&request.url, &client, true)?,
            })
        } else if request.download {
            let directory = std::env::current_dir()
                .map_err(|_| "REMOTE_QUARANTINE_FAILED")?
                .join("quarantine.git");
            Ok(Response::Download {
                value: pack::download(&request.url, &client, &directory)?,
            })
        } else {
            Ok(Response::Success {
                value: inspect_with_client(&request.url, &client)?,
            })
        }
    });
    let response = match outcome {
        Ok(Ok(value)) => value,
        Ok(Err(code)) => Response::Failure { code },
        Err(_) => Response::Failure {
            code: "REMOTE_WORKER_PANIC".into(),
        },
    };
    let Ok(bytes) = serde_json::to_vec(&response) else {
        return 2;
    };
    if bytes.len() as u64 > OUTPUT_LIMIT {
        return 3;
    }
    if std::io::stdout().write_all(&bytes).is_err() {
        return 4;
    }
    0
}
pub fn entry() -> Option<i32> {
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new(WORKER_ARG)) {
        Some(execute_worker())
    } else {
        None
    }
}
struct Job(Child);
impl Drop for Job {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Completed {
    response: Response,
    _private: tempfile::TempDir,
}
#[cfg(test)]
fn supervise(command: Command, url: &str, timeout: Duration) -> Result<Advertisement, String> {
    match supervise_job(command, url, timeout, false)?.response {
        Response::Success { value } => Ok(value),
        _ => Err("REMOTE_OUTPUT_INVALID".into()),
    }
}
#[cfg(test)]
fn supervise_job(
    command: Command,
    url: &str,
    timeout: Duration,
    download: bool,
) -> Result<Completed, String> {
    supervise_job_mode(command, url, timeout, download, false)
}
#[cfg(test)]
fn supervise_job_mode(
    command: Command,
    url: &str,
    timeout: Duration,
    download: bool,
    push_preview: bool,
) -> Result<Completed, String> {
    supervise_job_request(command, url, timeout, download, push_preview, None)
}
#[cfg(test)]
fn supervise_job_request(
    command: Command,
    url: &str,
    timeout: Duration,
    download: bool,
    push_preview: bool,
    push: Option<(&PushRequest, &push_pack::PreparedPack)>,
) -> Result<Completed, String> {
    supervise_job_authenticated(command, url, timeout, download, push_preview, push, None)
}
fn supervise_job_authenticated(
    mut command: Command,
    url: &str,
    timeout: Duration,
    download: bool,
    push_preview: bool,
    push: Option<(&PushRequest, &push_pack::PreparedPack)>,
    credential: Option<&auth::TransportCredential>,
) -> Result<Completed, String> {
    if let Some(value) = credential {
        value.credential(url)?;
    }
    // Fixed-size serialization avoids reallocating a partially written secret.
    let mut payload = zeroize::Zeroizing::new(vec![0u8; INPUT_LIMIT]);
    let length = {
        let mut cursor = std::io::Cursor::new(payload.as_mut_slice());
        serde_json::to_writer(
            &mut cursor,
            &Request {
                url: url.into(),
                download,
                push_preview,
                push: push.map(|(request, _)| request.clone()),
                credential: credential.cloned(),
            },
        )
        .map_err(|_| "REMOTE_INPUT_LIMIT")?;
        cursor.position() as usize
    };
    payload.truncate(length);
    let private = tempfile::tempdir().map_err(|_| "REMOTE_TEMP_FAILED")?;
    if let Some((_, pack)) = push {
        pack.stage(&private.path().join("outbound"))?;
    }
    command
        .env_clear()
        .current_dir(private.path())
        .env("HOME", private.path())
        .env("TMPDIR", private.path())
        .env("TEMP", private.path())
        .env("TMP", private.path())
        .env("XDG_CONFIG_HOME", private.path())
        .env("GIT_CONFIG_NOSYSTEM", "1");
    #[cfg(windows)]
    if let Some(system) = std::env::var_os("SystemRoot") {
        command.env("SystemRoot", system);
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = Job(command.spawn().map_err(|_| "REMOTE_WORKER_START_FAILED")?);
    let deadline = Instant::now() + timeout;
    let mut input = child.0.stdin.take().ok_or("REMOTE_INPUT_FAILED")?;
    let (input_sender, input_receiver) = std::sync::mpsc::sync_channel(1);
    let writer = std::thread::spawn(move || {
        let result = input.write_all(&payload);
        drop(input);
        drop(payload);
        let _ = input_sender.send(result);
    });
    let output = child.0.stdout.take().ok_or("REMOTE_OUTPUT_FAILED")?;
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = output
            .take(OUTPUT_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    let result = (|| {
        let mut bytes = None;
        // Input writing participates in the same timeout as child execution.
        // A child that stops reading must not block the UI operation forever.
        let mut input_complete = false;
        loop {
            if Instant::now() >= deadline {
                return Err("REMOTE_TIMEOUT".into());
            }
            if !input_complete {
                match input_receiver.try_recv() {
                    Ok(Ok(())) => input_complete = true,
                    Ok(Err(_)) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        // Preserve crash classification for an already exited worker.
                        if child
                            .0
                            .try_wait()
                            .ok()
                            .flatten()
                            .is_some_and(|s| !s.success())
                        {
                            return Err("REMOTE_WORKER_FAILED".into());
                        }
                        return Err("REMOTE_INPUT_FAILED".into());
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                }
            }
            match receiver.try_recv() {
                Ok(Ok(value)) => {
                    if value.len() as u64 > OUTPUT_LIMIT {
                        return Err("REMOTE_OUTPUT_LIMIT".into());
                    }
                    bytes = Some(value);
                }
                Ok(Err(_)) => return Err("REMOTE_OUTPUT_FAILED".into()),
                Err(std::sync::mpsc::TryRecvError::Disconnected) if bytes.is_none() => {
                    return Err("REMOTE_OUTPUT_FAILED".into())
                }
                Err(_) => {}
            }
            if let Some(status) = child.0.try_wait().map_err(|_| "REMOTE_WORKER_FAILED")? {
                if !status.success() {
                    return Err("REMOTE_WORKER_FAILED".into());
                }
                if input_complete && bytes.is_some() {
                    let bytes = bytes.take().unwrap();
                    let response: Response =
                        serde_json::from_slice(&bytes).map_err(|_| "REMOTE_OUTPUT_INVALID")?;
                    return match response {
                        Response::Success { value }
                            if !download
                                && !push_preview
                                && push.is_none()
                                && artifact::valid_advertisement(&value, url) =>
                        {
                            Ok(Response::Success { value })
                        }
                        Response::PushPreview { value }
                            if push_preview
                                && push.is_none()
                                && !download
                                && artifact::valid_advertisement(&value.advertisement, url)
                                && valid_capabilities(&value.capabilities) =>
                        {
                            Ok(Response::PushPreview { value })
                        }
                        Response::PushResult { value }
                            if push.is_some() && !download && !push_preview =>
                        {
                            match &value {
                                PushOutcome::Rejected(s) | PushOutcome::BeforeSend(s)
                                    if s.is_empty()
                                        || s.len() > 4096
                                        || s.chars().any(char::is_control) =>
                                {
                                    return Err("REMOTE_OUTPUT_INVALID".into())
                                }
                                _ => {}
                            }
                            Ok(Response::PushResult { value })
                        }
                        Response::Download { value }
                            if download && !push_preview && push.is_none() =>
                        {
                            artifact::verify(private.path(), url, &value)?;
                            Ok(Response::Download { value })
                        }
                        Response::Success { .. }
                        | Response::Download { .. }
                        | Response::PushPreview { .. }
                        | Response::PushResult { .. } => Err("REMOTE_OUTPUT_INVALID".into()),
                        Response::Failure { code } => Err(match code.as_str() {
                            "REMOTE_AUTH_REQUIRED"
                            | "REMOTE_CERTIFICATE_REJECTED"
                            | "REMOTE_REDIRECT_REJECTED"
                            | "REMOTE_TIMEOUT"
                            | "REMOTE_OUTPUT_LIMIT"
                            | "REMOTE_ADVERTISEMENT_INVALID"
                            | "REMOTE_OBJECT_FORMAT_UNSUPPORTED"
                            | "REMOTE_REF_LIMIT"
                            | "INVALID_REMOTE_REF"
                            | "REMOTE_OBJECT_IMPORT_FAILED"
                            | "REMOTE_PACK_LIMIT"
                            | "REMOTE_PACK_INVALID"
                            | "REMOTE_OBJECT_LIMIT"
                            | "REMOTE_OBJECT_SIZE_LIMIT"
                            | "REMOTE_PACK_MISSING_OBJECT"
                            | "REMOTE_BRANCH_NOT_COMMIT"
                            | "REMOTE_QUARANTINE_FAILED" => code,
                            _ => "REMOTE_CONNECTION_FAILED".into(),
                        }),
                    };
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    drop(child);
    let _ = reader.join();
    let _ = writer.join();
    result.map(|response| Completed {
        response,
        _private: private,
    })
}
pub struct Downloaded {
    completed: Completed,
}
impl Downloaded {
    pub(crate) fn object_repository(&self) -> Result<Option<git2::Repository>, String> {
        let Response::Download { value } = &self.completed.response else {
            return Err("REMOTE_OUTPUT_INVALID".into());
        };
        artifact::verify(
            self.completed._private.path(),
            &value.advertisement.endpoint,
            value,
        )?;
        if value.pack_hash.is_none() {
            return Ok(None);
        }
        let repo =
            git2::Repository::open_bare(self.completed._private.path().join("quarantine.git"))
                .map_err(|_| "REMOTE_PACK_INVALID")?;
        repo.set_config(&git2::Config::new().map_err(|_| "REMOTE_PACK_INVALID")?)
            .map_err(|_| "REMOTE_PACK_INVALID")?;
        Ok(Some(repo))
    }
    pub(crate) fn pack_hash(&self) -> Option<&str> {
        match &self.completed.response {
            Response::Download { value } => value.pack_hash.as_deref(),
            _ => None,
        }
    }
    pub(crate) fn pin(&self, git_dir: &std::path::Path) -> Result<Option<PackPin>, String> {
        let Response::Download { value } = &self.completed.response else {
            return Err("REMOTE_OUTPUT_INVALID".into());
        };
        artifact::verify(
            self.completed._private.path(),
            &value.advertisement.endpoint,
            value,
        )?;
        value
            .pack_hash
            .as_deref()
            .map(|hash| PackPin::acquire(git_dir, hash))
            .transpose()
    }
    #[cfg(test)]
    pub(crate) fn fixture_binary(binary: &std::ffi::OsStr, url: &str) -> Result<Self, String> {
        let endpoint = crate::repository::remote_endpoint::https(url)?.to_string();
        let mut command = Command::new(binary);
        command.arg(WORKER_ARG);
        Ok(Self {
            completed: supervise_job_authenticated(
                command,
                &endpoint,
                Duration::from_secs(30),
                true,
                false,
                None,
                None,
            )?,
        })
    }
    #[cfg(test)]
    pub(crate) fn fixture(source: &git2::Repository, advertisement: Advertisement) -> Self {
        let private = tempfile::tempdir().unwrap();
        if advertisement.refs.is_empty() {
            return Self {
                completed: Completed {
                    response: Response::Download {
                        value: pack::Download {
                            advertisement,
                            pack_hash: None,
                            received_bytes: 0,
                            objects: 0,
                        },
                    },
                    _private: private,
                },
            };
        }
        let destination = private.path().join("quarantine.git");
        git2::Repository::init_bare(&destination).unwrap();
        let mut builder = source.packbuilder().unwrap();
        // insert_recursive includes trees, but does not walk commit ancestry.
        // Fixtures must satisfy the same complete graph invariant as the worker.
        let mut pending = advertisement
            .refs
            .iter()
            .map(|r| git2::Oid::from_str(&r.oid).unwrap())
            .collect::<Vec<_>>();
        let mut seen = std::collections::HashSet::new();
        while let Some(oid) = pending.pop() {
            if !seen.insert(oid) {
                continue;
            }
            builder.insert_recursive(oid, None).unwrap();
            let object = source.find_object(oid, None).unwrap();
            if let Some(commit) = object.as_commit() {
                pending.extend(commit.parent_ids());
            }
            if let Some(tag) = object.as_tag() {
                pending.push(tag.target_id());
            }
        }
        builder
            .write(&destination.join("objects/pack"), 0o600)
            .unwrap();
        let hash = builder.name().unwrap().unwrap().to_owned();
        let received_bytes =
            std::fs::metadata(destination.join(format!("objects/pack/pack-{hash}.pack")))
                .unwrap()
                .len();
        Self {
            completed: Completed {
                response: Response::Download {
                    value: pack::Download {
                        advertisement,
                        pack_hash: Some(hash),
                        received_bytes,
                        objects: builder.object_count(),
                    },
                },
                _private: private,
            },
        }
    }
    pub fn advertisement(&self) -> &Advertisement {
        match &self.completed.response {
            Response::Download { value } => &value.advertisement,
            _ => unreachable!(),
        }
    }
    pub fn import_into(&self, git_dir: &std::path::Path) -> Result<(), String> {
        let Response::Download { value } = &self.completed.response else {
            return Err("REMOTE_OUTPUT_INVALID".into());
        };
        artifact::verify(
            self.completed._private.path(),
            &value.advertisement.endpoint,
            value,
        )?;
        if let Some(hash) = &value.pack_hash {
            object_import::publish(
                &self.completed._private.path().join("quarantine.git"),
                git_dir,
                hash,
            )?;
        }
        Ok(())
    }
}
pub(crate) fn run_download_with_credential(
    url: &str,
    credential: Option<&auth::TransportCredential>,
) -> Result<Downloaded, String> {
    let endpoint = crate::repository::remote_endpoint::https(url)?.to_string();
    let mut command =
        Command::new(std::env::current_exe().map_err(|_| "REMOTE_WORKER_START_FAILED")?);
    command.arg(WORKER_ARG);
    Ok(Downloaded {
        completed: supervise_job_authenticated(
            command,
            &endpoint,
            Duration::from_secs(30),
            true,
            false,
            None,
            credential,
        )?,
    })
}
pub(crate) fn run_push_with_credential(
    url: &str,
    old: &str,
    new: &str,
    reference: &str,
    prepared: &push_pack::PreparedPack,
    credential: Option<&auth::TransportCredential>,
) -> PushOutcome {
    let endpoint = match crate::repository::remote_endpoint::https(url) {
        Ok(u) => u.to_string(),
        Err(e) => return PushOutcome::BeforeSend(e),
    };
    let mut command = match std::env::current_exe() {
        Ok(exe) => Command::new(exe),
        Err(_) => return PushOutcome::BeforeSend("REMOTE_WORKER_START_FAILED".into()),
    };
    command.arg(WORKER_ARG);
    let request = PushRequest {
        old: old.into(),
        new: new.into(),
        reference: reference.into(),
    };
    match supervise_job_authenticated(
        command,
        &endpoint,
        Duration::from_secs(40),
        false,
        false,
        Some((&request, prepared)),
        credential,
    ) {
        Ok(completed) => match completed.response {
            Response::PushResult { value } => value,
            _ => PushOutcome::Unknown,
        },
        Err(_) => PushOutcome::Unknown,
    }
}
pub(crate) fn run_push_preview_with_credential(
    url: &str,
    credential: Option<&auth::TransportCredential>,
) -> Result<ReceiveAdvertisement, String> {
    let endpoint = crate::repository::remote_endpoint::https(url)?.to_string();
    let mut command =
        Command::new(std::env::current_exe().map_err(|_| "REMOTE_WORKER_START_FAILED")?);
    command.arg(WORKER_ARG);
    match supervise_job_authenticated(
        command,
        &endpoint,
        Duration::from_secs(20),
        false,
        true,
        None,
        credential,
    )?
    .response
    {
        Response::PushPreview { value } => Ok(value),
        _ => Err("REMOTE_OUTPUT_INVALID".into()),
    }
}
pub(crate) fn run_with_credential(
    url: &str,
    credential: Option<&auth::TransportCredential>,
) -> Result<Advertisement, String> {
    let endpoint = crate::repository::remote_endpoint::https(url)?.to_string();
    let mut command =
        Command::new(std::env::current_exe().map_err(|_| "REMOTE_WORKER_START_FAILED")?);
    command.arg(WORKER_ARG);
    match supervise_job_authenticated(
        command,
        &endpoint,
        Duration::from_secs(20),
        false,
        false,
        None,
        credential,
    )?
    .response
    {
        Response::Success { value } => Ok(value),
        _ => Err("REMOTE_OUTPUT_INVALID".into()),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    const URL: &str = "https://example.com/repo.git";
    fn shell(script: &str) -> Command {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", &format!("cat >/dev/null; {script}")]);
        command
    }
    fn outbound_fixture() -> (tempfile::TempDir, push_pack::PreparedPack, String) {
        let source = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init_bare(source.path()).unwrap();
        let sig = git2::Signature::now("Worker fixture", "worker@invalid").unwrap();
        let tree = repo
            .find_tree(repo.treebuilder(None).unwrap().write().unwrap())
            .unwrap();
        let id = repo.commit(None, &sig, &sig, "root", &tree, &[]).unwrap();
        let pack = push_pack::prepare(&repo, id).unwrap();
        (source, pack, id.to_string())
    }
    #[test]
    fn push_supervisor_stages_owned_pack_and_accepts_only_transfer_response() {
        let (_source, pack, new) = outbound_fixture();
        let request = PushRequest {
            old: "0".repeat(40),
            new,
            reference: "refs/heads/main".into(),
        };
        let script="cat >/dev/null; test -f outbound/metadata.json || exit 4; printf '%s' '{\"kind\":\"pushResult\",\"value\":{\"status\":\"accepted\"}}'";
        let completed = supervise_job_request(
            shell(script),
            URL,
            Duration::from_secs(2),
            false,
            false,
            Some((&request, &pack)),
        )
        .unwrap();
        assert!(matches!(
            completed.response,
            Response::PushResult {
                value: PushOutcome::Accepted
            }
        ));
        let path = completed._private.path().to_path_buf();
        assert!(path.join("outbound/metadata.json").exists());
        drop(completed);
        assert!(!path.exists());
        assert!(supervise(shell(script), URL, Duration::from_secs(2)).is_err());
        let wrong="cat >/dev/null; printf '%s' '{\"kind\":\"success\",\"value\":{\"endpoint\":\"https://example.com/repo.git\",\"refs\":[]}}'";
        assert!(supervise_job_request(
            shell(wrong),
            URL,
            Duration::from_secs(2),
            false,
            false,
            Some((&request, &pack))
        )
        .is_err());
    }
    #[test]
    #[ignore = "requires explicitly selected current release worker; no network performed"]
    fn actual_release_push_worker_revalidates_staged_source_before_network() {
        let executable =
            std::env::var_os("BRANCHDESK_PUSH_WORKER_EXE").expect("select release executable");
        let (_source, pack, _) = outbound_fixture();
        let request = PushRequest {
            old: "0".repeat(40),
            new: "1".repeat(40),
            reference: "refs/heads/main".into(),
        };
        let mut command = Command::new(executable);
        command.arg(WORKER_ARG);
        let completed = supervise_job_request(
            command,
            URL,
            Duration::from_secs(10),
            false,
            false,
            Some((&request, &pack)),
        )
        .unwrap();
        assert!(
            matches!(completed.response,Response::PushResult {value:PushOutcome::BeforeSend(ref code)} if code=="PUSH_SOURCE_MISMATCH")
        );
    }
    #[test]
    fn push_supervisor_requires_push_envelope_and_valid_capabilities() {
        let value = ReceiveAdvertisement {
            advertisement: Advertisement {
                endpoint: URL.into(),
                refs: vec![],
            },
            capabilities: vec!["report-status".into()],
        };
        let good = serde_json::to_string(&Response::PushPreview {
            value: value.clone(),
        })
        .unwrap();
        let script = format!("cat >/dev/null; printf '%s' '{good}'");
        let result =
            supervise_job_mode(shell(&script), URL, Duration::from_secs(2), false, true).unwrap();
        assert!(
            matches!(result.response,Response::PushPreview {value} if value.supports_report_status())
        );
        assert!(supervise(shell(&script), URL, Duration::from_secs(2)).is_err());
        let fetch = serde_json::to_string(&Response::Success {
            value: value.advertisement.clone(),
        })
        .unwrap();
        assert!(supervise_job_mode(
            shell(&format!("cat >/dev/null; printf '%s' '{fetch}'")),
            URL,
            Duration::from_secs(2),
            false,
            true
        )
        .is_err());
        let bad = serde_json::to_string(&Response::PushPreview {
            value: ReceiveAdvertisement {
                capabilities: vec!["report-status".into(), "report-status".into()],
                ..value
            },
        })
        .unwrap();
        assert!(supervise_job_mode(
            shell(&format!("cat >/dev/null; printf '%s' '{bad}'")),
            URL,
            Duration::from_secs(2),
            false,
            true
        )
        .is_err());
    }
    #[test]
    fn authenticated_push_rejection_cannot_persist_reflected_secret() {
        let reflected = "synthetic-password Basic c3ludGhldGljOnRva2Vu";
        let response = Response::PushResult {
            value: PushOutcome::Rejected(push_rejection_detail(reflected.into(), true)),
        };
        let serialized = serde_json::to_string(&response).unwrap();
        assert!(!serialized.contains("synthetic-password"));
        assert!(!serialized.contains("c3ludGhldGlj"));
        assert!(serialized.contains("REMOTE_REJECTED_AUTHENTICATED_PUSH"));
        assert_eq!(
            push_rejection_detail("hook refused".into(), false),
            "hook refused"
        );
    }
    #[test]
    fn credentials_use_only_private_pipe_and_reject_mismatched_scope_before_spawn() {
        let fixture = tempfile::tempdir().unwrap();
        let script = fixture.path().join("worker.py");
        std::fs::write(
            &script,
            r#"import os,sys,json
request=json.load(sys.stdin)
secret='synthetic-private-pipe-token'
assert secret not in repr(sys.argv)
assert all(secret not in value for value in os.environ.values())
# macOS's Python launcher may create a Library directory under private HOME.
# Inspect actual bytes instead of assuming the working directory stays empty.
for root, _, files in os.walk('.'):
    for name in files:
        with open(os.path.join(root,name),'rb') as handle:
            content=handle.read(1024*1024+1)
        assert len(content)<=1024*1024
        assert secret.encode() not in content
        assert b'BDC1' not in content
record=bytes(request['credential']['record'])
assert record.endswith(secret.encode())
assert request['credential']['endpoint']==request['url']
print(json.dumps({'kind':'success','value':{'endpoint':request['url'],'refs':[]}}))
"#,
        )
        .unwrap();
        let value = crate::credential_vault::Credential::new(
            "synthetic".into(),
            "synthetic-private-pipe-token".into(),
        )
        .unwrap();
        let bound = auth::TransportCredential::new(URL, &value).unwrap();
        let mut command = Command::new("/usr/bin/python3");
        command.arg(&script);
        assert!(supervise_job_authenticated(
            command,
            URL,
            Duration::from_secs(3),
            false,
            false,
            None,
            Some(&bound)
        )
        .is_ok());
        let error = supervise_job_authenticated(
            Command::new("/nonexistent-worker"),
            "https://else.invalid/repo.git",
            Duration::from_secs(1),
            false,
            false,
            None,
            Some(&bound),
        )
        .err()
        .unwrap();
        assert_eq!(error, "REMOTE_CREDENTIAL_SCOPE_MISMATCH");
    }
    #[test]
    fn bounded_credential_input_cannot_bypass_worker_deadline() {
        let value =
            crate::credential_vault::Credential::new("u".repeat(1024), "é".repeat(4096)).unwrap();
        let bound = auth::TransportCredential::new(URL, &value).unwrap();
        let mut command = Command::new("/bin/sleep");
        command.arg("30");
        let start = Instant::now();
        let result = supervise_job_authenticated(
            command,
            URL,
            Duration::from_millis(150),
            false,
            false,
            None,
            Some(&bound),
        );
        assert_eq!(result.err().unwrap(), "REMOTE_TIMEOUT");
        assert!(start.elapsed() < Duration::from_secs(3));
        let oversized = "x".repeat(INPUT_LIMIT);
        let result = supervise_job_authenticated(
            Command::new("/nonexistent-worker"),
            &oversized,
            Duration::from_secs(1),
            false,
            false,
            None,
            None,
        );
        assert_eq!(result.err().unwrap(), "REMOTE_INPUT_LIMIT");
    }
    #[test]
    fn supervisor_accepts_only_matching_structured_response() {
        let valid = r#"printf '%s' '{"kind":"success","value":{"endpoint":"https://example.com/repo.git","refs":[]}}'"#;
        assert!(supervise(shell(valid), URL, Duration::from_secs(2))
            .unwrap()
            .refs
            .is_empty());
        assert_eq!(
            supervise(
                shell(&valid.replace("example.com", "other.example")),
                URL,
                Duration::from_secs(2)
            )
            .unwrap_err(),
            "REMOTE_OUTPUT_INVALID"
        );
        assert_eq!(
            supervise(shell("printf invalid"), URL, Duration::from_secs(2)).unwrap_err(),
            "REMOTE_OUTPUT_INVALID"
        );
    }
    #[test]
    fn supervisor_kills_timeout_and_rejects_crash() {
        let start = Instant::now();
        assert_eq!(
            supervise(shell("exec /bin/sleep 30"), URL, Duration::from_millis(100)).unwrap_err(),
            "REMOTE_TIMEOUT"
        );
        assert!(start.elapsed() < Duration::from_secs(3));
        assert_eq!(
            supervise(shell("exit 9"), URL, Duration::from_secs(2)).unwrap_err(),
            "REMOTE_WORKER_FAILED"
        );
    }
    #[test]
    fn supervisor_discards_inherited_environment_and_private_errors() {
        let mut command = shell(
            r#"if [ -n "$BRANCHDESK_TEST_SECRET" ]; then exit 9; fi; printf '%s' '{"kind":"failure","code":"secret raw server response"}'"#,
        );
        command.env("BRANCHDESK_TEST_SECRET", "secret");
        assert_eq!(
            supervise(command, URL, Duration::from_secs(2)).unwrap_err(),
            "REMOTE_CONNECTION_FAILED"
        );
    }
    #[test]
    fn supervisor_bounds_output() {
        let script = "chunk=xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; i=0; while [ $i -lt 17 ]; do chunk=$chunk$chunk; i=$((i+1)); done; printf '%s' \"$chunk\"";
        assert_eq!(
            supervise(shell(script), URL, Duration::from_secs(5)).unwrap_err(),
            "REMOTE_OUTPUT_LIMIT"
        );
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tls_tests {
    use super::*;
    struct Fixture {
        _job: Job,
        dir: tempfile::TempDir,
        url: String,
    }
    impl Fixture {
        fn new(mode: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            if mode.starts_with("pack") || mode.starts_with("roundtrip") {
                let repo = git2::Repository::init_bare(dir.path().join("source")).unwrap();
                let sig = git2::Signature::now("Fixture", "fixture@example.invalid").unwrap();
                for n in 0..2 {
                    let blob = repo.blob(format!("contents {n}\n").as_bytes()).unwrap();
                    let mut builder = repo.treebuilder(None).unwrap();
                    builder.insert("readme.txt", blob, 0o100644).unwrap();
                    let tree = repo.find_tree(builder.write().unwrap()).unwrap();
                    let parent = repo.head().ok().map(|head| head.peel_to_commit().unwrap());
                    let parents = parent.iter().collect::<Vec<_>>();
                    let oid = repo
                        .commit(Some("HEAD"), &sig, &sig, "fixture commit", &tree, &parents)
                        .unwrap();
                    if n == 0 {
                        repo.tag(
                            "v1",
                            &repo.find_object(oid, None).unwrap(),
                            &sig,
                            "fixture tag",
                            false,
                        )
                        .unwrap();
                    }
                }
            }
            if mode.starts_with("push") {
                git2::Repository::init_bare(dir.path().join("source")).unwrap();
            }
            let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../scripts/remote_tls_fixture.py");
            let mut job = Job(Command::new("/usr/bin/python3")
                .arg(script)
                .arg(dir.path())
                .arg(mode)
                .stdout(Stdio::null())
                .spawn()
                .unwrap());
            let deadline = Instant::now() + Duration::from_secs(15);
            let port = loop {
                if let Ok(port) = std::fs::read_to_string(dir.path().join("port")) {
                    if !port.is_empty() {
                        break port;
                    }
                }
                if let Some(status) = job.0.try_wait().unwrap() {
                    panic!("TLS fixture exited before becoming ready: {status}");
                }
                assert!(Instant::now() < deadline, "fixture startup timed out");
                std::thread::sleep(Duration::from_millis(10));
            };
            Self {
                _job: job,
                dir,
                url: format!("https://127.0.0.1:{port}/repo.git"),
            }
        }
        fn inspect(&self) -> Result<Advertisement, String> {
            let pem = std::fs::read(self.dir.path().join("cert.pem")).unwrap();
            let client = client_builder()
                .add_root_certificate(reqwest::Certificate::from_pem(&pem).unwrap())
                .build()
                .unwrap();
            inspect_with_client(&self.url, &client)
        }
        fn requests(&self) -> Vec<serde_json::Value> {
            std::fs::read_to_string(self.dir.path().join("requests.jsonl"))
                .unwrap_or_default()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect()
        }
    }
    #[test]
    fn controlled_tls_push_transfer_distinguishes_rejection_and_lost_ack_after_publication() {
        for mode in [
            "push",
            "push-drop",
            "push-truncate",
            "push-badtype",
            "push-redirect",
            "push-reject",
            "push-stale",
            "auth",
        ] {
            let fixture = Fixture::new(mode);
            let local = tempfile::tempdir().unwrap();
            let repo = git2::Repository::init_bare(local.path()).unwrap();
            let sig = git2::Signature::now("Push TLS", "tls@invalid").unwrap();
            let blob = repo.blob(b"only selected bytes").unwrap();
            let unrelated = repo.blob(b"unrelated").unwrap();
            let mut tree = repo.treebuilder(None).unwrap();
            tree.insert("file", blob, 0o100644).unwrap();
            let tree = repo.find_tree(tree.write().unwrap()).unwrap();
            let base = repo.commit(None, &sig, &sig, "base", &tree, &[]).unwrap();
            let tip = repo
                .commit(
                    Some("refs/heads/main"),
                    &sig,
                    &sig,
                    "tip",
                    &tree,
                    &[&repo.find_commit(base).unwrap()],
                )
                .unwrap();
            let prepared = push_pack::prepare(&repo, tip).unwrap();
            if mode == "push-reject" {
                use std::os::unix::fs::PermissionsExt;
                let hook = fixture.dir.path().join("source/hooks/pre-receive");
                std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
                std::fs::set_permissions(hook, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            let pem = std::fs::read(fixture.dir.path().join("cert.pem")).unwrap();
            let client = client_builder()
                .add_root_certificate(reqwest::Certificate::from_pem(&pem).unwrap())
                .build()
                .unwrap();
            let old = if mode == "push-stale" {
                base.to_string()
            } else {
                "0".repeat(40)
            };
            let result = push_transport::transfer(
                &client,
                &fixture.url,
                &old,
                &tip.to_string(),
                "refs/heads/main",
                &prepared,
            );
            match mode {
                "push" => assert_eq!(result.unwrap(), push_wire::Report::Accepted),
                "push-reject" => assert!(matches!(result, Ok(push_wire::Report::Rejected(_)))),
                "push-stale" => assert_eq!(
                    result.unwrap_err(),
                    push_transport::Failure::BeforeSend("PUSH_REMOTE_CHANGED".into())
                ),
                "auth" => assert_eq!(
                    result.unwrap_err(),
                    push_transport::Failure::BeforeSend("REMOTE_AUTH_REQUIRED".into())
                ),
                _ => assert_eq!(result.unwrap_err(), push_transport::Failure::OutcomeUnknown),
            }
            let requests = fixture.requests();
            let no_post = matches!(mode, "push-stale" | "auth");
            assert_eq!(
                requests.len(),
                if no_post { 1 } else { 2 },
                "{mode}: no automatic retry"
            );
            assert!(requests.iter().all(|r| r["authorization"].is_null()));
            if !no_post {
                assert_eq!(requests[1]["method"], "POST");
                assert_eq!(requests[1]["path"], "/repo.git/git-receive-pack");
            }
            if mode.starts_with("push") {
                let server =
                    git2::Repository::open_bare(fixture.dir.path().join("source")).unwrap();
                let published = matches!(
                    mode,
                    "push" | "push-drop" | "push-truncate" | "push-badtype"
                );
                assert_eq!(
                    server.refname_to_id("refs/heads/main").ok(),
                    if published { Some(tip) } else { None },
                    "{mode}"
                );
                assert!(server.find_blob(unrelated).is_err());
                if published {
                    assert_eq!(
                        server.find_blob(blob).unwrap().content(),
                        b"only selected bytes"
                    );
                }
            }
            assert_eq!(repo.refname_to_id("refs/heads/main").unwrap(), tip);
        }
    }
    #[test]
    fn controlled_tls_push_preview_uses_receive_pack_and_never_posts() {
        for mode in ["receive", "success", "auth", "redirect-path"] {
            let fixture = Fixture::new(mode);
            let pem = std::fs::read(fixture.dir.path().join("cert.pem")).unwrap();
            let client = client_builder()
                .add_root_certificate(reqwest::Certificate::from_pem(&pem).unwrap())
                .build()
                .unwrap();
            let result = inspect_service(&fixture.url, &client, true);
            match mode {
                "receive" => assert_eq!(result.unwrap().refs[0].name, "refs/heads/main"),
                "auth" => assert_eq!(result.unwrap_err(), "REMOTE_AUTH_REQUIRED"),
                "redirect-path" => assert_eq!(result.unwrap_err(), "REMOTE_REDIRECT_REJECTED"),
                _ => assert_eq!(result.unwrap_err(), "REMOTE_ADVERTISEMENT_INVALID"),
            }
            let requests = fixture.requests();
            assert_eq!(requests.len(), 1);
            assert_eq!(
                requests[0]["path"],
                "/repo.git/info/refs?service=git-receive-pack"
            );
            assert!(requests[0]["authorization"].is_null());
            assert!(requests[0]["method"].is_null());
        }
    }
    #[test]
    fn authenticated_clone_push_fetch_roundtrip_matches_git_and_preserves_worktree() {
        let fixture = Fixture::new("roundtrip-authenticated");
        let certificate = reqwest::Certificate::from_pem(&std::fs::read(fixture.dir.path().join("cert.pem")).unwrap()).unwrap();
        let anonymous = client_builder().add_root_certificate(certificate.clone()).build().unwrap();
        assert_eq!(inspect_with_client(&fixture.url, &anonymous).unwrap_err(), "REMOTE_AUTH_REQUIRED");
        let wrong = crate::credential_vault::Credential::new("synthetic".into(), "wrong".into()).unwrap();
        let wrong_bound = auth::TransportCredential::new(&fixture.url, &wrong).unwrap();
        let wrong_client = auth::client_builder(&fixture.url, Some(&wrong_bound)).unwrap().add_root_certificate(certificate.clone()).build().unwrap();
        assert_eq!(inspect_with_client(&fixture.url, &wrong_client).unwrap_err(), "REMOTE_AUTH_REQUIRED");
        let credential = crate::credential_vault::Credential::new("synthetic".into(), "token".into()).unwrap();
        let bound = auth::TransportCredential::new(&fixture.url, &credential).unwrap();
        let client = auth::client_builder(&fixture.url, Some(&bound)).unwrap().add_root_certificate(certificate).build().unwrap();
        let download = || {
            let private = tempfile::tempdir().unwrap();
            let value = pack::download(&fixture.url,&client,&private.path().join("quarantine.git")).unwrap();
            artifact::verify(private.path(),&fixture.url,&value).unwrap();
            Downloaded{completed:Completed{response:Response::Download{value},_private:private}}
        };
        let initial = download();
        let local = tempfile::tempdir().unwrap();
        let path = local.path().join("clone");
        let store: crate::Store = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let cloned = crate::clone_repository::create(&store,&path,&initial,"").unwrap();
        let old = cloned.head_oid.clone().unwrap();
        let pull_path = local.path().join("pull-clone");
        crate::clone_repository::create(&store,&pull_path,&initial,"").unwrap();
        let mut session = crate::repository::Session::new(path.clone()).unwrap();
        std::fs::write(path.join("local.txt"),"authenticated push contents\n").unwrap();
        let s = session.snapshot().unwrap();
        let s = session.mutate(&s.revision,crate::repository::Action::StageAll).unwrap();
        let s = session.mutate(&s.revision,crate::repository::Action::Commit{message:"auth roundtrip".into(),author:"Synthetic".into(),email:"test@example.invalid".into()}).unwrap();
        let tip = s.head_oid.unwrap();
        let repo = git2::Repository::open(&path).unwrap();
        let prepared = push_pack::prepare(&repo,git2::Oid::from_str(&tip).unwrap()).unwrap();
        let target = format!("refs/heads/{}",cloned.branch);
        assert!(inspect_service_details(&fixture.url,&client,true).unwrap().supports_report_status());
        assert_eq!(push_transport::transfer(&client,&fixture.url,&old,&tip,&target,&prepared).unwrap(),push_wire::Report::Accepted);
        let remote = git2::Repository::open_bare(fixture.dir.path().join("source")).unwrap();
        assert_eq!(remote.refname_to_id(&target).unwrap().to_string(),tip);
        std::fs::write(path.join("local.txt"),"keep unstaged edit\n").unwrap();
        let before = session.snapshot().unwrap();
        let index = std::fs::read(path.join(".git/index")).unwrap();
        let config = std::fs::read(path.join(".git/config")).unwrap();
        let fetched = download();
        let plan = crate::repository::fetch::preview_download(&mut session,&before.revision,"origin",&fetched).unwrap();
        assert!(!plan.mappings.is_empty());
        // Preview/cancel is read only; accepting the same frozen review updates tracking refs only.
        assert_eq!(repo.refname_to_id(&format!("refs/remotes/origin/{}",cloned.branch)).unwrap().to_string(),old);
        let after = crate::repository::fetch::execute(&mut session,&before.revision,"origin",&fetched).unwrap();
        assert_eq!(after.head_oid.as_deref(),Some(tip.as_str()));
        assert_eq!(repo.refname_to_id(&format!("refs/remotes/origin/{}",cloned.branch)).unwrap().to_string(),tip);
        assert_eq!(std::fs::read(path.join(".git/index")).unwrap(),index);
        assert_eq!(std::fs::read(path.join(".git/config")).unwrap(),config);
        assert_eq!(std::fs::read(path.join("local.txt")).unwrap(),b"keep unstaged edit\n");
        assert!(std::fs::read_to_string(path.join(".git/FETCH_HEAD")).unwrap().contains(&tip));
        let mut pull_session = crate::repository::Session::new(pull_path.clone()).unwrap();
        let pull_before = pull_session.snapshot().unwrap();
        let prepared_pull = crate::repository::pull::prepare(&mut pull_session,&pull_before.revision,"origin",&fetched).unwrap();
        assert_eq!(prepared_pull.snapshot.head_oid.as_deref(),Some(old.as_str()));
        let crate::repository::pull::Integration::Merge(plan) = prepared_pull.integration else { panic!("expected fast-forward merge"); };
        let pulled = pull_session.mutate(&prepared_pull.snapshot.revision,crate::repository::Action::FastForwardMerge{name:plan.target_branch}).unwrap();
        assert_eq!(pulled.head_oid.as_deref(),Some(tip.as_str()));
        assert!(pulled.changes.is_empty());
        assert_eq!(std::fs::read(pull_path.join("local.txt")).unwrap(),b"authenticated push contents\n");
        std::fs::write(pull_path.join("local.txt"),"dirty pull must survive\n").unwrap();
        let dirty = pull_session.snapshot().unwrap();
        assert!(crate::repository::pull::prepare(&mut pull_session,&dirty.revision,"origin",&fetched).unwrap_err().contains("PULL_DIRTY_WORKTREE"));
        assert_eq!(std::fs::read(pull_path.join("local.txt")).unwrap(),b"dirty pull must survive\n");
        for git_dir in [path.clone(),pull_path,fixture.dir.path().join("source")] {
            let result=Command::new(crate::test_git_executable()).arg("-C").arg(git_dir).args(["fsck","--strict"]).output().unwrap();
            assert!(result.status.success(),"{}",String::from_utf8_lossy(&result.stderr));
        }
        let requests=fixture.requests();
        assert!(requests.iter().any(|r|r["path"].as_str().unwrap().ends_with("git-receive-pack") && r["method"]=="POST"));
        assert!(requests[2..].iter().all(|r|r["authorization"]=="Basic c3ludGhldGljOnRva2Vu"));
    }
    #[test]
    fn authenticated_git_upload_and_receive_require_credentials_on_get_and_post() {
        for mode in ["pack-authenticated", "push-authenticated"] {
            let fixture = Fixture::new(mode);
            let pem = std::fs::read(fixture.dir.path().join("cert.pem")).unwrap();
            let certificate = reqwest::Certificate::from_pem(&pem).unwrap();
            let anonymous = client_builder()
                .add_root_certificate(certificate.clone())
                .build()
                .unwrap();
            assert_eq!(
                inspect_service(&fixture.url, &anonymous, mode.starts_with("push")).unwrap_err(),
                "REMOTE_AUTH_REQUIRED"
            );
            let credential =
                crate::credential_vault::Credential::new("synthetic".into(), "token".into())
                    .unwrap();
            let bound = auth::TransportCredential::new(&fixture.url, &credential).unwrap();
            let client = auth::client_builder(&fixture.url, Some(&bound))
                .unwrap()
                .add_root_certificate(certificate)
                .build()
                .unwrap();
            if mode.starts_with("pack") {
                let destination = fixture.dir.path().join("authenticated-quarantine.git");
                let downloaded = pack::download(&fixture.url, &client, &destination).unwrap();
                let repo = git2::Repository::open_bare(&destination).unwrap();
                assert!(repo
                    .find_commit(
                        git2::Oid::from_str(&downloaded.advertisement.refs[0].oid).unwrap()
                    )
                    .is_ok());
            } else {
                let local = tempfile::tempdir().unwrap();
                let repo = git2::Repository::init_bare(local.path()).unwrap();
                let tree = repo
                    .find_tree(repo.treebuilder(None).unwrap().write().unwrap())
                    .unwrap();
                let sig = git2::Signature::now("Synthetic", "synthetic@invalid").unwrap();
                let tip = repo
                    .commit(None, &sig, &sig, "authenticated root", &tree, &[])
                    .unwrap();
                let prepared = push_pack::prepare(&repo, tip).unwrap();
                assert_eq!(
                    push_transport::transfer(
                        &client,
                        &fixture.url,
                        &"0".repeat(40),
                        &tip.to_string(),
                        "refs/heads/main",
                        &prepared
                    )
                    .unwrap(),
                    push_wire::Report::Accepted
                );
                let remote =
                    git2::Repository::open_bare(fixture.dir.path().join("source")).unwrap();
                assert_eq!(remote.refname_to_id("refs/heads/main").unwrap(), tip);
            }
            let requests = fixture.requests();
            assert_eq!(requests.len(), 3);
            assert!(requests[0]["authorization"].is_null());
            assert!(requests[1..]
                .iter()
                .all(|r| r["authorization"] == "Basic c3ludGhldGljOnRva2Vu"));
            assert_eq!(requests[2]["method"], "POST");
        }
    }
    #[test]
    fn authenticated_tls_is_scoped_sensitive_and_never_follows_redirects_or_retries_auth() {
        use crate::credential_vault::Credential;
        for mode in [
            "success",
            "receive",
            "auth",
            "redirect-path",
            "redirect-host",
            "redirect-port",
            "redirect-http",
        ] {
            let fixture = Fixture::new(mode);
            let secret = Credential::new("synthetic".into(), "token".into()).unwrap();
            let header = secret.authorization().unwrap();
            assert!(header.is_sensitive());
            assert!(!format!("{header:?}").contains("c3ludGhldGlj"));
            let bound = auth::TransportCredential::new(&fixture.url, &secret).unwrap();
            assert!(auth::client_builder(&(fixture.url.clone() + "/other"), Some(&bound)).is_err());
            let pem = std::fs::read(fixture.dir.path().join("cert.pem")).unwrap();
            let client = auth::client_builder(&fixture.url, Some(&bound))
                .unwrap()
                .add_root_certificate(reqwest::Certificate::from_pem(&pem).unwrap())
                .build()
                .unwrap();
            let result = inspect_service(&fixture.url, &client, mode == "receive");
            match mode {
                "success" | "receive" => {
                    result.unwrap();
                }
                "auth" => assert_eq!(result.unwrap_err(), "REMOTE_AUTH_REQUIRED"),
                _ => assert_eq!(result.unwrap_err(), "REMOTE_REDIRECT_REJECTED"),
            }
            let requests = fixture.requests();
            assert_eq!(requests.len(), 1, "{mode}: must not retry/follow redirect");
            assert_eq!(
                requests[0]["authorization"].as_str(),
                Some("Basic c3ludGhldGljOnRva2Vu")
            );
        }
    }
    #[test]
    fn controlled_tls_advertisement_parses_refs_without_authorization() {
        let fixture = Fixture::new("success");
        let result = fixture.inspect().unwrap();
        assert_eq!(result.refs.len(), 2);
        assert_eq!(
            result.refs[0].symbolic_target.as_deref(),
            Some("refs/heads/main")
        );
        let requests = fixture.requests();
        assert_eq!(requests.len(), 1);
        assert!(requests[0]["authorization"].is_null());
    }
    #[test]
    fn controlled_tls_redirects_are_not_followed() {
        for mode in [
            "redirect-path",
            "redirect-host",
            "redirect-port",
            "redirect-http",
        ] {
            let fixture = Fixture::new(mode);
            assert_eq!(fixture.inspect().unwrap_err(), "REMOTE_REDIRECT_REJECTED");
            assert_eq!(
                fixture.requests().len(),
                1,
                "must not issue redirected HTTP request"
            );
        }
    }
    #[test]
    fn controlled_tls_rejects_oversized_body_and_stalled_response() {
        let fixture = Fixture::new("oversized");
        assert_eq!(fixture.inspect().unwrap_err(), "REMOTE_OUTPUT_LIMIT");
        let fixture = Fixture::new("slow");
        let pem = std::fs::read(fixture.dir.path().join("cert.pem")).unwrap();
        let client = client_builder()
            .add_root_certificate(reqwest::Certificate::from_pem(&pem).unwrap())
            .timeout(Duration::from_millis(150))
            .build()
            .unwrap();
        let start = Instant::now();
        assert_eq!(
            inspect_with_client(&fixture.url, &client).unwrap_err(),
            "REMOTE_TIMEOUT"
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn quarantined_download_matches_git_objects_without_publishing_refs() {
        let fixture = Fixture::new("pack-success");
        let source = fixture.dir.path().join("source");
        let before = std::fs::read(source.join("HEAD")).unwrap();
        let pem = std::fs::read(fixture.dir.path().join("cert.pem")).unwrap();
        let client = client_builder()
            .add_root_certificate(reqwest::Certificate::from_pem(&pem).unwrap())
            .build()
            .unwrap();
        let quarantine = fixture.dir.path().join("quarantine.git");
        let result = pack::download(&fixture.url, &client, &quarantine).unwrap();
        assert!(result.pack_hash.is_some());
        assert_eq!(result.objects, 7);
        let repo = git2::Repository::open_bare(&quarantine).unwrap();
        assert_eq!(repo.references().unwrap().count(), 0);
        let all_objects = |path: &std::path::Path| {
            let output = Command::new(crate::test_git_executable())
                .arg("--git-dir")
                .arg(path)
                .args([
                    "cat-file",
                    "--batch-all-objects",
                    "--batch-check=%(objectname)",
                ])
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .output()
                .unwrap();
            assert!(output.status.success());
            output.stdout
        };
        assert_eq!(all_objects(&source), all_objects(&quarantine));
        let checked = Command::new(crate::test_git_executable())
            .arg("--git-dir")
            .arg(&quarantine)
            .args(["fsck", "--strict", "--no-reflogs"])
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(
            checked.status.success(),
            "{}",
            String::from_utf8_lossy(&checked.stderr)
        );
        assert_eq!(std::fs::read(source.join("HEAD")).unwrap(), before);
        assert_eq!(fixture.requests().len(), 2);
        assert!(fixture
            .requests()
            .iter()
            .all(|r| r["authorization"].is_null()));
    }
    #[test]
    fn quarantined_download_rejects_corruption_truncation_and_post_redirect() {
        for mode in ["pack-corrupt", "pack-truncate", "pack-redirect"] {
            let fixture = Fixture::new(mode);
            let pem = std::fs::read(fixture.dir.path().join("cert.pem")).unwrap();
            let client = client_builder()
                .add_root_certificate(reqwest::Certificate::from_pem(&pem).unwrap())
                .build()
                .unwrap();
            let quarantine = fixture.dir.path().join("quarantine.git");
            let error = pack::download(&fixture.url, &client, &quarantine).unwrap_err();
            assert_eq!(
                error,
                if mode == "pack-redirect" {
                    "REMOTE_REDIRECT_REJECTED"
                } else {
                    "REMOTE_PACK_INVALID"
                }
            );
            if quarantine.exists() {
                assert_eq!(
                    git2::Repository::open_bare(&quarantine)
                        .unwrap()
                        .references()
                        .unwrap()
                        .count(),
                    0
                );
            }
            assert_eq!(fixture.requests().len(), 2);
        }
    }
    #[test]
    fn controlled_tls_auth_challenge_does_not_send_credentials() {
        let fixture = Fixture::new("auth");
        let result = fixture.inspect();
        assert_eq!(result.unwrap_err(), "REMOTE_AUTH_REQUIRED");
        let requests = fixture.requests();
        assert!(!requests.is_empty());
        assert!(requests
            .iter()
            .all(|request| request["authorization"].is_null()));
    }
}

#[cfg(test)]
mod packet_tests {
    use super::*;
    fn pkt(line: &str) -> Vec<u8> {
        format!("{:04x}{}", line.len() + 4, line).into_bytes()
    }
    fn wire(lines: &[String]) -> Vec<u8> {
        let mut bytes = pkt("# service=git-upload-pack\n");
        bytes.extend_from_slice(b"0000");
        for line in lines {
            bytes.extend(pkt(line));
        }
        bytes.extend_from_slice(b"0000");
        bytes
    }
    #[test]
    fn parser_accepts_optional_newlines_peeled_tags_and_empty_repo() {
        let oid = "1".repeat(40);
        let bytes = wire(&[
            format!("{oid} HEAD\0symref=HEAD:refs/heads/main"),
            format!("{oid} refs/heads/main\n"),
            format!("{oid} refs/tags/v1^{{}}"),
        ]);
        let result = parse_advertisement("https://example.com/r", &bytes).unwrap();
        assert_eq!(result.refs.len(), 3);
        assert_eq!(
            result.refs[0].symbolic_target.as_deref(),
            Some("refs/heads/main")
        );
        assert!(parse_advertisement(
            "https://example.com/r",
            &wire(&[format!("{} capabilities^{{}}\0multi_ack", "0".repeat(40))])
        )
        .unwrap()
        .refs
        .is_empty());
    }
    #[test]
    fn parser_rejects_truncation_duplicate_invalid_refs_and_sha256() {
        let oid = "1".repeat(40);
        let bytes = wire(&[format!("{oid} refs/heads/main")]);
        for n in 0..bytes.len() {
            assert!(
                parse_advertisement("https://example.com/r", &bytes[..n]).is_err(),
                "truncation {n}"
            );
        }
        for lines in [
            vec![
                format!("{oid} refs/heads/main"),
                format!("{oid} refs/heads/main"),
            ],
            vec![format!("{oid} refs/heads/../bad")],
            vec![format!("{oid} HEAD\0object-format=sha256")],
            vec!["not-an-oid HEAD".into()],
        ] {
            assert!(parse_advertisement("https://example.com/r", &wire(&lines)).is_err());
        }
        let mut trailing = bytes;
        trailing.push(0);
        assert!(parse_advertisement("https://example.com/r", &trailing).is_err());
    }
    #[test]
    fn zero_oid_only_allowed_for_sole_empty_repository_sentinel() {
        let zero = "0".repeat(40);
        for lines in [
            vec![format!("{zero} refs/heads/main")],
            vec![
                format!("{zero} capabilities^{{}}"),
                format!("{} refs/heads/main", "1".repeat(40)),
            ],
            vec![
                format!("{} refs/heads/main", "1".repeat(40)),
                format!("{zero} capabilities^{{}}"),
            ],
        ] {
            assert!(parse_advertisement("https://example.com/r", &wire(&lines)).is_err());
        }
    }
    #[test]
    fn parser_caps_reference_count() {
        let lines = (0..2001)
            .map(|n| format!("{} refs/heads/b{n}", "1".repeat(40)))
            .collect::<Vec<_>>();
        assert_eq!(
            parse_advertisement("https://example.com/r", &wire(&lines)).unwrap_err(),
            "REMOTE_REF_LIMIT"
        );
    }
}

#[cfg(test)]
mod push_protocol_tests {
    use super::*;
    fn wire(service: &str) -> Vec<u8> {
        let header = format!("# service={service}\n");
        let reference = format!(
            "{} refs/heads/main\0report-status delete-refs\n",
            "1".repeat(40)
        );
        format!(
            "{:04x}{}0000{:04x}{}0000",
            header.len() + 4,
            header,
            reference.len() + 4,
            reference
        )
        .into_bytes()
    }
    #[test]
    fn push_advertisement_requires_receive_service_and_complete_packets() {
        let receive = wire("git-receive-pack");
        let upload = wire("git-upload-pack");
        assert_eq!(
            parse_service_advertisement("https://example.com/r", &receive, true)
                .unwrap()
                .refs
                .len(),
            1
        );
        assert!(parse_service_advertisement("https://example.com/r", &upload, true).is_err());
        assert!(parse_service_advertisement("https://example.com/r", &receive, false).is_err());
        for end in 0..receive.len() {
            assert!(
                parse_service_advertisement("https://example.com/r", &receive[..end], true)
                    .is_err()
            );
        }
    }
    #[test]
    fn receive_capabilities_survive_empty_repository_and_reject_malformed_values() {
        let directory = tempfile::tempdir().unwrap();
        git2::Repository::init_bare(directory.path()).unwrap();
        let output = std::process::Command::new(crate::test_git_executable())
            .args(["receive-pack", "--stateless-rpc", "--advertise-refs"])
            .arg(directory.path())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .output()
            .unwrap();
        assert!(output.status.success());
        let header = "# service=git-receive-pack\n";
        let mut raw = format!("{:04x}{header}0000", header.len() + 4).into_bytes();
        raw.extend_from_slice(&output.stdout);
        let parsed = parse_service_details("https://example.com/r", &raw, true).unwrap();
        assert!(parsed.advertisement.refs.is_empty());
        assert!(parsed.supports_report_status());
        for caps in [
            "report-status report-status",
            "report-status\tbad",
            "report-status\0bad",
        ] {
            let line = format!("{} capabilities^{{}}\0{caps}\n", "0".repeat(40));
            let wire = format!(
                "{:04x}{header}0000{:04x}{line}0000",
                header.len() + 4,
                line.len() + 4
            );
            assert!(parse_service_details("https://example.com/r", wire.as_bytes(), true).is_err());
        }
        assert!(!valid_capabilities(&["a".repeat(1025)]));
        assert!(!valid_capabilities(
            &(0..257).map(|n| format!("c{n}")).collect::<Vec<_>>()
        ));
        let only_v2 = ReceiveAdvertisement {
            advertisement: parsed.advertisement,
            capabilities: vec!["report-status-v2".into()],
        };
        assert!(!only_v2.supports_report_status());
    }
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) fn index_ssh_fixture_pack(bytes: impl Read, directory: &std::path::Path, advertisement: &Advertisement) -> Result<(), String> {
    pack::index_response(bytes, directory, advertisement).map(|_| ())
}

// SSH has no HTTP service preamble. Reuse the existing strict ref/capability
// parser after reading exactly one bounded advertisement from the stream.
#[cfg(all(test, target_os = "macos"))]
pub(crate) fn read_ssh_advertisement(endpoint: &str, reader: &mut impl Read) -> Result<Advertisement, String> {
    crate::ssh_endpoint::parse(endpoint)?;
    let mut bytes = b"001e# service=git-upload-pack\n0000".to_vec();
    let mut received = 0usize;
    loop {
        let mut prefix = [0u8; 4];
        reader.read_exact(&mut prefix).map_err(|_| "SSH_ADVERTISEMENT_INVALID")?;
        let text = std::str::from_utf8(&prefix).map_err(|_| "SSH_ADVERTISEMENT_INVALID")?;
        if !text.bytes().all(|b| b.is_ascii_hexdigit()) { return Err("SSH_ADVERTISEMENT_INVALID".into()); }
        let length = usize::from_str_radix(text, 16).map_err(|_| "SSH_ADVERTISEMENT_INVALID")?;
        received += length.max(4);
        if received > 64 * 1024 { return Err("SSH_ADVERTISEMENT_LIMIT".into()); }
        bytes.extend_from_slice(&prefix);
        if length == 0 { break; }
        if !(4..=65520).contains(&length) { return Err("SSH_ADVERTISEMENT_INVALID".into()); }
        let start = bytes.len(); bytes.resize(start + length - 4, 0);
        reader.read_exact(&mut bytes[start..]).map_err(|_| "SSH_ADVERTISEMENT_INVALID")?;
    }
    Ok(parse_service_details(endpoint, &bytes, false)?.advertisement)
}

#[cfg(all(test, target_os = "macos"))]
mod ssh_advertisement_tests {
    use super::*;
    use std::io::Cursor;
    const URL: &str = "ssh://fixture@example.invalid:2222/repo";
    fn packet(line: &str) -> Vec<u8> { format!("{:04x}{line}", line.len() + 4).into_bytes() }
    fn sample() -> Vec<u8> {
        let oid = "1".repeat(40);
        let mut bytes = packet(&format!("{oid} HEAD\0symref=HEAD:refs/heads/main object-format=sha1\n"));
        bytes.extend(packet(&format!("{oid} refs/heads/main\n")));
        bytes.extend(packet(&format!("{} refs/tags/v1\n", "2".repeat(40))));
        bytes.extend(packet(&format!("{oid} refs/tags/v1^{{}}\n")));
        bytes.extend(b"0000"); bytes
    }
    #[test]
    fn ssh_refs_preserve_scope_symbols_tags_and_pack_boundary() {
        let raw = sample();
        let mut stream = Cursor::new([raw.as_slice(), b"0008NAK\nPACK"].concat());
        let result = read_ssh_advertisement(URL, &mut stream).unwrap();
        assert_eq!(result.endpoint, URL);
        assert_eq!(result.refs.len(), 4);
        assert_eq!(result.refs[0].symbolic_target.as_deref(), Some("refs/heads/main"));
        assert_eq!(result.refs[3].name, "refs/tags/v1^{}");
        assert_eq!(stream.position(), raw.len() as u64);
        let mut remaining = Vec::new(); stream.read_to_end(&mut remaining).unwrap();
        assert_eq!(remaining, b"0008NAK\nPACK");
        assert!(read_ssh_advertisement(URL, &mut b"0000".as_slice()).unwrap().refs.is_empty());
    }
    #[test]
    fn ssh_advertisement_truncation_injection_duplicates_and_format_are_refused() {
        let raw = sample();
        for end in 0..raw.len() { assert!(read_ssh_advertisement(URL, &mut &raw[..end]).is_err(), "end={end}"); }
        for input in [b"0001".to_vec(), b"0003".to_vec(), b"zzzz".to_vec(), packet("version 2\n"),
            [packet(&format!("{} refs/heads/main\n", "g".repeat(40))), b"0000".to_vec()].concat(),
            [packet(&format!("{} refs/heads/../bad\n", "1".repeat(40))), b"0000".to_vec()].concat(),
            [packet(&format!("{} HEAD\0object-format=sha256\n", "1".repeat(40))), b"0000".to_vec()].concat(),
            [packet(&format!("{} refs/heads/main\n", "1".repeat(40))).repeat(2), b"0000".to_vec()].concat(),
            [packet(&format!("{} HEAD\n", "1".repeat(40))), packet(&format!("{} refs/heads/main\0agent=x\n", "1".repeat(40))), b"0000".to_vec()].concat(),
        ] { assert!(read_ssh_advertisement(URL, &mut input.as_slice()).is_err()); }
        let mut oversized = packet(&"x".repeat(65516));
        oversized.extend(packet(&"x".repeat(20))); oversized.extend(b"0000");
        assert_eq!(read_ssh_advertisement(URL, &mut oversized.as_slice()).unwrap_err(), "SSH_ADVERTISEMENT_LIMIT");
    }
}
