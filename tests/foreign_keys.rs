//! Namespace-changing foreign keys and safe actions on real libSQL files.

mod common;

use common::{MIGRATIONS, empty, fixture};
use crlt::{Community, Error, Migration};

const TRANSFER_SCHEMA: &str = "
CREATE TABLE parent (
    community_id TEXT NOT NULL, id TEXT NOT NULL,
    PRIMARY KEY (community_id, id)
) WITHOUT ROWID;
CREATE TABLE child (
    community_id TEXT NOT NULL DEFAULT 'b', id TEXT NOT NULL,
    parent_id TEXT NOT NULL DEFAULT 'anchor', secret TEXT,
    PRIMARY KEY (community_id, id),
    FOREIGN KEY (community_id, parent_id) REFERENCES parent(community_id, id)
        ON DELETE SET DEFAULT
) WITHOUT ROWID;
";

#[tokio::test]
async fn real_libsql_reproduces_namespace_transfer() {
    for (event, mutation) in [
        (
            "DELETE",
            "DELETE FROM parent WHERE id = 'root' AND community_id = 'a'",
        ),
        (
            "UPDATE",
            "UPDATE parent SET id = 'changed' WHERE id = 'root' AND community_id = 'a'",
        ),
    ] {
        // This raw connection is only the control proving why the DDL is unsafe.
        let database = libsql::Builder::new_local(":memory:")
            .build()
            .await
            .unwrap();
        let conn = database.connect().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON")
            .await
            .unwrap();
        conn.execute_batch(&TRANSFER_SCHEMA.replace("ON DELETE", &format!("ON {event}")))
            .await
            .unwrap();
        conn.execute(
            "INSERT INTO parent VALUES ('a', 'root'), ('b', 'anchor')",
            (),
        )
        .await
        .unwrap();
        conn.execute(
            "INSERT INTO child VALUES ('a', 'unique', 'root', 'private-to-a')",
            (),
        )
        .await
        .unwrap();
        conn.execute(mutation, ()).await.unwrap();
        let mut rows = conn
            .query(
                "SELECT secret FROM child WHERE id = 'unique' AND community_id = 'b'",
                (),
            )
            .await
            .unwrap();
        assert_eq!(
            rows.next()
                .await
                .unwrap()
                .unwrap()
                .get::<String>(0)
                .unwrap(),
            "private-to-a"
        );
    }
}

#[tokio::test]
async fn namespace_transfer_reproduction_is_refused_by_migration() {
    let (_dir, db) = empty().await;
    for event in ["DELETE", "UPDATE"] {
        let sql = TRANSFER_SCHEMA.replace("ON DELETE", &format!("ON {event}"));
        assert!(
            matches!(
                db.migrate(&[Migration::new(1, "namespace transfer", &sql)])
                    .await,
                Err(Error::Schema(_))
            ),
            "ON {event} SET DEFAULT must be refused before scoped use"
        );
        assert!(
            db.community("a")
                .unwrap()
                .query("SELECT * FROM child", ())
                .await
                .is_err()
        );
    }
    // The rejected DDL and ledger must roll back together.
    assert_eq!(db.migrate(MIGRATIONS).await.unwrap(), 1);
}

#[tokio::test]
async fn rejected_namespace_transfer_preserves_existing_schema_and_history() {
    let (_dir, db) = fixture().await;
    let a = db.community("a").unwrap();
    a.execute(
        "INSERT INTO members (id, handle) VALUES ('same', 'alice')",
        (),
    )
    .await
    .unwrap();
    let invalid = Migration::new(2, "namespace transfer", TRANSFER_SCHEMA);
    assert!(matches!(
        db.migrate(&[MIGRATIONS[0], invalid]).await,
        Err(Error::Schema(_))
    ));
    assert_eq!(db.migrate(MIGRATIONS).await.unwrap(), 0);
    assert_eq!(
        a.query("SELECT handle FROM members WHERE id = 'same'", ())
            .await
            .unwrap()[0]
            .get_str(0)
            .unwrap(),
        "alice"
    );
}

