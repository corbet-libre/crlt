
CREATE TABLE cpky_passkeys (
    community_id TEXT NOT NULL,
    credential_id BLOB NOT NULL,
    member TEXT NOT NULL,
    record BLOB NOT NULL,
    PRIMARY KEY (community_id, member, credential_id)
) WITHOUT ROWID;
CREATE UNIQUE INDEX cpky_passkeys_credential ON cpky_passkeys (community_id, credential_id);
