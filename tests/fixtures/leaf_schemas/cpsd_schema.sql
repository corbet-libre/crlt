
        CREATE TABLE cpsd_challenges (
            community_id TEXT NOT NULL,
            nonce BLOB NOT NULL CHECK (length(nonce) = 32),
            binding BLOB NOT NULL CHECK (length(binding) = 32),
            expires INTEGER NOT NULL CHECK (expires > 0 AND expires < 1099511627776),
            PRIMARY KEY (community_id, nonce)
        ) WITHOUT ROWID;
        CREATE INDEX cpsd_challenges_expiry ON cpsd_challenges (community_id, expires);
    