use super::*;

/// The statement guard is the layer behind the query guard: a SQLite parse of a
/// supported statement never hands the visitor a nested write, so this callback
/// is exercised directly. It must pass a read through and refuse a write.
#[test]
fn the_statement_guard_admits_a_read_and_refuses_a_nested_write() {
    let tables = BTreeSet::new();
    let mut scoper = Scoper {
        tables: &tables,
        param_count: 0,
        used: BTreeSet::new(),
    };
    let mut query = parse("SELECT 1").expect("query").remove(0);
    assert!(matches!(
        scoper.pre_visit_statement(&mut query),
        ControlFlow::Continue(())
    ));
    let mut write = parse("INSERT INTO t (a) VALUES (?1)")
        .expect("insert")
        .remove(0);
    assert!(matches!(
        scoper.pre_visit_statement(&mut write),
        ControlFlow::Break(Error::Sql("nested writes are not supported"))
    ));
}
