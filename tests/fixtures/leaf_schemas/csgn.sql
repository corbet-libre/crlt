
CREATE TABLE csgn_state (
    community_id TEXT NOT NULL,
    issuer TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    state BLOB NOT NULL,
    PRIMARY KEY (community_id, issuer)
) WITHOUT ROWID;
