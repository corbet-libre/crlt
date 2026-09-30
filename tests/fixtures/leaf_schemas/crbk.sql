CREATE TABLE rulebook_revision (
    community_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    announced_at INTEGER NOT NULL CHECK (announced_at >= 0),
    effective_at INTEGER NOT NULL CHECK (effective_at >= announced_at),
    notice_seconds INTEGER NOT NULL CHECK (notice_seconds >= 0),
    policy_epoch INTEGER NOT NULL CHECK (policy_epoch >= 0),
    PRIMARY KEY (community_id, revision)
) WITHOUT ROWID;
CREATE UNIQUE INDEX rulebook_effective ON rulebook_revision (community_id, effective_at);

CREATE TABLE setting_catalog (
    community_id TEXT NOT NULL, revision INTEGER NOT NULL,
    entry_key TEXT NOT NULL, payload TEXT NOT NULL,
    PRIMARY KEY (community_id, revision, entry_key),
    FOREIGN KEY (community_id, revision) REFERENCES rulebook_revision (community_id, revision)
) WITHOUT ROWID;
CREATE TABLE platform_value (
    community_id TEXT NOT NULL, revision INTEGER NOT NULL,
    entry_key TEXT NOT NULL, payload TEXT NOT NULL,
    PRIMARY KEY (community_id, revision, entry_key),
    FOREIGN KEY (community_id, revision) REFERENCES rulebook_revision (community_id, revision)
) WITHOUT ROWID;
CREATE TABLE template (
    community_id TEXT NOT NULL, revision INTEGER NOT NULL,
    entry_key TEXT NOT NULL, payload TEXT NOT NULL,
    PRIMARY KEY (community_id, revision, entry_key),
    FOREIGN KEY (community_id, revision) REFERENCES rulebook_revision (community_id, revision)
) WITHOUT ROWID;
CREATE TABLE template_value (
    community_id TEXT NOT NULL, revision INTEGER NOT NULL,
    entry_key TEXT NOT NULL, payload TEXT NOT NULL,
    PRIMARY KEY (community_id, revision, entry_key),
    FOREIGN KEY (community_id, revision) REFERENCES rulebook_revision (community_id, revision)
) WITHOUT ROWID;
CREATE TABLE community_template (
    community_id TEXT NOT NULL, revision INTEGER NOT NULL,
    entry_key TEXT NOT NULL CHECK (entry_key = 'pin'), payload TEXT NOT NULL,
    PRIMARY KEY (community_id, revision, entry_key),
    FOREIGN KEY (community_id, revision) REFERENCES rulebook_revision (community_id, revision)
) WITHOUT ROWID;
CREATE TABLE community_value (
    community_id TEXT NOT NULL, revision INTEGER NOT NULL,
    entry_key TEXT NOT NULL, payload TEXT NOT NULL,
    PRIMARY KEY (community_id, revision, entry_key),
    FOREIGN KEY (community_id, revision) REFERENCES rulebook_revision (community_id, revision)
) WITHOUT ROWID;
