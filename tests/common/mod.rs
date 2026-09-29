use crlt::{Config, Db, Migration};
use tempfile::TempDir;

pub const MIGRATIONS: &[Migration<'static>] = &[Migration::new(
    1,
    "members and notes",
    "
    CREATE TABLE members (
        community_id TEXT NOT NULL,
        id TEXT NOT NULL,
        handle TEXT NOT NULL,
        score INTEGER NOT NULL DEFAULT 0,
        payload BLOB,
        PRIMARY KEY (community_id, id),
        UNIQUE (community_id, handle)
    ) WITHOUT ROWID;
    CREATE INDEX members_score ON members (community_id, score);
    CREATE TABLE notes (
        community_id TEXT NOT NULL,
        member_id TEXT NOT NULL,
        body TEXT NOT NULL,
        PRIMARY KEY (community_id, member_id),
        FOREIGN KEY (community_id, member_id) REFERENCES members (community_id, id)
            ON DELETE CASCADE
    ) WITHOUT ROWID;
",
)];

pub fn config(dir: &TempDir) -> Config {
    Config::new(
        url::Url::from_file_path(dir.path().join("test.db"))
            .unwrap()
            .to_string(),
        "",
    )
}

pub async fn empty() -> (TempDir, Db) {
    let dir = tempfile::tempdir().unwrap();
    let db = Db::open(config(&dir)).await.unwrap();
    (dir, db)
}

pub async fn fixture() -> (TempDir, Db) {
    let (dir, db) = empty().await;
    db.migrate(MIGRATIONS).await.unwrap();
    (dir, db)
}
