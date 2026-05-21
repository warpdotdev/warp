use diesel::{
    Connection, QueryableByName, RunQueryDsl, SqliteConnection,
    connection::SimpleConnection,
    migration::{Migration, MigrationSource},
    sql_types::{BigInt, Text},
    sqlite::Sqlite,
};
use diesel_migrations::MigrationHarness;
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const HOSTED_RESIDUE_CLEANUP_VERSION: &str = "20260505020000";
type TestResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

struct TempDatabaseDir {
    path: PathBuf,
}

impl TempDatabaseDir {
    fn new() -> io::Result<Self> {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "warper-hosted-residue-migrations-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&path)?;
        Ok(Self { path })
    }

    fn database_path(&self) -> PathBuf {
        self.path.join("warp.sqlite")
    }
}

impl Drop for TempDatabaseDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(QueryableByName)]
struct SqlCount {
    #[diesel(sql_type = BigInt)]
    count: i64,
}

fn setup_database_before_hosted_cleanup(database_path: &Path) -> TestResult<SqliteConnection> {
    let db_url = database_path
        .to_str()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "database path is not UTF-8"))?;
    let mut conn = SqliteConnection::establish(db_url)?;
    conn.batch_execute(
        r#"
        PRAGMA foreign_keys = ON;
        CREATE TABLE IF NOT EXISTS __diesel_schema_migrations (
            version VARCHAR(50) PRIMARY KEY NOT NULL,
            run_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        "#,
    )?;

    let migrations: Vec<Box<dyn Migration<Sqlite>>> = persistence::MIGRATIONS
        .migrations()
        .map_err(|err| io::Error::other(err.to_string()))?;
    let migrations_before_cutoff = migrations
        .into_iter()
        .filter(|migration| {
            migration.name().version().to_string().as_str() < HOSTED_RESIDUE_CLEANUP_VERSION
        })
        .collect::<Vec<_>>();

    conn.run_migrations(&migrations_before_cutoff)
        .map_err(|err| io::Error::other(err.to_string()))?;
    Ok(conn)
}

fn table_exists(conn: &mut SqliteConnection, table_name: &str) -> bool {
    diesel::sql_query(
        "SELECT COUNT(*) AS count FROM sqlite_master WHERE type = 'table' AND name = ?",
    )
    .bind::<Text, _>(table_name)
    .get_result::<SqlCount>(conn)
    .expect("sqlite_master should be queryable")
    .count
        > 0
}