#[tokio::test]
async fn rejects_both_unsafe_actions_on_delete_and_update_without_defaults() {
    for event in ["DELETE", "UPDATE"] {
        for action in ["SET DEFAULT", "SET NULL"] {
            for foreign_keys in [
                format!("FOREIGN KEY (community_id, parent_id) REFERENCES parent (community_id, id) ON {event} {action}"),
                format!("FOREIGN KEY (community_id, id) REFERENCES parent (community_id, id) ON DELETE CASCADE,
                    FOREIGN KEY (community_id, parent_id) REFERENCES parent (community_id, id) ON {event} {action}"),
                format!("FOREIGN KEY (community_id, parent_id) REFERENCES parent (community_id, id) ON {event} {action},
                    FOREIGN KEY (community_id, id) REFERENCES parent (community_id, id) ON UPDATE RESTRICT"),
            ] {
                let (_dir, db) = empty().await;
                let sql = format!("
                    CREATE TABLE parent (
                        community_id TEXT NOT NULL, id TEXT NOT NULL,
                        PRIMARY KEY (community_id, id)
                    ) WITHOUT ROWID;
                    CREATE TABLE child (
                        community_id TEXT NOT NULL, id TEXT NOT NULL, parent_id TEXT,
                        PRIMARY KEY (community_id, id), {foreign_keys}
                    ) WITHOUT ROWID;
                ");
                assert!(matches!(
                    db.migrate(&[Migration::new(1, "unsafe action", &sql)]).await,
                    Err(Error::Schema("foreign key actions must not replace community_id"))
                ), "{foreign_keys}");
                assert_eq!(db.migrate(MIGRATIONS).await.unwrap(), 1);
            }
        }
    }
}

#[tokio::test]
async fn rejects_inline_quoted_case_variant_namespace_actions() {
    for event in ["delete", "update"] {
        for action in ["set default", "set null"] {
            let (_dir, db) = empty().await;
            let sql = format!(
                r#"
                CREATE TABLE parent ("COMMUNITY_ID" TEXT NOT NULL PRIMARY KEY) WITHOUT ROWID;
                CREATE TABLE child (
                    "CoMmUnItY_Id" TEXT NOT NULL PRIMARY KEY
                        REFERENCES parent ("COMMUNITY_ID") on {event} {action}
                ) WITHOUT ROWID;
            "#
            );
            assert!(matches!(
                db.migrate(&[Migration::new(1, "inline action", &sql)])
                    .await,
                Err(Error::Schema(
                    "foreign key actions must not replace community_id"
                ))
            ));
        }
    }
}

#[tokio::test]
async fn rejects_namespace_defaults_even_without_foreign_keys() {
    for default in ["'b'", "('b')", "('b' || '')", "NULL", "0"] {
        let (_dir, db) = empty().await;
        let sql = format!(
            r#"
            CREATE TABLE child (
                "COMMUNITY_ID" TEXT NOT NULL DEFAULT {default}, id TEXT NOT NULL,
                PRIMARY KEY ("COMMUNITY_ID", id)
            ) WITHOUT ROWID;
        "#
        );
        assert!(
            matches!(
                db.migrate(&[Migration::new(1, "namespace default", &sql)])
                    .await,
                Err(Error::Schema("community_id must not have a default"))
            ),
            "{default}"
        );
    }
}

#[tokio::test]
async fn audits_namespace_defaults_after_alter_table() {
    let (_dir, db) = empty().await;
    let sql = "CREATE TABLE child (
        tenant TEXT NOT NULL DEFAULT 'b', id TEXT NOT NULL,
        PRIMARY KEY (tenant, id)
    ) WITHOUT ROWID;
    ALTER TABLE child RENAME COLUMN tenant TO community_id;";
    assert!(matches!(
        db.migrate(&[Migration::new(1, "renamed namespace", sql)])
            .await,
        Err(Error::Schema("community_id must not have a default"))
    ));
    assert_eq!(db.migrate(MIGRATIONS).await.unwrap(), 1);
}

#[tokio::test]
async fn rejects_repeated_namespace_bound_to_a_mutable_parent_column() {
    let (_dir, db) = empty().await;
    let sql = "CREATE TABLE parent (
        community_id TEXT NOT NULL, id TEXT NOT NULL, label TEXT NOT NULL,
        PRIMARY KEY (community_id, id), UNIQUE (community_id, id, label)
    ) WITHOUT ROWID;
    CREATE TABLE child (
        community_id TEXT NOT NULL, id TEXT NOT NULL, parent_id TEXT NOT NULL,
        PRIMARY KEY (community_id, id),
        FOREIGN KEY (community_id, parent_id, community_id)
            REFERENCES parent (community_id, id, label) ON UPDATE CASCADE
    ) WITHOUT ROWID;";
    assert!(matches!(
        db.migrate(&[Migration::new(1, "repeated namespace", sql)])
            .await,
        Err(Error::Schema(
            "community_id must only reference community_id"
        ))
    ));
}

#[tokio::test]
async fn unsafe_persisted_schema_is_refused_before_exposing_tables() {
    let dir = tempfile::tempdir().unwrap();
    let database = libsql::Builder::new_local(dir.path().join("test.db"))
        .build()
        .await
        .unwrap();
    let conn = database.connect().unwrap();
    conn.execute_batch(TRANSFER_SCHEMA).await.unwrap();
    drop(conn);
    drop(database);

    let db = crlt::Db::open(common::config(&dir)).await.unwrap();
    assert!(matches!(db.migrate(&[]).await, Err(Error::Schema(_))));
    for community in ["a", "b"] {
        assert!(matches!(
            db.community(community)
                .unwrap()
                .query("SELECT * FROM child", ())
                .await,
            Err(Error::Sql(_))
        ));
    }
}

fn safe_schema(action: &str) -> String {
    format!(
        "
        CREATE TABLE parent (
            community_id TEXT NOT NULL, id TEXT NOT NULL,
            PRIMARY KEY (community_id, id)
        ) WITHOUT ROWID;
        CREATE TABLE child (
            community_id TEXT NOT NULL, id TEXT NOT NULL,
            parent_id TEXT NOT NULL DEFAULT 'root', secret TEXT NOT NULL,
            PRIMARY KEY (community_id, id),
            FOREIGN KEY (community_id, parent_id) REFERENCES parent (community_id, id)
                ON DELETE {action} ON UPDATE {action}
        ) WITHOUT ROWID;
        CREATE INDEX child_parent ON child (community_id, parent_id);
    "
    )
}

async fn populate(scope: &Community, secret: &str) {
    scope
        .execute("INSERT INTO parent (id) VALUES ('root')", ())
        .await
        .unwrap();
    scope
        .execute(
            "INSERT INTO child (id, secret) VALUES ('same', ?1)",
            [secret],
        )
        .await
        .unwrap();
}

async fn assert_child(scope: &Community, community: &str, parent: &str, secret: &str) {
    let rows = scope
        .query(
            "SELECT community_id, parent_id, secret FROM child WHERE id = 'same'",
            (),
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get_str(0).unwrap(), community);
    assert_eq!(rows[0].get_str(1).unwrap(), parent);
    assert_eq!(rows[0].get_str(2).unwrap(), secret);
}

#[tokio::test]
async fn scoped_deletion_preserves_other_community_for_safe_actions() {
    for action in ["CASCADE", "RESTRICT", "NO ACTION"] {
        let (_dir, db) = empty().await;
        db.migrate(&[Migration::new(1, "safe actions", &safe_schema(action))])
            .await
            .unwrap();
        let a = db.community("a").unwrap();
        let b = db.community("b").unwrap();
        populate(&a, "private-to-a").await;
        populate(&b, "private-to-b").await;
        let sql = "DELETE FROM parent WHERE id = 'root'";
        a.explain(sql, ()).await.unwrap().assert_indexed().unwrap();
        let result = a.execute(sql, ()).await;
        if action == "CASCADE" {
            assert_eq!(result.unwrap(), 1);
            assert!(
                a.query("SELECT * FROM parent", ())
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert!(a.query("SELECT * FROM child", ()).await.unwrap().is_empty());
        } else {
            assert!(matches!(result, Err(Error::Database(_))));
            assert_eq!(
                a.query("SELECT id FROM parent", ()).await.unwrap()[0]
                    .get_str(0)
                    .unwrap(),
                "root"
            );
            assert_child(&a, "a", "root", "private-to-a").await;
        }
        assert_eq!(
            b.query("SELECT id FROM parent", ()).await.unwrap()[0]
                .get_str(0)
                .unwrap(),
            "root"
        );
        assert_child(&b, "b", "root", "private-to-b").await;
    }
}

#[tokio::test]
async fn scoped_update_preserves_both_communities_for_safe_actions() {
    for action in ["CASCADE", "RESTRICT", "NO ACTION"] {
        let (_dir, db) = empty().await;
        db.migrate(&[Migration::new(1, "safe actions", &safe_schema(action))])
            .await
            .unwrap();
        let a = db.community("a").unwrap();
        let b = db.community("b").unwrap();
        populate(&a, "private-to-a").await;
        populate(&b, "private-to-b").await;
        let sql = "UPDATE parent SET id = 'changed' WHERE id = 'root'";
        a.explain(sql, ()).await.unwrap().assert_indexed().unwrap();
        let result = a.execute(sql, ()).await;
        let expected = if action == "CASCADE" {
            assert_eq!(result.unwrap(), 1);
            "changed"
        } else {
            assert!(matches!(result, Err(Error::Database(_))));
            "root"
        };
        assert_eq!(
            a.query("SELECT id FROM parent", ()).await.unwrap()[0]
                .get_str(0)
                .unwrap(),
            expected
        );
        assert_child(&a, "a", expected, "private-to-a").await;
        assert_eq!(
            b.query("SELECT id FROM parent", ()).await.unwrap()[0]
                .get_str(0)
                .unwrap(),
            "root"
        );
        assert_child(&b, "b", "root", "private-to-b").await;

        // Cascades remain safe because the parent namespace is immutable too.
        for table in ["parent", "child"] {
            assert!(matches!(
                a.execute(&format!("UPDATE {table} SET community_id = 'b'"), ())
                    .await,
                Err(Error::Sql(_))
            ));
        }
        assert_child(&a, "a", expected, "private-to-a").await;
        assert_child(&b, "b", "root", "private-to-b").await;
    }
}
