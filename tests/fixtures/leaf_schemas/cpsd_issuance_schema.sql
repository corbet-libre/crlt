
        CREATE TABLE cpsd_issuer_tags (
            community_id TEXT NOT NULL,
            person BLOB NOT NULL CHECK (length(person) = 32),
            tag BLOB NOT NULL CHECK (length(tag) = 48),
            PRIMARY KEY (community_id, person)
        ) WITHOUT ROWID;
    