#[test]
fn hosted_cloud_team_billing_account_state_is_deleted_by_migrations() {
    let tempdir = TempDatabaseDir::new().expect("tempdir should be created");
    let database_path = tempdir.database_path();
    let mut conn = setup_database_before_hosted_cleanup(&database_path)
        .expect("database should initialize before hosted cleanup migration");

    conn.batch_execute(
        r#"
        PRAGMA foreign_keys = off;

        INSERT INTO generic_string_objects (id, data)
        VALUES (700, '{"hosted":"cloud-payload"}');

        INSERT INTO object_metadata (
            id,
            is_pending,
            object_type,
            revision_ts,
            client_id,
            shareable_object_id,
            retry_count,
            metadata_last_updated_ts,
            trashed_ts,
            folder_id,
            is_welcome_object,
            creator_uid,
            last_editor_uid,
            current_editor
        )
        VALUES (
            701,
            FALSE,
            'NOTEBOOK',
            123,
            'client-object-701',
            700,
            2,
            456,
            NULL,
            NULL,
            FALSE,
            'hosted-creator',
            'hosted-editor',
            NULL
        );

        INSERT INTO object_permissions (
            id,
            object_metadata_id,
            subject_type,
            subject_id,
            subject_uid,
            permissions_last_updated_at,
            object_guests,
            anyone_with_link_access_level,
            anyone_with_link_source
        )
        VALUES (
            702,
            701,
            'TEAM',
            'team-server-id',
            'team-server-uid',
            789,
            X'01',
            'EDITOR',
            X'02'
        );

        INSERT INTO object_actions (
            id,
            hashed_object_id,
            timestamp,
            action,
            data,
            count,
            oldest_timestamp,
            latest_timestamp,
            pending
        )
        VALUES (
            703,
            'hosted-object-hash',
            CURRENT_TIMESTAMP,
            'SHARE',
            '{"source":"hosted"}',
            1,
            CURRENT_TIMESTAMP,
            CURRENT_TIMESTAMP,
            TRUE
        );

        CREATE TABLE IF NOT EXISTS cloud_objects_refreshes (
            id INTEGER NOT NULL PRIMARY KEY,
            object_type TEXT NOT NULL,
            last_synced_ts BIGINTEGER
        );
        INSERT INTO cloud_objects_refreshes (id, object_type, last_synced_ts)
        VALUES (704, 'NOTEBOOK', 123456);

        CREATE TABLE IF NOT EXISTS teams (
            id integer NOT NULL PRIMARY KEY,
            name TEXT NOT NULL,
            server_uid TEXT NOT NULL UNIQUE
        );
        ALTER TABLE teams ADD COLUMN billing_metadata_json TEXT;
        INSERT INTO teams (id, name, server_uid, billing_metadata_json)
        VALUES (800, 'Hosted Team', 'team-old', '{"plan":"enterprise"}');

        CREATE TABLE IF NOT EXISTS team_settings (
            id INTEGER PRIMARY KEY NOT NULL,
            team_id INTEGER NOT NULL UNIQUE,
            settings_json TEXT NOT NULL,
            FOREIGN KEY (team_id) REFERENCES teams (id)
        );
        INSERT INTO team_settings (id, team_id, settings_json)
        VALUES (801, 800, '{"sharing":"enabled"}');

        CREATE TABLE IF NOT EXISTS team_members (
            id INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
            team_id INTEGER NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
            user_uid TEXT NOT NULL,
            email TEXT NOT NULL,
            role TEXT NOT NULL
        );
        INSERT INTO team_members (id, team_id, user_uid, email, role)
        VALUES (802, 800, 'hosted-user', 'old@example.com', 'admin');

        CREATE TABLE IF NOT EXISTS workspace_teams (
            id INTEGER NOT NULL PRIMARY KEY,
            workspace_uid TEXT NOT NULL,
            team_uid TEXT NOT NULL
        );
        INSERT INTO workspace_teams (id, workspace_uid, team_uid)
        VALUES (803, 'workspace-old', 'team-old');

        CREATE TABLE IF NOT EXISTS users (
            id INTEGER NOT NULL PRIMARY KEY,
            email TEXT,
            firebase_uid TEXT
        );
        INSERT INTO users (id, email, firebase_uid)
        VALUES (900, 'old@example.com', 'firebase-old');

        CREATE TABLE IF NOT EXISTS user_profiles (
            id INTEGER NOT NULL PRIMARY KEY,
            user_id INTEGER NOT NULL,
            profile_json TEXT NOT NULL
        );
        INSERT INTO user_profiles (id, user_id, profile_json)
        VALUES (901, 900, '{"name":"Hosted User"}');

        CREATE TABLE IF NOT EXISTS current_user_information (
            id INTEGER NOT NULL PRIMARY KEY,
            user_id INTEGER NOT NULL,
            email TEXT
        );
        INSERT INTO current_user_information (id, user_id, email)
        VALUES (902, 900, 'old@example.com');

        PRAGMA foreign_keys = on;
        "#,
    )
    .expect("old hosted rows should be seeded");

    for table_name in [
        "cloud_objects_refreshes",
        "object_actions",
        "object_permissions",
        "object_metadata",
        "generic_string_objects",
        "team_members",
        "team_settings",
        "workspace_teams",
        "teams",
        "current_user_information",
        "user_profiles",
        "users",
    ] {
        assert!(
            table_exists(&mut conn, table_name),
            "test seed should create old hosted table: {table_name}"
        );
    }

    conn.run_pending_migrations(persistence::MIGRATIONS)
        .expect("remaining migrations should remove hosted residue");

    for table_name in [
        "cloud_objects_refreshes",
        "object_actions",
        "object_permissions",
        "object_metadata",
        "generic_string_objects",
        "team_members",
        "team_settings",
        "workspace_teams",
        "teams",
        "current_user_information",
        "user_profiles",
        "users",
    ] {
        assert!(
            !table_exists(&mut conn, table_name),
            "stale hosted table should be removed: {table_name}"
        );
    }
}
