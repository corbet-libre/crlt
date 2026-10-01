//! Migration and configuration failures against real local files.

mod common;

use common::{MIGRATIONS, config, empty, fixture};
use crlt::{Config, Db, Error, Migration};

#[tokio::test]
async fn existing_leaf_schemas_migrate_together_and_reopen() {
    // Exact source revisions are recorded beside these SQL snapshots.
    let schemas = [
        ("crgs", include_str!("fixtures/leaf_schemas/crgs.sql")),
        ("clbs", include_str!("fixtures/leaf_schemas/clbs.sql")),
        ("cnrl", include_str!("fixtures/leaf_schemas/cnrl.sql")),
        ("crbk", include_str!("fixtures/leaf_schemas/crbk.sql")),
        ("cpky", include_str!("fixtures/leaf_schemas/cpky.sql")),
        ("cpns", include_str!("fixtures/leaf_schemas/cpns.sql")),
        ("csgn", include_str!("fixtures/leaf_schemas/csgn.sql")),
        ("cgrd", include_str!("fixtures/leaf_schemas/cgrd.sql")),
        ("cssr", include_str!("fixtures/leaf_schemas/cssr.sql")),
        (
            "cpsd",
            include_str!("fixtures/leaf_schemas/cpsd_schema.sql"),
        ),
        (
            "cpsd issuance",
            include_str!("fixtures/leaf_schemas/cpsd_issuance_schema.sql"),
        ),
        (
            "cpsd uniqueness",
            include_str!("fixtures/leaf_schemas/cpsd_issuance_uniqueness_schema.sql"),
        ),
    ];
    let (dir, db) = empty().await;
    let mut migrations = Vec::new();
    for (index, (name, sql)) in schemas.iter().enumerate() {
        migrations.push(Migration::new(u32::try_from(index + 1).unwrap(), name, sql));
        assert_eq!(db.migrate(&migrations).await.unwrap(), 1, "{name}");
    }
    drop(db);
    let db = Db::open(config(&dir)).await.unwrap();
    assert_eq!(db.migrate(&migrations).await.unwrap(), 0);
    for community in ["a", "b"] {
        let scope = db.community(community).unwrap();
        for table in [
            "crgs_members",
            "crgs_claims",
            "clbs_orders",
            "cnrl_enrolments",
            "rulebook_revision",
            "setting_catalog",
            "platform_value",
            "template",
            "template_value",
            "community_template",
            "community_value",
            "cpky_passkeys",
            "cpns_pins",
            "csgn_state",
            "cgrd_snapshots",
            "cssr_records",
            "cpsd_challenges",
            "cpsd_issuer_tags",
        ] {
            assert!(
                scope
                    .query(&format!("SELECT * FROM {table}"), ())
                    .await
                    .unwrap()
                    .is_empty(),
                "{table}"
            );
        }
    }
}

#[tokio::test]
async fn repeat_reopen_and_append_preserve_data() {
    let (dir, db) = fixture().await;
    assert_eq!(db.migrate(MIGRATIONS).await.unwrap(), 0);
    db.community("a")
        .unwrap()
        .execute("INSERT INTO members (id, handle) VALUES ('x', 'alice')", ())
        .await
        .unwrap();
    drop(db);
    let db = Db::open(config(&dir)).await.unwrap();
    assert!(matches!(
        db.community("a")
            .unwrap()
            .query("SELECT * FROM members", ())
            .await,
        Err(Error::Sql(_))
    ));
    assert_eq!(db.migrate(MIGRATIONS).await.unwrap(), 0);
    let mut history = MIGRATIONS.to_vec();
    history.push(Migration::new(
        2,
        "add label",
        "ALTER TABLE members ADD COLUMN label TEXT;",
    ));
    assert_eq!(db.migrate(&history).await.unwrap(), 1);
    assert_eq!(db.migrate(&history).await.unwrap(), 0);
    let rows = db
        .community("a")
        .unwrap()
        .query("SELECT handle, label FROM members", ())
        .await
        .unwrap();
    assert_eq!(rows[0].get_str(0).unwrap(), "alice");
    assert_eq!(rows[0].get_value(1).unwrap(), &crlt::Value::Null);
}

