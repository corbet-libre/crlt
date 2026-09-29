//! Real libSQL round trips through the public scoped API.

mod common;

use std::time::Duration;

use common::{MIGRATIONS, config, fixture};
use crlt::{Community, Db, Error, Value, params};

async fn insert(community: &Community, id: &str, handle: &str) {
    community
        .execute(
            "INSERT INTO members (id, handle) VALUES (?1, ?2)",
            params![id, handle],
        )
        .await
        .unwrap();
}

async fn count(community: &Community) -> i64 {
    community
        .query("SELECT count(*) FROM members", ())
        .await
        .unwrap()[0]
        .get_i64(0)
        .unwrap()
}

#[tokio::test]
async fn omitted_filters_and_same_ids_remain_isolated() {
    let (_dir, db) = fixture().await;
    let a = db.community("a").unwrap();
    let b = db.community("b").unwrap();
    insert(&a, "same", "alice").await;
    insert(&b, "same", "bob").await;
    assert_eq!(
        a.query("SELECT community_id, handle FROM members", ())
            .await
            .unwrap()[0]
            .get_str(1)
            .unwrap(),
        "alice"
    );
    assert_eq!(
        b.query("SELECT handle FROM members", ()).await.unwrap()[0]
            .get_str(0)
            .unwrap(),
        "bob"
    );
    assert!(
        a.query("SELECT * FROM members WHERE community_id = ?1", ["b"])
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        a.execute(
            "UPDATE members SET handle = ?1 WHERE id = ?2 OR 1 = 1",
            params!["updated", "absent"]
        )
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        b.query("SELECT handle FROM members", ()).await.unwrap()[0]
            .get_str(0)
            .unwrap(),
        "bob"
    );
    assert_eq!(a.execute("DELETE FROM members", ()).await.unwrap(), 1);
    assert_eq!(count(&b).await, 1);
}

