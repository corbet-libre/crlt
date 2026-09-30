CREATE TABLE cpns_pins (
    community_id TEXT NOT NULL,
    member_id TEXT NOT NULL CHECK (length(member_id) > 0),
    field_id TEXT NOT NULL CHECK (length(field_id) > 0),
    fingerprint BLOB NOT NULL CHECK (typeof(fingerprint) = 'blob' AND length(fingerprint) = 32),
    revision INTEGER NOT NULL CHECK (typeof(revision) = 'integer' AND revision > 0),
    PRIMARY KEY (community_id, member_id, field_id)
) WITHOUT ROWID