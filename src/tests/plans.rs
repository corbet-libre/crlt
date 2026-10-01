use super::*;

#[tokio::test]
async fn detects_real_table_and_index_scans() {
    let db = libsql::Builder::new_local(":memory:")
        .build()
        .await
        .unwrap();
    let conn = crate::backend::Connection::Local(db.connect().unwrap());
    conn.execute(
        "CREATE TABLE sample (id INTEGER PRIMARY KEY, value TEXT)",
        (),
    )
    .await
    .unwrap();
    conn.execute("INSERT INTO sample VALUES (1, 'one'), (2, 'two')", ())
        .await
        .unwrap();
    let scan = explain(
        &conn,
        "SELECT * FROM sample WHERE value = ?1",
        &[libsql::Value::Text("one".into())],
    )
    .await
    .unwrap();
    assert!(matches!(scan.assert_indexed(), Err(Error::UnindexedQuery)));
    conn.execute("CREATE INDEX by_value ON sample (value)", ())
        .await
        .unwrap();
    let search = explain(
        &conn,
        "SELECT * FROM sample WHERE value = ?1",
        &[libsql::Value::Text("one".into())],
    )
    .await
    .unwrap();
    search.assert_indexed().unwrap();
    assert!(
        search
            .details()
            .iter()
            .any(|d| d.contains("USING COVERING INDEX by_value"))
    );
    let index_scan = explain(&conn, "SELECT value FROM sample INDEXED BY by_value", &[])
        .await
        .unwrap();
    assert!(matches!(
        index_scan.assert_indexed(),
        Err(Error::UnindexedQuery)
    ));
}
