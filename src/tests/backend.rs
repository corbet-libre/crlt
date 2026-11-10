use super::*;

/// The fixture is optional here: without `.ci/remote.sh`'s exported endpoint a
/// test skips (as the live Turso test does) instead of failing, so the one
/// instrumented coverage execution runs every test of the crate.
fn fixture_present() -> bool {
    matches!(
        (std::env::var("CRLT_TEST_URL"), std::env::var("CRLT_TEST_TOKEN")),
        (Ok(url), Ok(token)) if !url.is_empty() && !token.is_empty()
    )
}

#[tokio::test]
async fn actual_remote_adapter_preserves_named_parameters_and_transaction_modes() {
    if !fixture_present() {
        eprintln!("skipping remote test: .ci/remote.sh supplies the sqld TLS fixture");
        return;
    }
    let url = std::env::var("CRLT_TEST_URL").unwrap();
    let token = std::env::var("CRLT_TEST_TOKEN").unwrap();
    let remote = Remote::new(
        &url,
        &token,
        Sender::new(Duration::from_secs(5)).unwrap(),
        Duration::from_secs(5),
    )
    .unwrap();
    let connection = Connection::Remote(Arc::new(remote));
    connection.check().await.unwrap();
    connection.busy_timeout(Duration::from_millis(1)).unwrap();
    let mut rows = connection
        .query(
            "SELECT :value",
            libsql::named_params! {":value": "named-value"},
        )
        .await
        .unwrap();
    assert_eq!(
        rows.next().await.unwrap().unwrap().get_str(0).unwrap(),
        "named-value"
    );
    for mode in [
        TransactionBehavior::Deferred,
        TransactionBehavior::Exclusive,
    ] {
        let transaction = connection.transaction_with_behavior(mode).await.unwrap();
        assert!(connection.check().await.is_err());
        transaction.rollback().await.unwrap();
        connection.check().await.unwrap();
    }
    // Hrana's server must decide whether its SQLite version supports READONLY.
    match connection
        .transaction_with_behavior(TransactionBehavior::ReadOnly)
        .await
    {
        Ok(transaction) => transaction.rollback().await.unwrap(),
        Err(error) => assert!(matches!(error, Error::Database(_))),
    }
}

#[tokio::test]
async fn broken_remote_stream_cannot_send_a_followup_request() {
    if !fixture_present() {
        eprintln!("skipping remote test: .ci/remote.sh supplies the sqld TLS fixture");
        return;
    }
    let token = std::env::var("CRLT_TEST_TOKEN").unwrap();
    let remote = Remote::new(
        "https://localhost:18443/wrong-status",
        &token,
        Sender::new(Duration::from_secs(5)).unwrap(),
        Duration::from_secs(5),
    )
    .unwrap();
    let connection = Connection::Remote(Arc::new(remote));
    assert!(connection.query("SELECT 1", ()).await.is_err());
    assert!(connection.query("SELECT 1", ()).await.is_err());
    assert!(connection.check().await.is_err());
}

#[test]
fn an_actual_open_stream_can_be_dropped_outside_its_runtime() {
    if !fixture_present() {
        eprintln!("skipping remote test: .ci/remote.sh supplies the sqld TLS fixture");
        return;
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let connection = runtime.block_on(async {
        let token = std::env::var("CRLT_TEST_TOKEN").unwrap();
        let remote = Remote::new(
            "https://localhost:18443/missing-base",
            &token,
            Sender::new(Duration::from_secs(5)).unwrap(),
            Duration::from_secs(5),
        )
        .unwrap();
        let connection = Connection::Remote(Arc::new(remote));
        connection.check().await.unwrap();
        connection
    });
    // Real resources survive leaving block_on; cleanup must tolerate no entered runtime.
    drop(connection);
}