#[tokio::test]
async fn migration_history_is_complete_contiguous_and_immutable() {
    let (_dir, db) = fixture().await;
    assert!(matches!(
        db.migrate(&[]).await,
        Err(Error::MigrationHistory)
    ));
    let original = MIGRATIONS[0];
    for migration in [
        Migration::new(1, "renamed", original.sql),
        Migration::new(1, original.name, "CREATE TABLE changed (id TEXT)"),
        Migration::new(0, original.name, original.sql),
        Migration::new(2, original.name, original.sql),
        Migration::new(1, "", original.sql),
        Migration::new(1, original.name, ""),
    ] {
        assert!(matches!(
            db.migrate(&[migration]).await,
            Err(Error::MigrationHistory)
        ));
    }
    assert!(matches!(
        db.migrate(&[original, original]).await,
        Err(Error::MigrationHistory)
    ));
    assert_eq!(db.migrate(MIGRATIONS).await.unwrap(), 0);
}

#[tokio::test]
async fn failed_pending_batch_rolls_back_ddl_and_history() {
    let (_dir, db) = fixture().await;
    let added = Migration::new(
        2,
        "extra",
        "CREATE TABLE extra (community_id TEXT NOT NULL, id TEXT NOT NULL, PRIMARY KEY (community_id, id)) WITHOUT ROWID;",
    );
    let broken = Migration::new(3, "broken", "CREATE INDEX broken ON absent (community_id);");
    assert!(db.migrate(&[MIGRATIONS[0], added, broken]).await.is_err());
    assert_eq!(db.migrate(&[MIGRATIONS[0], added]).await.unwrap(), 1);
    assert!(
        db.community("a")
            .unwrap()
            .query("SELECT * FROM extra", ())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn first_migration_failure_leaves_a_reusable_database() {
    let (_dir, db) = empty().await;
    assert!(
        db.migrate(&[Migration::new(
            1,
            "unsafe",
            "CREATE TABLE unsafe_table (id TEXT)"
        )])
        .await
        .is_err()
    );
    assert_eq!(db.migrate(MIGRATIONS).await.unwrap(), 1);
}

#[tokio::test]
async fn rejects_schemas_that_could_mix_communities() {
    let invalid = [
        "CREATE TABLE sqlitexordinary (id TEXT PRIMARY KEY)",
        "CREATE TABLE bad (id TEXT PRIMARY KEY)",
        "CREATE TABLE bad (community_id TEXT, id TEXT, PRIMARY KEY (community_id, id))",
        "CREATE TABLE bad (community_id BLOB NOT NULL, id TEXT, PRIMARY KEY (community_id, id))",
        "CREATE TABLE bad (community_id TEXT NOT NULL, id TEXT, PRIMARY KEY (id, community_id))",
        "CREATE TABLE bad (community_id TEXT NOT NULL COLLATE NOCASE, id TEXT, PRIMARY KEY (community_id, id))",
        "CREATE TABLE bad (community_id TEXT NOT NULL COLLATE NOCASE, id TEXT, PRIMARY KEY (community_id COLLATE BINARY, id))",
        "CREATE TABLE bad (community_id TEXT NOT NULL, id TEXT UNIQUE, PRIMARY KEY (community_id, id))",
        "CREATE UNIQUE INDEX global_handle ON members (handle)",
        "CREATE UNIQUE INDEX fuzzy_community ON members (community_id COLLATE NOCASE, score)",
        "CREATE TABLE bad (community_id TEXT NOT NULL, id TEXT, PRIMARY KEY (community_id, id), FOREIGN KEY (id) REFERENCES members (id))",
        "CREATE TABLE bad (community_id TEXT NOT NULL, id TEXT, PRIMARY KEY (community_id, id), FOREIGN KEY (community_id, id) REFERENCES members (id, community_id))",
        "CREATE VIEW bad AS SELECT * FROM members",
        "CREATE TRIGGER bad AFTER INSERT ON members BEGIN DELETE FROM members; END",
        "CREATE VIRTUAL TABLE bad USING fts5(body)",
        "CREATE TEMP TABLE bad (id TEXT)",
        "CREATE TABLE bad AS SELECT * FROM members",
        "ALTER TABLE members RENAME COLUMN community_id TO tenant",
        "PRAGMA foreign_keys = OFF",
        "COMMIT",
        "INSERT INTO members (community_id, id, handle) VALUES ('x', 'x', 'x')",
        "DROP TABLE __crlt_migrations",
        "ALTER TABLE __crlt_migrations ADD COLUMN changed TEXT",
    ];
    let (_dir, db) = fixture().await;
    for sql in invalid {
        let migrations = [MIGRATIONS[0], Migration::new(2, "invalid", sql)];
        assert!(db.migrate(&migrations).await.is_err(), "{sql}");
        assert_eq!(db.migrate(MIGRATIONS).await.unwrap(), 0, "{sql}");
    }
}

#[tokio::test]
async fn accepts_case_variants_and_explicit_binary_collation() {
    let (_dir, db) = empty().await;
    db.migrate(&[Migration::new(1, "case", "CREATE TABLE Mixed (COMMUNITY_ID TEXT NOT NULL COLLATE BINARY, ID TEXT NOT NULL, PRIMARY KEY (COMMUNITY_ID, ID)) WITHOUT ROWID;")]).await.unwrap();
    let a = db.community("a").unwrap();
    let b = db.community("A").unwrap();
    a.execute("INSERT INTO Mixed (ID) VALUES ('same')", ())
        .await
        .unwrap();
    b.execute("INSERT INTO Mixed (ID) VALUES ('same')", ())
        .await
        .unwrap();
    assert_eq!(
        a.query("SELECT COMMUNITY_ID FROM Mixed", ()).await.unwrap()[0]
            .get_str(0)
            .unwrap(),
        "a"
    );
}

#[tokio::test]
async fn concurrent_migration_calls_apply_once() {
    let (_dir, db) = empty().await;
    let other = db.clone();
    let (a, b) = tokio::join!(db.migrate(MIGRATIONS), other.migrate(MIGRATIONS));
    assert_eq!(a.unwrap() + b.unwrap(), 1);
}

#[tokio::test]
async fn validates_urls_and_limits_without_network_or_credentials() {
    for (url, token) in [
        ("http://example.com", "token"),
        ("libsql://example.com", ""),
        ("https://user:secret@example.com", "token"),
        ("https://example.com?token=secret", "token"),
        ("libsql://example.com#secret", "token"),
        ("file:///tmp/db", "token"),
        (":memory:", ""),
        ("relative.db", ""),
        ("file://other-host/tmp/db", ""),
        ("file:///", ""),
    ] {
        assert!(
            matches!(
                Db::open(Config::new(url, token)).await,
                Err(Error::Config(_))
            ),
            "{url}"
        );
    }
    let dir = tempfile::tempdir().unwrap();
    let mut settings = config(&dir);
    settings.max_connections = 0;
    assert!(matches!(Db::open(settings).await, Err(Error::Config(_))));
    let mut settings = config(&dir);
    settings.timeout = std::time::Duration::ZERO;
    assert!(matches!(Db::open(settings).await, Err(Error::Config(_))));
    let mut settings = config(&dir);
    settings.max_rows = 0;
    assert!(matches!(Db::open(settings).await, Err(Error::Config(_))));
    let mut settings = config(&dir);
    settings.busy_timeout = std::time::Duration::from_secs(60);
    assert!(matches!(Db::open(settings).await, Err(Error::Config(_))));
    let mut settings = config(&dir);
    settings.max_connections = usize::MAX;
    assert!(matches!(Db::open(settings).await, Err(Error::Config(_))));
    let mut settings = config(&dir);
    settings.timeout = std::time::Duration::MAX;
    assert!(matches!(Db::open(settings).await, Err(Error::Config(_))));
    let mut settings = config(&dir);
    settings.timeout = std::time::Duration::from_secs(u64::from(u32::MAX));
    settings.busy_timeout = std::time::Duration::from_millis(i32::MAX as u64 + 1);
    assert!(matches!(Db::open(settings).await, Err(Error::Config(_))));
    let settings = Config::new("https://private.example.com", "never-show-this-token");
    let formatted = format!("{settings:?}");
    assert!(!formatted.contains("private.example.com"));
    assert!(!formatted.contains("never-show-this-token"));
    let error = Error::Database(libsql::Error::ConnectionFailed(
        "never-show-this-token".into(),
    ));
    assert!(!format!("{error:?} {error}").contains("never-show-this-token"));
}

#[tokio::test]
async fn a_missing_directory_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let url = url::Url::from_file_path(dir.path().join("absent").join("db")).unwrap();
    assert!(Db::open(Config::new(url.to_string(), "")).await.is_err());
}
