//! Opt-in round trip against a disposable real Turso database.

use crlt::{Config, Db, Migration, params};

#[tokio::test]
async fn turso_round_trip_when_credentials_are_supplied() -> crlt::Result<()> {
    let (Ok(url), Ok(token)) = (std::env::var("TURSO_URL"), std::env::var("TURSO_TOKEN")) else {
        eprintln!("skipping live Turso test: TURSO_URL and TURSO_TOKEN are both required");
        return Ok(());
    };
    if url.is_empty() || token.is_empty() {
        eprintln!("skipping live Turso test: TURSO_URL and TURSO_TOKEN must be nonempty");
        return Ok(());
    }
    let db = Db::open(Config::new(url, token)).await?;
    let migrations = [Migration::new(
        1,
        "crlt integration",
        "CREATE TABLE crlt_probe (
        community_id TEXT NOT NULL, id TEXT NOT NULL, value TEXT NOT NULL,
        PRIMARY KEY (community_id, id)
    ) WITHOUT ROWID;",
    )];
    db.migrate(&migrations).await?;
    assert_eq!(db.migrate(&migrations).await?, 0);
    let nonce = tempfile::NamedTempFile::new().expect("local test nonce");
    let key = format!(
        "crlt-{}",
        nonce.path().file_name().unwrap().to_string_lossy()
    );
    let a = db.community(format!("{key}-a"))?;
    let b = db.community(format!("{key}-b"))?;
    let mut tx = a.tx().await?;
    tx.execute(
        "INSERT INTO crlt_probe (id, value) VALUES (?1, ?2)",
        params!["same", "a"],
    )
    .await?;
    tx.commit().await?;
    b.execute(
        "INSERT INTO crlt_probe (id, value) VALUES (?1, ?2)",
        params!["same", "b"],
    )
    .await?;
    assert_eq!(
        a.query("SELECT value FROM crlt_probe", ()).await?[0].get_str(0)?,
        "a"
    );
    assert_eq!(
        b.query("SELECT value FROM crlt_probe", ()).await?[0].get_str(0)?,
        "b"
    );
    a.explain("SELECT value FROM crlt_probe WHERE id = ?1", ["same"])
        .await?
        .assert_indexed()?;
    let mut tx = a.tx().await?;
    tx.execute("DELETE FROM crlt_probe", ()).await?;
    tx.rollback().await?;
    assert_eq!(a.query("SELECT value FROM crlt_probe", ()).await?.len(), 1);
    a.execute("DELETE FROM crlt_probe", ()).await?;
    b.execute("DELETE FROM crlt_probe", ()).await?;
    Ok(())
}
