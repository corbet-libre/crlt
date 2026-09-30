# Leaf schema fixtures

SQL snapshots from the published leaf sources below, captured on 2026-09-30.
The migration regression applies them together and reopens the database without
adding leaf dependencies to crlt. Refresh snapshots explicitly when those
schemas change; this tests these revisions, not downstream dependency adoption.

| Fixture | Source |
|---|---|
| `crgs.sql` | [crgs `f6c19af` / src/schema.sql](https://github.com/corbet-foss/crgs/blob/f6c19af1e8f1a55e31aec58c3dbf3f4fe1074f7b/src/schema.sql) |
| `clbs.sql` | [clbs `32b098c` / src/schema.sql](https://github.com/corbet-foss/clbs/blob/32b098c724ed6d3c7c3d9f4a7243f1032887c68c/src/schema.sql) |
| `cnrl.sql` | [cnrl `8523f04` / src/schema.sql](https://github.com/corbet-foss/cnrl/blob/8523f046c2b4ca27797c691ac001a16d2b12a54d/src/schema.sql) |
| `crbk.sql` | [crbk `cb93c06` / src/storage/schema.sql](https://github.com/corbet-foss/crbk/blob/cb93c06eba8da8a01bbc18eeba4be4844d8df52e/src/storage/schema.sql) |
| `cpky.sql` | [cpky `eae28c0` / LIBSQL_SCHEMA](https://github.com/corbet-foss/cpky/blob/eae28c04116642816e72efd37a35f7e13f2f255a/src/libsql_store.rs) |
| `cpns.sql` | [cpns `23968a9` / SCHEMA](https://github.com/corbet-foss/cpns/blob/23968a9fa338791e70078c6070a6f1f3cf185ff7/src/server/libsql.rs) |
| `csgn.sql` | [csgn `d7203f3` / SCHEMA](https://github.com/corbet-foss/csgn/blob/d7203f310c3c129c3f67ef4ef3e34ccf0ae8c93b/src/database.rs) |
| `cgrd.sql` | [cgrd `8a3fe79` / SCHEMA](https://github.com/corbet-foss/cgrd/blob/8a3fe79680478d18ba730e7ba8faa0b1b95e1138/src/storage/libsql.rs) |
| `cssr.sql` | [cssr `736db98` / SCHEMA](https://github.com/corbet-foss/cssr/blob/736db98ad6ba272dd32b8f3d6e20f2e8f54da8bb/src/lib.rs) |
| `cpsd_schema.sql` | [cpsd `b60f2bb` / SCHEMA](https://github.com/corbet-foss/cpsd/blob/b60f2bbff04dbe0a93cb41992006b06dafc798c3/src/storage.rs) |
| `cpsd_issuance_schema.sql` | [cpsd `b60f2bb` / ISSUANCE_SCHEMA](https://github.com/corbet-foss/cpsd/blob/b60f2bbff04dbe0a93cb41992006b06dafc798c3/src/storage.rs) |
| `cpsd_issuance_uniqueness_schema.sql` | [cpsd `b60f2bb` / ISSUANCE_UNIQUENESS_SCHEMA](https://github.com/corbet-foss/cpsd/blob/b60f2bbff04dbe0a93cb41992006b06dafc798c3/src/storage.rs) |
