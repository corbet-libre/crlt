
CREATE TABLE cgrd_snapshots (
    community_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('settings', 'schema')),
    revision INTEGER NOT NULL CHECK (revision > 0),
    policy_epoch INTEGER NOT NULL CHECK (policy_epoch >= 0),
    cose BLOB NOT NULL,
    PRIMARY KEY (community_id, kind)
) WITHOUT ROWID;
