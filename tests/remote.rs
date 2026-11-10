//! Required in CI against authenticated sqld behind a real, verified TLS endpoint.

#[allow(dead_code)]
mod common;

use std::time::Duration;

use crlt::{Config, Db, Error, Migration, Value, params};

/// The fixture is optional here: without `.ci/remote.sh`'s exported endpoint a
/// test skips (as the live Turso test does) instead of failing, so the one
/// instrumented coverage execution runs every test of the crate.
fn fixture_present() -> bool {
    matches!(
        (std::env::var("CRLT_TEST_URL"), std::env::var("CRLT_TEST_TOKEN")),
        (Ok(url), Ok(token)) if !url.is_empty() && !token.is_empty()
    )
}

fn config() -> Config {
    Config::new(
        std::env::var("CRLT_TEST_URL").expect("run .ci/remote.sh"),
        std::env::var("CRLT_TEST_TOKEN").expect("run .ci/remote.sh"),
    )
}

async fn fixture() -> Db {
    let db = Db::open(config()).await.unwrap();
    db.migrate(common::MIGRATIONS).await.unwrap();
    db
}

#[tokio::test]
async fn tls_values_namespaces_migrations_and_indexes() -> crlt::Result<()> {
    if !fixture_present() {
        eprintln!("skipping remote test: .ci/remote.sh supplies the sqld TLS fixture");
        return Ok(());
    }
    let db = fixture().await;
    assert_eq!(db.migrate(common::MIGRATIONS).await?, 0);
    let a = db.community("remote-a")?;
    let b = db.community("remote-b")?;
    let mut tx = a.tx().await?;
    let text = "Unicode 🦀, quotes ' and NUL\0 survive parameters";
    let blob = vec![0, 1, 127, 128, 255];
    tx.execute(
        "INSERT INTO members (id, handle, score, payload) VALUES (?1, ?2, ?3, ?4)",
        params!["same", text, i64::MIN, blob.clone()],
    )
    .await?;
    tx.execute(
        "INSERT INTO notes (member_id, body) VALUES (?1, ?2)",
        params!["same", "joined"],
    )
    .await?;
    tx.commit().await?;
    b.execute(
        "INSERT INTO members (id, handle, score, payload) VALUES (?1, ?2, ?3, ?4)",
        params!["same", "other", i64::MAX, Value::Null],
    )
    .await?;
    let rows = a
        .query(
            "SELECT handle, score, payload FROM members WHERE id = ?1",
            ["same"],
        )
        .await?;
    assert_eq!(rows[0].columns(), &["handle", "score", "payload"]);
    assert_eq!(rows[0].get_str(0)?, text);
    assert_eq!(rows[0].get_i64(1)?, i64::MIN);
    assert_eq!(rows[0].get_value(2)?, &Value::Blob(blob));
    let rows = b
        .query("SELECT score, payload FROM members WHERE id = ?1", ["same"])
        .await?;
    assert_eq!(rows[0].get_i64(0)?, i64::MAX);
    assert_eq!(rows[0].get_value(1)?, &Value::Null);
    assert!(
        a.query(
            "SELECT handle FROM members WHERE community_id = ?1",
            ["remote-b"]
        )
        .await?
        .is_empty()
    );
    assert_eq!(
        a.query(
            "SELECT n.body FROM members AS m JOIN notes AS n ON n.member_id = m.id",
            ()
        )
        .await?[0]
            .get_str(0)?,
        "joined"
    );
    let plan = a
        .explain("SELECT id FROM members WHERE score = ?1", [i64::MIN])
        .await?;
    plan.assert_indexed()?;
    assert!(
        plan.details()
            .iter()
            .any(|line| line.contains("members_score"))
    );
    let rows = a
        .query(
            "UPDATE members SET score = ?1, payload = ?2 WHERE id = ?3 RETURNING score, payload",
            params![1.25, Vec::<u8>::new(), "same"],
        )
        .await?;
    assert_eq!(rows[0].get_value(0)?, &Value::Real(1.25));
    assert_eq!(rows[0].get_value(1)?, &Value::Blob(vec![]));
    assert!(matches!(
        db.migrate(&[]).await,
        Err(Error::MigrationHistory)
    ));
    let invalid = [
        common::MIGRATIONS[0],
        Migration::new(
            2,
            "invalid",
            "CREATE TABLE missing_scope (id INTEGER PRIMARY KEY)",
        ),
    ];
    assert!(matches!(db.migrate(&invalid).await, Err(Error::Schema(_))));
    assert_eq!(db.migrate(common::MIGRATIONS).await?, 0);
    // The failed DDL was rolled back and the persisted ledger reopens cleanly.
    let reopened = fixture().await;
    assert_eq!(
        reopened
            .community("remote-a")?
            .query("SELECT id FROM members", ())
            .await?
            .len(),
        1
    );
    Ok(())
}

