//! HTTPS Push transport exercised against synthetic TLS before worker/journal wiring.
use super::*;
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Failure {
    BeforeSend(String),
    OutcomeUnknown,
}
pub(super) fn transfer(
    client: &reqwest::blocking::Client,
    url: &str,
    old: &str,
    new: &str,
    reference: &str,
    prepared: &push_pack::PreparedPack,
) -> Result<push_wire::Report, Failure> {
    let before = Failure::BeforeSend;
    prepared.validate_update(old, new).map_err(before)?;
    let pack = prepared.bytes().map_err(before)?;
    // Validate caller fields before making even a read-only remote request.
    push_wire::request(old, new, reference, &["report-status".into()], &pack).map_err(before)?;
    let received = inspect_service_details(url, client, true).map_err(before)?;
    let actual = received
        .advertisement
        .refs
        .iter()
        .find(|r| r.name == reference);
    if actual.is_some_and(|r| r.symbolic_target.is_some())
        || actual
            .map(|r| r.oid.as_str())
            .unwrap_or("0000000000000000000000000000000000000000")
            != old
    {
        return Err(before("PUSH_REMOTE_CHANGED".into()));
    }
    let body =
        push_wire::request(old, new, reference, &received.capabilities, &pack).map_err(before)?;
    let endpoint = format!(
        "{}/git-receive-pack",
        received.advertisement.endpoint.trim_end_matches('/')
    );
    // Once POST begins, transport errors cannot prove whether publication occurred.
    let response = client
        .post(endpoint)
        .header("Content-Type", "application/x-git-receive-pack-request")
        .header("Accept", "application/x-git-receive-pack-result")
        .body(body)
        .send()
        .map_err(|_| Failure::OutcomeUnknown)?;
    if !response.status().is_success()
        || response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|v| v.split(';').next().unwrap_or("").trim())
            != Some("application/x-git-receive-pack-result")
        || response.content_length().is_some_and(|n| n > 65536)
    {
        return Err(Failure::OutcomeUnknown);
    }
    let mut bytes = Vec::new();
    response
        .take(65537)
        .read_to_end(&mut bytes)
        .map_err(|_| Failure::OutcomeUnknown)?;
    push_wire::report(&bytes, reference).map_err(|_| Failure::OutcomeUnknown)
}
