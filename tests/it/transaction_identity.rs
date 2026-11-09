//! Transaction composition uses exact database and community capability identity.
use crlt::{Config, Db};

#[tokio::test]
async fn transaction_identity_accepts_clones_and_refuses_other_databases_scopes_and_reopened_handles()
 {
    let directory = tempfile::tempdir().unwrap();
    let url = format!("file://{}", directory.path().join("identity.db").display());
    let db = Db::open(Config::new(&url, "")).await.unwrap();
    let another = Db::open(Config::new(
        format!("file://{}", directory.path().join("other.db").display()),
        "",
    ))
    .await
    .unwrap();
    let reopened = Db::open(Config::new(&url, "")).await.unwrap();
    let community = db.community("a").unwrap();
    let transaction = community.tx().await.unwrap();
    assert!(transaction.belongs_to(&community));
    assert!(transaction.belongs_to(&community.clone()));
    assert!(transaction.belongs_to(&db.clone().community("a").unwrap()));
    assert!(!transaction.belongs_to(&db.community("b").unwrap()));
    assert!(!transaction.belongs_to(&another.community("a").unwrap()));
    assert!(!transaction.belongs_to(&reopened.community("a").unwrap()));
    transaction.rollback().await.unwrap();
}