async fn await_rollback(db: &Db, key: &str) -> crlt::Result<()> {
    // Cleanup is best effort and asynchronous. Acquiring a new writer proves
    // the discarded stream released its write lock as well as rolling back.
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Ok(mut tx) = db.tx(key).await {
                assert!(tx.query("SELECT id FROM members", ()).await?.is_empty());
                tx.rollback().await?;
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("discarded transaction must release the server lock")
}

#[tokio::test]
async fn tls_transactions_rollback_poison_and_drop() -> crlt::Result<()> {
    if !fixture_present() {
        eprintln!("skipping remote test: .ci/remote.sh supplies the sqld TLS fixture");
        return Ok(());
    }
    let db = fixture().await;
    let a = db.community("remote-rollback")?;
    let mut tx = a.tx().await?;
    tx.execute(
        "INSERT INTO members (id, handle) VALUES (?1, ?2)",
        params!["rollback", "rollback"],
    )
    .await?;
    tx.rollback().await?;
    assert!(a.query("SELECT id FROM members", ()).await?.is_empty());
    let mut tx = a.tx().await?;
    tx.execute(
        "INSERT INTO members (id, handle) VALUES (?1, ?2)",
        params!["dropped", "dropped"],
    )
    .await?;
    drop(tx);
    await_rollback(&db, "remote-rollback").await?;
    let mut tx = a.tx().await?;
    tx.execute(
        "INSERT INTO members (id, handle) VALUES (?1, ?2)",
        params!["duplicate", "duplicate"],
    )
    .await?;
    assert!(
        tx.execute(
            "INSERT INTO members (id, handle) VALUES (?1, ?2)",
            params!["duplicate", "duplicate"]
        )
        .await
        .is_err()
    );
    assert!(matches!(tx.commit().await, Err(Error::TransactionAborted)));
    await_rollback(&db, "remote-rollback").await?;
    // Foreign key checks are still active over the remote connection.
    assert!(
        a.execute(
            "INSERT INTO notes (member_id, body) VALUES (?1, ?2)",
            params!["missing", "orphan"]
        )
        .await
        .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn tls_limits_deadlines_and_pool_recovery() -> crlt::Result<()> {
    if !fixture_present() {
        eprintln!("skipping remote test: .ci/remote.sh supplies the sqld TLS fixture");
        return Ok(());
    }
    let db = fixture().await;
    let a = db.community("remote-limits")?;
    a.execute(
        "INSERT INTO members (id, handle) VALUES (?1, ?2), (?3, ?4)",
        params!["1", "one", "2", "two"],
    )
    .await?;
    let mut limited = config();
    limited.max_rows = 1;
    let limited = Db::open(limited).await?;
    limited.migrate(common::MIGRATIONS).await?;
    let mut tx = limited.tx("remote-limits").await?;
    assert!(matches!(
        tx.query("DELETE FROM members RETURNING id", ()).await,
        Err(Error::RowLimit)
    ));
    assert!(matches!(tx.commit().await, Err(Error::TransactionAborted)));
    let mut tx = a.tx().await?;
    assert_eq!(tx.query("SELECT id FROM members", ()).await?.len(), 2);
    tx.rollback().await?;
    // Cancelling a waiter must not steal or recycle an active transaction lease.
    let tx = a.tx().await?;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), a.tx())
            .await
            .is_err()
    );
    tx.rollback().await?;
    a.tx().await?.rollback().await?;

    let mut short = config();
    short.timeout = Duration::from_secs(1);
    short.busy_timeout = Duration::from_millis(10);
    let short = Db::open(short).await?;
    short.migrate(common::MIGRATIONS).await?;
    let mut tx = short.tx("remote-expired").await?;
    tx.execute(
        "INSERT INTO members (id, handle) VALUES (?1, ?2)",
        params!["late", "late"],
    )
    .await?;
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(matches!(tx.commit().await, Err(Error::Timeout)));
    await_rollback(&db, "remote-expired").await?;
    Ok(())
}

