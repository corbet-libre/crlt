//! Isolated process: intentionally invalid upstream TLS configuration must fail closed.

#[tokio::test]
async fn invalid_installed_crypto_provider_cannot_open_a_pool() {
    // Reqwest explicitly honors the process default maintained Rustls provider.
    // No cipher implementation is replaced; an empty supported list is invalid.
    let mut provider = rustls::crypto::ring::default_provider();
    provider.cipher_suites.clear();
    assert!(provider.install_default().is_ok());
    assert!(matches!(
        crlt::Db::open(crlt::Config::new(
            "https://localhost:9",
            "unused-test-token"
        ))
        .await,
        Err(crlt::Error::Config("could not initialize HTTPS client"))
    ));
}
