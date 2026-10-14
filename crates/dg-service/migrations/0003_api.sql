CREATE TABLE worker_status (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1),
    ready INTEGER NOT NULL CHECK(ready IN (0,1)),
    observed_at_ms INTEGER NOT NULL
);
CREATE TABLE deletions (
    kind TEXT NOT NULL CHECK(kind IN ('submission','job')),
    id TEXT NOT NULL, PRIMARY KEY(kind,id)
);
PRAGMA user_version=3;