#[tokio::test]
async fn tls_rejects_bad_certificates_auth_redirects_and_downgrades() {
    if !fixture_present() {
        eprintln!("skipping remote test: .ci/remote.sh supplies the sqld TLS fixture");
        return;
    }
    // Positive control uses the same endpoint and OS trust store as the failures.
    Db::open(config()).await.unwrap();
    let token = std::env::var("CRLT_TEST_TOKEN").unwrap();
    Db::open(Config::new("libsql://localhost:18443", &token))
        .await
        .unwrap();
    for url in [
        "https://localhost:18444", // self-signed, untrusted root
        "https://127.0.0.1:18443", // trusted certificate, wrong hostname
        "https://localhost:18443/redirect",
        "https://localhost:18443/downgrade", // server-supplied HTTP base URL
        "https://localhost:18443/userinfo",  // server-supplied URL credentials
        "https://localhost:18443/missing-baton",
        "https://localhost:18443/missing-status",
        "http://localhost:18443",
    ] {
        assert!(
            Db::open(Config::new(url, &token)).await.is_err(),
            "accepted {url}"
        );
    }
    assert!(
        Db::open(Config::new("https://localhost:18443", "invalid-test-token"))
            .await
            .is_err()
    );
    // Invalid header construction occurs while acquiring the first pool lease.
    assert!(matches!(
        Db::open(Config::new("https://localhost:18443", "invalid\nheader")).await,
        Err(Error::Pool)
    ));
    let mut delayed = Config::new("https://localhost:18443/delay", token);
    delayed.timeout = Duration::from_millis(100);
    delayed.busy_timeout = Duration::from_millis(10);
    assert!(matches!(Db::open(delayed).await, Err(Error::Timeout)));
}

#[tokio::test]
async fn tls_cancelled_response_aborts_transaction_and_releases_lease() -> crlt::Result<()> {
    if !fixture_present() {
        eprintln!("skipping remote test: .ci/remote.sh supplies the sqld TLS fixture");
        return Ok(());
    }
    let db = fixture().await;
    let mut tx = db.tx("remote-cancelled").await?;
    tx.execute(
        "INSERT INTO members (id, handle) VALUES (?1, ?2)",
        params!["cancelled", "cancelled"],
    )
    .await?;
    // The proxy holds a real sqld reply after execution. The public transaction
    // and private stream must both become unusable when that future is dropped.
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            tx.query("SELECT id FROM members WHERE id = ?1", ["crlt-delay-query"]),
        )
        .await
        .is_err()
    );
    assert!(matches!(tx.commit().await, Err(Error::TransactionAborted)));
    await_rollback(&db, "remote-cancelled").await
}

