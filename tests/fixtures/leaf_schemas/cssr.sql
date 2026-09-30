CREATE TABLE cssr_records (
 community_id TEXT NOT NULL, key BLOB NOT NULL, value BLOB NOT NULL,
 PRIMARY KEY (community_id,key)) WITHOUT ROWID;