#[tokio::test]
async fn aliases_joins_subqueries_unions_and_aggregates_are_scoped() {
    let (_dir, db) = fixture().await;
    let a = db.community("a").unwrap();
    let b = db.community("b").unwrap();
    for (scope, handle) in [(&a, "alice"), (&b, "bob")] {
        insert(scope, "same", handle).await;
        scope
            .execute(
                "INSERT INTO notes (member_id, body) VALUES (?1, ?2)",
                params!["same", handle],
            )
            .await
            .unwrap();
    }
    let queries = [
        "SELECT n.body FROM members AS m JOIN notes AS n ON n.member_id = m.id",
        "SELECT n.body FROM members AS m LEFT JOIN notes AS n ON n.member_id = m.id",
        "SELECT handle FROM members WHERE id IN (SELECT member_id FROM notes)",
        "SELECT (SELECT body FROM notes WHERE member_id = members.id) FROM members",
        "SELECT handle FROM members WHERE EXISTS (SELECT 1 FROM notes WHERE notes.member_id = members.id)",
        "SELECT handle FROM members UNION SELECT body FROM notes",
        "SELECT m.handle FROM (SELECT handle FROM members) AS m",
    ];
    for sql in queries {
        let rows = a
            .query(sql, ())
            .await
            .unwrap_or_else(|e| panic!("{sql}: {e:?}"));
        assert_eq!(rows.len(), 1, "{sql}");
        assert_eq!(rows[0].get_str(0).unwrap(), "alice", "{sql}");
    }
    assert_eq!(
        a.query("SELECT count(*), max(handle) FROM members", ())
            .await
            .unwrap()[0]
            .get_i64(0)
            .unwrap(),
        1
    );
    assert_eq!(
        a.execute(
            "UPDATE members SET handle = (SELECT body FROM notes WHERE member_id = ?1)",
            ["same"]
        )
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        a.execute(
            "DELETE FROM notes WHERE member_id IN (SELECT id FROM members)",
            ()
        )
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        b.query("SELECT body FROM notes", ()).await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn parameters_and_community_keys_are_data_and_case_sensitive() {
    let (_dir, db) = fixture().await;
    let keys = ["a", "A", "a' OR 1=1 --", "日本語🦀", " whitespace "];
    for key in keys {
        let community = db.community(key).unwrap();
        insert(&community, "same", "Robert'); DROP TABLE members; --").await;
        let rows = community
            .query(
                "SELECT community_id, handle FROM members WHERE id = ?1 OR id = ?1",
                ["same"],
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get_str(0).unwrap(), key);
        assert_eq!(
            rows[0].get_str(1).unwrap(),
            "Robert'); DROP TABLE members; --"
        );
    }
    assert!(db.community("").is_err());
    assert!(db.community("a\0b").is_err());
}

#[tokio::test]
async fn inserts_cannot_choose_community_and_conflicts_cannot_reach_another_scope() {
    let (_dir, db) = fixture().await;
    let a = db.community("a").unwrap();
    let b = db.community("b").unwrap();
    insert(&a, "same", "same").await;
    insert(&b, "same", "same").await;
    assert!(
        a.execute(
            "INSERT INTO members (id, handle) VALUES (?1, ?2)",
            params!["same", "duplicate"]
        )
        .await
        .is_err()
    );
    let writes = [
        "INSERT INTO members (community_id, id, handle) VALUES ('b', 'evil', 'evil')",
        "INSERT INTO members (COMMUNITY_ID, id, handle) VALUES ('b', 'evil', 'evil')",
        "UPDATE members SET community_id = 'b'",
        "UPDATE members SET (community_id, handle) = ('b', 'evil')",
        "INSERT OR REPLACE INTO members (id, handle) VALUES ('same', 'evil')",
        "UPDATE OR REPLACE members SET handle = 'evil'",
        "INSERT INTO members (id, handle) VALUES ('same', 'evil') ON CONFLICT DO UPDATE SET handle = 'evil'",
        "INSERT INTO members SELECT * FROM members",
    ];
    for sql in writes {
        assert!(
            matches!(a.execute(sql, ()).await, Err(Error::Sql(_))),
            "{sql}"
        );
    }
    assert_eq!(count(&a).await, 1);
    assert_eq!(count(&b).await, 1);
}

#[tokio::test]
async fn rejects_unscoped_sql_escape_hatches() {
    let (_dir, db) = fixture().await;
    let a = db.community("a").unwrap();
    insert(&a, "one", "alice").await;
    let statements = [
        "SELECT * FROM main.members",
        "SELECT * FROM sqlite_schema",
        "SELECT * FROM __crlt_migrations",
        "SELECT * FROM pragma_table_info('members')",
        "SELECT load_extension('extension')",
        "SELECT changes()",
        "SELECT * FROM members; SELECT * FROM members",
        "SELECT (SELECT name FROM sqlite_schema) FROM members",
        "WITH m AS (SELECT * FROM members) SELECT * FROM m",
        "SELECT * FROM members UNION SELECT * FROM sqlite_schema",
        "SELECT * FROM members UNION TABLE members",
        "ATTACH DATABASE ':memory:' AS other",
        "PRAGMA foreign_keys = OFF",
        "BEGIN",
        "COMMIT",
        "ROLLBACK",
        "CREATE TABLE bad (id INTEGER)",
        "DELETE FROM members; COMMIT",
        "",
        "-- only a comment",
    ];
    for sql in statements {
        assert!(
            matches!(a.query(sql, ()).await, Err(Error::Sql(_))),
            "{sql}"
        );
    }
    assert_eq!(count(&a).await, 1);
}

#[tokio::test]
async fn numbered_parameters_reject_gaps_missing_named_and_extra_values() {
    let (_dir, db) = fixture().await;
    let a = db.community("a").unwrap();
    for sql in [
        "SELECT ?",
        "SELECT ?0",
        "SELECT ?2",
        "SELECT :id",
        "SELECT @id",
        "SELECT $id",
    ] {
        assert!(
            matches!(a.query(sql, ["x"]).await, Err(Error::Sql(_))),
            "{sql}"
        );
    }
    assert!(a.query("SELECT ?1", ()).await.is_err());
    assert!(a.query("SELECT 1", ["extra"]).await.is_err());
    assert!(
        a.query("SELECT :id", libsql::named_params! {":id": "x"})
            .await
            .is_err()
    );
    let rows = a
        .query(
            "SELECT ?2, ?1, ?2, '?1 is a literal'",
            params!["first", "second"],
        )
        .await
        .unwrap();
    assert_eq!(rows[0].get_str(0).unwrap(), "second");
    assert_eq!(rows[0].get_str(1).unwrap(), "first");
    assert_eq!(rows[0].get_str(3).unwrap(), "?1 is a literal");
}

#[tokio::test]
async fn owns_all_row_values_after_cursor_and_pool_reuse() {
    let (_dir, db) = fixture().await;
    let a = db.community("a").unwrap();
    a.execute("INSERT INTO members (id, handle, score, payload) VALUES (?1, ?2, ?3, ?4), (?5, ?6, ?7, ?8)",
        params!["a", "first", i64::MIN, vec![0_u8, 255], "b", "second", i64::MAX, Value::Null]).await.unwrap();
    let rows = a
        .query(
            "SELECT id, handle, score, payload FROM members ORDER BY id",
            (),
        )
        .await
        .unwrap();
    a.execute("DELETE FROM members", ()).await.unwrap();
    assert_eq!(rows[0].get_str(1).unwrap(), "first");
    assert_eq!(rows[1].get_str(1).unwrap(), "second");
    assert_eq!(rows[0].get_i64(2).unwrap(), i64::MIN);
    assert_eq!(rows[1].get_i64(2).unwrap(), i64::MAX);
    assert_eq!(rows[0].get_value(3).unwrap(), &Value::Blob(vec![0, 255]));
    assert_eq!(rows[1].get_value(3).unwrap(), &Value::Null);
    assert_eq!(rows[0].columns(), &["id", "handle", "score", "payload"]);
    assert_eq!(rows[0].values().len(), 4);
    assert!(matches!(rows[0].get_str(2), Err(Error::Column)));
    assert!(matches!(rows[0].get_i64(1), Err(Error::Column)));
    assert!(matches!(rows[0].get_value(99), Err(Error::Column)));
}

#[tokio::test]
async fn commit_rollback_drop_and_failed_commit_are_atomic() {
    let (_dir, db) = fixture().await;
    let a = db.community("a").unwrap();
    let mut tx = db.tx("a").await.unwrap();
    tx.execute(
        "INSERT INTO members (id, handle) VALUES ('committed', 'committed')",
        (),
    )
    .await
    .unwrap();
    assert_eq!(
        tx.query("SELECT count(*) FROM members", ()).await.unwrap()[0]
            .get_i64(0)
            .unwrap(),
        1
    );
    tx.explain("SELECT * FROM members WHERE id = ?1", ["committed"])
        .await
        .unwrap()
        .assert_indexed()
        .unwrap();
    tx.commit().await.unwrap();
    let mut tx = a.tx().await.unwrap();
    tx.execute("DELETE FROM members", ()).await.unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(count(&a).await, 1);
    let mut tx = a.tx().await.unwrap();
    tx.execute("DELETE FROM members", ()).await.unwrap();
    drop(tx);
    assert_eq!(count(&a).await, 1);
    let mut tx = a.tx().await.unwrap();
    tx.execute(
        "INSERT INTO members (id, handle) VALUES ('lost', 'lost')",
        (),
    )
    .await
    .unwrap();
    assert!(
        tx.execute(
            "INSERT INTO members (id, handle) VALUES ('committed', 'duplicate')",
            ()
        )
        .await
        .is_err()
    );
    assert!(matches!(tx.commit().await, Err(Error::TransactionAborted)));
    assert_eq!(count(&a).await, 1);
}

#[tokio::test]
async fn returning_queries_and_empty_results() {
    let (_dir, db) = fixture().await;
    let a = db.community("a").unwrap();
    let mut tx = a.tx().await.unwrap();
    let rows = tx
        .query(
            "INSERT INTO members (id, handle) VALUES ('x', 'first') RETURNING community_id, handle",
            (),
        )
        .await
        .unwrap();
    assert_eq!(rows[0].get_str(0).unwrap(), "a");
    let rows = tx
        .query("UPDATE members SET handle = 'second' RETURNING handle", ())
        .await
        .unwrap();
    assert_eq!(rows[0].get_str(0).unwrap(), "second");
    let rows = tx
        .query("DELETE FROM members RETURNING handle", ())
        .await
        .unwrap();
    assert_eq!(rows[0].get_str(0).unwrap(), "second");
    tx.commit().await.unwrap();
    assert!(
        a.query("SELECT * FROM members", ())
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        a.execute("DELETE FROM members WHERE id = 'missing'", ())
            .await
            .unwrap(),
        0
    );
    assert!(a.execute("SELECT * FROM members", ()).await.is_err());
    assert!(a.query("DELETE FROM members", ()).await.is_err());
}

#[tokio::test]
async fn foreign_keys_and_cascades_cannot_cross_communities() {
    let (_dir, db) = fixture().await;
    let a = db.community("a").unwrap();
    let b = db.community("b").unwrap();
    insert(&b, "same", "bob").await;
    assert!(
        a.execute(
            "INSERT INTO notes (member_id, body) VALUES ('same', 'orphan')",
            ()
        )
        .await
        .is_err()
    );
    insert(&a, "same", "alice").await;
    for scope in [&a, &b] {
        scope
            .execute(
                "INSERT INTO notes (member_id, body) VALUES ('same', 'note')",
                (),
            )
            .await
            .unwrap();
    }
    a.execute("DELETE FROM members", ()).await.unwrap();
    assert!(a.query("SELECT * FROM notes", ()).await.unwrap().is_empty());
    assert_eq!(b.query("SELECT * FROM notes", ()).await.unwrap().len(), 1);
}

#[tokio::test]
async fn query_plan_helper_confirms_composite_indexes() {
    let (_dir, db) = fixture().await;
    let a = db.community("a").unwrap();
    let cases = [
        (
            "SELECT * FROM members WHERE id = ?1",
            "PRIMARY KEY",
            Value::Text("x".into()),
        ),
        (
            "SELECT * FROM members WHERE handle = ?1",
            "INDEX",
            Value::Text("x".into()),
        ),
        // This projection is covered by the score index, avoiding a table lookup.
        (
            "SELECT score FROM members WHERE score = ?1",
            "members_score",
            Value::Integer(0),
        ),
    ];
    for (sql, expected, value) in cases {
        let plan = a.explain(sql, [value]).await.unwrap();
        plan.assert_indexed().unwrap();
        assert!(
            plan.details()
                .iter()
                .any(|d| d.contains("SEARCH") && d.contains(expected)),
            "{sql}: {plan:?}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn pool_wait_timeout_does_not_reuse_an_active_transaction() {
    let (_dir, db) = fixture().await;
    let a = db.community("a").unwrap();
    let tx = a.tx().await.unwrap();
    assert!(matches!(
        a.query("SELECT * FROM members", ()).await,
        Err(Error::Timeout)
    ));
    tx.rollback().await.unwrap();
    assert_eq!(count(&a).await, 0);
}

#[tokio::test(start_paused = true)]
async fn expired_transactions_cannot_commit_and_can_be_rolled_back() {
    let (_dir, db) = fixture().await;
    let a = db.community("a").unwrap();
    let mut tx = a.tx().await.unwrap();
    tx.execute(
        "INSERT INTO members (id, handle) VALUES ('lost', 'lost')",
        (),
    )
    .await
    .unwrap();
    tokio::time::advance(Duration::from_secs(11)).await;
    assert!(matches!(tx.commit().await, Err(Error::Timeout)));
    assert_eq!(count(&a).await, 0);
    let mut tx = a.tx().await.unwrap();
    tokio::time::advance(Duration::from_secs(11)).await;
    assert!(matches!(
        tx.query("SELECT * FROM members", ()).await,
        Err(Error::Timeout)
    ));
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn cancellation_drops_the_transaction_and_releases_the_pool() {
    let (_dir, db) = fixture().await;
    let a = db.community("a").unwrap();
    let worker = a.clone();
    let (ready, wait) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut tx = worker.tx().await.unwrap();
        tx.execute(
            "INSERT INTO members (id, handle) VALUES ('lost', 'lost')",
            (),
        )
        .await
        .unwrap();
        ready.send(()).unwrap();
        std::future::pending::<()>().await;
        tx.commit().await.unwrap();
    });
    wait.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(count(&a).await, 0);
}

#[tokio::test]
async fn pooled_file_connections_share_committed_data() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = config(&dir);
    settings.max_connections = 3;
    let db = Db::open(settings).await.unwrap();
    db.migrate(MIGRATIONS).await.unwrap();
    let a = db.community("a").unwrap();
    insert(&a, "old", "old").await;
    let mut tx = a.tx().await.unwrap();
    tx.execute("INSERT INTO members (id, handle) VALUES ('new', 'new')", ())
        .await
        .unwrap();
    assert_eq!(count(&a).await, 1);
    tx.commit().await.unwrap();
    assert_eq!(count(&a).await, 2);
}

#[tokio::test]
async fn row_limit_errors_poison_writes_with_returning() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = config(&dir);
    settings.max_rows = 1;
    let db = Db::open(settings).await.unwrap();
    db.migrate(MIGRATIONS).await.unwrap();
    let a = db.community("a").unwrap();
    let mut tx = a.tx().await.unwrap();
    assert!(matches!(
        tx.query(
            "INSERT INTO members (id, handle) VALUES ('one', 'one'), ('two', 'two') RETURNING id",
            ()
        )
        .await,
        Err(Error::RowLimit)
    ));
    assert!(matches!(tx.commit().await, Err(Error::TransactionAborted)));
    assert_eq!(count(&a).await, 0);
    insert(&a, "one", "one").await;
    insert(&a, "two", "two").await;
    assert!(matches!(
        a.query("SELECT * FROM members", ()).await,
        Err(Error::RowLimit)
    ));
    assert_eq!(count(&a).await, 2);
}
