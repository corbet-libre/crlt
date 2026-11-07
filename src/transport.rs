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

/// reqwest is built with `rustls-no-provider`: the process default must be
/// selected before a client is built. Keep an embedding application's
/// provider; otherwise pick the maintained ring provider.
fn install_provider() {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
}

fn builder(timeout: Duration) -> reqwest::ClientBuilder {
    Client::builder()
        .tls_backend_rustls()
        .https_only(true)
        .http1_only()
        .redirect(redirect::Policy::none())
        .retry(retry::never())
        .no_proxy()
        .timeout(timeout)
        .user_agent("crlt")
}

impl Sender {
    pub(crate) fn new(timeout: Duration) -> Result<Self> {
        install_provider();
        let client = builder(timeout)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// One real TLS 1.3 handshake with a self-signed loopback certificate.
    /// No network, no name resolution: the certificate is the only root.
    async fn tls_server() -> (SocketAddr, reqwest::Certificate) {
        let key = rcgen::KeyPair::generate().expect("key");
        let mut params = rcgen::CertificateParams::new(Vec::new()).expect("params");
        params.subject_alt_names = vec![rcgen::SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST))];
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "crlt-handshake");
        let cert = params.self_signed(&key).expect("self-signed");
        let der = cert.der().to_vec();
        let config = tokio_rustls::rustls::ServerConfig::builder_with_provider(Arc::new(
            tokio_rustls::rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .expect("versions")
        .with_no_client_auth()
        .with_single_cert(
            vec![der.clone().into()],
            tokio_rustls::rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into()),
        )
        .expect("server config");
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("listener");
        let addr = listener.local_addr().expect("address");
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let mut stream = acceptor.accept(stream).await.expect("handshake");
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request).await.expect("request");
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\nok")
                .await
                .expect("response");
            stream.shutdown().await.expect("close");
        });
        (addr, reqwest::Certificate::from_der(&der).expect("root"))
    }

    #[tokio::test]
    async fn the_product_client_trusts_an_explicit_root_and_completes_tls() {
        install_provider();
        let (addr, root) = tls_server().await;
        let client = builder(Duration::from_secs(5))
            .add_root_certificate(root)
            .build()
            .expect("client");
        let response = client
            .get(format!("https://127.0.0.1:{}/", addr.port()))
            .send()
            .await
            .expect("request");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.text().await.expect("body"), "ok");
    }
}
