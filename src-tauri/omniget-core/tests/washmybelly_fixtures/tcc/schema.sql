-- `access` table of ~/Library/Application Support/com.apple.TCC/TCC.db on
-- macOS 14 Sonoma / 15 Sequoia (column list as the system creates it).
CREATE TABLE admin (key TEXT PRIMARY KEY NOT NULL, value INTEGER NOT NULL);
CREATE TABLE policies (id INTEGER NOT NULL PRIMARY KEY, bundle_id TEXT NOT NULL, uuid TEXT NOT NULL, display TEXT NOT NULL, UNIQUE (bundle_id, uuid));
CREATE TABLE access (
    service TEXT NOT NULL,
    client TEXT NOT NULL,
    client_type INTEGER NOT NULL,
    auth_value INTEGER NOT NULL,
    auth_reason INTEGER NOT NULL,
    auth_version INTEGER NOT NULL,
    csreq BLOB,
    policy_id INTEGER,
    indirect_object_identifier_type INTEGER,
    indirect_object_identifier TEXT NOT NULL DEFAULT 'UNUSED',
    indirect_object_code_identity BLOB,
    flags INTEGER,
    last_modified INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER)),
    pid INTEGER,
    pid_version INTEGER,
    boot_uuid TEXT NOT NULL DEFAULT 'UNUSED',
    last_reminded INTEGER NOT NULL DEFAULT (CAST(strftime('%s','now') AS INTEGER)),
    PRIMARY KEY (service, client, client_type, indirect_object_identifier),
    FOREIGN KEY (policy_id) REFERENCES policies(id) ON DELETE CASCADE ON UPDATE CASCADE
);
