//! Verified HTTPS with bounded bodies, no redirects and no automatic retries.

use std::time::Duration;

use libsql_hrana::proto::{PipelineReqBody, PipelineRespBody};
use reqwest::{Client, StatusCode, header, redirect, retry};

use crate::{Error, Result};

// Pipeline replies are materialized by the protocol. Bound the wire body before
// decoding it; the public row limit is checked separately during row collection.
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct Sender(Client);

impl Sender {
    pub(crate) fn new(timeout: Duration) -> Result<Self> {
        let client = Client::builder()
            .tls_backend_rustls()
            .https_only(true)
            .http1_only()
            .redirect(redirect::Policy::none())
            .retry(retry::never())
            .no_proxy()
            .timeout(timeout)
            .user_agent("crlt")
            .build()
            .map_err(|_| Error::Config("could not initialize HTTPS client"))?;
        Ok(Self(client))
    }

    pub(crate) async fn send(
        &self,
        url: &url::Url,
        auth: &header::HeaderValue,
        request: &PipelineReqBody,
    ) -> Result<PipelineRespBody> {
        validate_url(url)?;
        let body = serde_json::to_vec(request).map_err(|_| failure("invalid Hrana request"))?;
        let mut response = self
            .0
            .post(url.clone())
            .header(header::AUTHORIZATION, auth.clone())
            .header(header::CONTENT_TYPE, "application/json")
            .header("x-libsql-client-version", "crlt-hrana")
            .body(body)
            .send()
            .await
            .map_err(|_| failure("HTTPS request failed"))?;
        if response.status() != StatusCode::OK {
            // Do not retain error pages, URLs, tokens or request bodies.
            return Err(failure("unexpected HTTPS response status"));
        }
        if response
            .content_length()
            .is_some_and(|len| len > MAX_RESPONSE_BYTES as u64)
        {
            return Err(failure("Hrana response exceeds 16 MiB"));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| failure("HTTPS response failed"))?
        {
            if chunk.len() > MAX_RESPONSE_BYTES - body.len() {
                return Err(failure("Hrana response exceeds 16 MiB"));
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|_| failure("invalid Hrana response"))
    }
}

pub(crate) fn validate_url(url: &url::Url) -> Result<()> {
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(failure("invalid Hrana HTTPS URL"));
    }
    Ok(())
}

pub(crate) fn failure(message: &'static str) -> Error {
    libsql::Error::Hrana(std::io::Error::other(message).into()).into()
}
