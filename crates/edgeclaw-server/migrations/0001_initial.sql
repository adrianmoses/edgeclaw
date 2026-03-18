CREATE TABLE IF NOT EXISTS users (
    id          TEXT PRIMARY KEY,
    created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS messages (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id     TEXT    NOT NULL REFERENCES users(id),
    role        TEXT    NOT NULL,
    content     TEXT    NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS skills (
    user_id     TEXT    NOT NULL REFERENCES users(id),
    name        TEXT    NOT NULL,
    url         TEXT    NOT NULL,
    tools       TEXT    NOT NULL,
    added_at    INTEGER NOT NULL,
    PRIMARY KEY (user_id, name)
);

CREATE TABLE IF NOT EXISTS credentials (
    user_id           TEXT    NOT NULL REFERENCES users(id),
    skill_name        TEXT    NOT NULL,
    provider          TEXT    NOT NULL,
    access_token_enc  BLOB    NOT NULL,
    refresh_token_enc BLOB,
    expires_at        INTEGER,
    scopes            TEXT    NOT NULL,
    user_salt         BLOB    NOT NULL,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    PRIMARY KEY (user_id, skill_name, provider)
);

CREATE TABLE IF NOT EXISTS scheduled_tasks (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id     TEXT    NOT NULL REFERENCES users(id),
    name        TEXT    NOT NULL,
    cron        TEXT,
    run_at      INTEGER,
    payload     TEXT    NOT NULL,
    last_run    INTEGER,
    enabled     INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS pending_approvals (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id     TEXT    NOT NULL REFERENCES users(id),
    tool_call   TEXT    NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS prefs (
    user_id     TEXT    NOT NULL REFERENCES users(id),
    key         TEXT    NOT NULL,
    value       TEXT    NOT NULL,
    PRIMARY KEY (user_id, key)
);

CREATE TABLE IF NOT EXISTS memory_facts (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id     TEXT    NOT NULL REFERENCES users(id),
    key         TEXT    NOT NULL,
    value       TEXT    NOT NULL,
    tags        TEXT,
    created_at  INTEGER NOT NULL,
    UNIQUE(user_id, key)
);

CREATE INDEX IF NOT EXISTS idx_messages_user_created
    ON messages(user_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_tasks_run_at
    ON scheduled_tasks(run_at) WHERE run_at IS NOT NULL AND enabled = 1;