#[tokio::test]
async fn tls_uncertain_write_is_not_retried() -> crlt::Result<()> {
    if !fixture_present() {
        eprintln!("skipping remote test: .ci/remote.sh supplies the sqld TLS fixture");
        return Ok(());
    }
    let db = fixture().await;
    let a = db.community("remote-uncertain")?;
    a.execute(
        "INSERT INTO members (id, handle) VALUES (?1, ?2)",
        params!["crlt-unknown-write", "counter"],
    )
    .await?;
    // The proxy replaces a successful UPDATE reply with HTTP 503. A retry
    // would increment this counter twice, even though the caller saw an error.
    assert!(
        a.execute(
            "UPDATE members SET score = score + ?1 WHERE id = ?2",
            params![1, "crlt-unknown-write"],
        )
        .await
        .is_err()
    );
    assert_eq!(
        a.query(
            "SELECT score FROM members WHERE id = ?1",
            ["crlt-unknown-write"],
        )
        .await?[0]
            .get_i64(0)?,
        1
    );
    Ok(())
}

#[tokio::test]
async fn tls_refuses_corrupted_actual_server_replies_and_recovers() {
    if !fixture_present() {
        eprintln!("skipping remote test: .ci/remote.sh supplies the sqld TLS fixture");
        return;
    }
    let token = std::env::var("CRLT_TEST_TOKEN").unwrap();
    for fault in [
        "missing-health",
        "wrong-health",
        "wrong-width",
        "invalid-value",
        "wrong-status",
        "wrong-response",
        "foreign-keys-off",
        "missing-foreign-keys",
        "query-url",
        "fragment-url",
        "password-url",
        "oversize-length",
        "oversize-stream",
        "truncated-body",
    ] {
        let endpoint = format!("https://localhost:18443/{fault}");
        assert!(
            Db::open(Config::new(endpoint, &token)).await.is_err(),
            "accepted corrupted server reply: {fault}"
        );
    }
    // The actual upstream remains healthy after refused deliveries.
    Db::open(config()).await.unwrap();
}

#[tokio::test]
async fn tls_refuses_inconsistent_actual_transaction_state() {
    if !fixture_present() {
        eprintln!("skipping remote test: .ci/remote.sh supplies the sqld TLS fixture");
        return;
    }
    let token = std::env::var("CRLT_TEST_TOKEN").unwrap();
    for fault in ["begin-state", "commit-state", "rollback-state"] {
        let endpoint = format!("https://localhost:18443/{fault}");
        let db = Db::open(Config::new(endpoint, &token)).await.unwrap();
        let transaction = db.tx("state-refusal").await;
        if fault == "begin-state" {
            assert!(transaction.is_err());
        } else if fault == "commit-state" {
            assert!(transaction.unwrap().commit().await.is_err());
        } else {
            assert!(transaction.unwrap().rollback().await.is_err());
        }
    }
    Db::open(config()).await.unwrap();
}

#[tokio::test]
async fn tls_refuses_non_finite_parameters_without_mutation() {
    if !fixture_present() {
        eprintln!("skipping remote test: .ci/remote.sh supplies the sqld TLS fixture");
        return;
    }
    let db = fixture().await;
    let community = db.community("non-finite").unwrap();
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(
            community
                .execute(
                    "INSERT INTO members (id, handle, score) VALUES (?1, ?2, ?3)",
                    params!["finite", "handle", value],
                )
                .await
                .is_err()
        );
    }
    assert!(
        community
            .query("SELECT id FROM members WHERE id = ?1", ["finite"])
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn failed_remote_migration_preserves_its_error_when_rollback_state_is_invalid() {
    if !fixture_present() {
        eprintln!("skipping remote test: .ci/remote.sh supplies the sqld TLS fixture");
        return;
    }
    let healthy = fixture().await;
    let token = std::env::var("CRLT_TEST_TOKEN").unwrap();
    let db = Db::open(Config::new("https://localhost:18443/rollback-state", token))
        .await
        .unwrap();
    assert!(matches!(
        db.migrate(&[]).await,
        Err(Error::MigrationHistory)
    ));
    assert_eq!(healthy.migrate(common::MIGRATIONS).await.unwrap(), 0);
}
