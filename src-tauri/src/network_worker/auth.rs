//! Private parent/worker pipe envelope only, never renderer IPC or disk.
use crate::credential_vault::Credential;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TransportCredential {
    endpoint: String,
    #[serde(deserialize_with = "read_record")]
    record: Zeroizing<Vec<u8>>,
}
fn read_record<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Zeroizing<Vec<u8>>, D::Error> {
    struct Record;
    impl<'de> serde::de::Visitor<'de> for Record {
        type Value = Zeroizing<Vec<u8>>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("bounded private credential record")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(
            self,
            mut seq: A,
        ) -> Result<Self::Value, A::Error> {
            let mut value = Zeroizing::new(Vec::with_capacity(16384));
            while let Some(byte) = seq.next_element::<u8>()? {
                if value.len() == 16384 {
                    return Err(serde::de::Error::custom("credential record limit"));
                }
                value.push(byte);
            }
            Ok(value)
        }
    }
    deserializer.deserialize_seq(Record)
}
impl TransportCredential {
    pub(crate) fn new(endpoint: &str, credential: &Credential) -> Result<Self, String> {
        Ok(Self {
            endpoint: crate::repository::remote_endpoint::https(endpoint)?.to_string(),
            record: credential.encode(),
        })
    }
    pub(crate) fn credential(&self, endpoint: &str) -> Result<Credential, String> {
        let canonical = crate::repository::remote_endpoint::https(endpoint)?;
        if canonical.as_str() != self.endpoint {
            return Err("REMOTE_CREDENTIAL_SCOPE_MISMATCH".into());
        }
        Credential::decode(&self.record).map_err(str::to_owned)
    }
}
// No Debug: a derived representation would expose the binary record.
pub(super) fn client_builder(
    endpoint: &str,
    credential: Option<&TransportCredential>,
) -> Result<reqwest::blocking::ClientBuilder, String> {
    let mut builder = super::client_builder();
    if let Some(bound) = credential {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::AUTHORIZATION,
            bound.credential(endpoint)?.authorization()?,
        );
        builder = builder.default_headers(headers);
    }
    // Client is private to this one worker request. All HTTP requests derive
    // from its validated endpoint; redirects, retries and proxies stay disabled.
    Ok(builder)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forged_or_malformed_private_envelopes_never_create_authenticated_client() {
        let url = "https://example.invalid/repo.git";
        for record in [vec![], b"BDC1\0\0\0\x01u\n".to_vec(), vec![0; 16385]] {
            let value = TransportCredential {
                endpoint: url.into(),
                record: Zeroizing::new(record),
            };
            assert!(client_builder(url, Some(&value)).is_err());
        }
        let secret = Credential::new("u".into(), "token".into()).unwrap();
        let bound = TransportCredential::new(url, &secret).unwrap();
        for other in [
            "https://example.invalid/other.git",
            "https://example.invalid:8443/repo.git",
            "https://else.invalid/repo.git",
            "http://example.invalid/repo.git",
        ] {
            assert!(client_builder(other, Some(&bound)).is_err());
        }
        let bytes = Zeroizing::new(serde_json::to_vec(&bound).unwrap());
        let restored: TransportCredential = serde_json::from_slice(&bytes).unwrap();
        assert!(client_builder(url, Some(&restored)).is_ok());
    }
}
