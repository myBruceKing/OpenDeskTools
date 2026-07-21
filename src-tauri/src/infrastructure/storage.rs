use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, ToSql, Transaction, TransactionBehavior};
use thiserror::Error;

const DATABASE_FILE_NAME: &str = "opendesktools.sqlite3";
const FILES_DIRECTORY_NAME: &str = "files";
const LATEST_SCHEMA_VERSION: u32 = 5;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("failed to {operation} at {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("SQLite operation failed: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("storage database lock is poisoned")]
    LockPoisoned,
    #[error("query_i64 only accepts read-only SQL")]
    QueryMustBeReadOnly,
    #[error("storage path must be a non-empty relative path without parent traversal: {0}")]
    InvalidRelativePath(PathBuf),
    #[error("resolved storage path escapes the application data root: {0}")]
    PathEscape(PathBuf),
    #[error("database schema version {found} is newer than the supported version {supported}")]
    UnsupportedSchemaVersion { found: u32, supported: u32 },
    #[error("transaction failed and rollback also failed: {rollback}; original error: {original}")]
    Rollback {
        original: Box<StorageError>,
        rollback: rusqlite::Error,
    },
}

#[derive(Debug)]
pub struct StorageService {
    data_root: PathBuf,
    database_path: PathBuf,
    files_dir: PathBuf,
    connection: Mutex<Connection>,
}

impl StorageService {
    pub fn initialize(data_root: impl AsRef<Path>) -> Result<Self, StorageError> {
        let requested_root = data_root.as_ref();
        create_directory(requested_root, "create application data root")?;

        let data_root = fs::canonicalize(requested_root).map_err(|source| StorageError::Io {
            operation: "resolve application data root",
            path: requested_root.to_path_buf(),
            source,
        })?;
        let files_dir = data_root.join(FILES_DIRECTORY_NAME);
        create_directory(&files_dir, "create managed files directory")?;

        let database_path = data_root.join(DATABASE_FILE_NAME);
        let mut connection = Connection::open(&database_path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        run_migrations(&mut connection)?;

        Ok(Self {
            data_root,
            database_path,
            files_dir,
            connection: Mutex::new(connection),
        })
    }

    pub fn data_root(&self) -> &Path {
        &self.data_root
    }

    pub fn database_path(&self) -> &Path {
        &self.database_path
    }

    pub fn files_dir(&self) -> &Path {
        &self.files_dir
    }

    pub fn migration_version(&self) -> Result<u32, StorageError> {
        let version = self.query_i64(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            &[],
        )?;
        u32::try_from(version).map_err(|_| StorageError::UnsupportedSchemaVersion {
            found: u32::MAX,
            supported: LATEST_SCHEMA_VERSION,
        })
    }

    pub fn resolve_relative_path(
        &self,
        relative_path: impl AsRef<Path>,
    ) -> Result<PathBuf, StorageError> {
        let relative_path = relative_path.as_ref();
        let mut has_component = false;

        for component in relative_path.components() {
            match component {
                Component::Normal(_) => has_component = true,
                Component::Prefix(_)
                | Component::RootDir
                | Component::CurDir
                | Component::ParentDir => {
                    return Err(StorageError::InvalidRelativePath(
                        relative_path.to_path_buf(),
                    ));
                }
            }
        }

        if !has_component {
            return Err(StorageError::InvalidRelativePath(
                relative_path.to_path_buf(),
            ));
        }

        let resolved = self.data_root.join(relative_path);
        let existing_ancestor = nearest_existing_ancestor(&resolved)
            .ok_or_else(|| StorageError::PathEscape(resolved.clone()))?;
        let canonical_ancestor =
            fs::canonicalize(existing_ancestor).map_err(|source| StorageError::Io {
                operation: "resolve managed path ancestor",
                path: existing_ancestor.to_path_buf(),
                source,
            })?;

        if !canonical_ancestor.starts_with(&self.data_root) {
            return Err(StorageError::PathEscape(resolved));
        }

        Ok(resolved)
    }

    pub fn transaction<T, F>(&self, operation: F) -> Result<T, StorageError>
    where
        F: FnOnce(&Transaction<'_>) -> Result<T, StorageError>,
    {
        let mut connection = self.lock_connection()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;

        match operation(&transaction) {
            Ok(value) => {
                transaction.commit()?;
                Ok(value)
            }
            Err(original) => match transaction.rollback() {
                Ok(()) => Err(original),
                Err(rollback) => Err(StorageError::Rollback {
                    original: Box::new(original),
                    rollback,
                }),
            },
        }
    }

    pub(crate) fn read<T, F>(&self, operation: F) -> Result<T, StorageError>
    where
        F: FnOnce(&Connection) -> Result<T, StorageError>,
    {
        let connection = self.lock_connection()?;
        operation(&connection)
    }

    pub fn query_i64(&self, sql: &str, parameters: &[&dyn ToSql]) -> Result<i64, StorageError> {
        let connection = self.lock_connection()?;
        let mut statement = connection.prepare(sql)?;
        if !statement.readonly() {
            return Err(StorageError::QueryMustBeReadOnly);
        }
        Ok(statement.query_row(parameters, |row| row.get(0))?)
    }

    pub fn read_setting(&self, key: &str) -> Result<Option<String>, StorageError> {
        let connection = self.lock_connection()?;
        Ok(connection
            .query_row(
                "SELECT value FROM application_settings WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn write_settings(&self, settings: &[(&str, &str)]) -> Result<(), StorageError> {
        self.transaction(|transaction| {
            let mut statement = transaction.prepare(
                "INSERT INTO application_settings (key, value)
                 VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET
                    value = excluded.value,
                    updated_at = CURRENT_TIMESTAMP",
            )?;

            for (key, value) in settings {
                statement.execute((*key, *value))?;
            }

            drop(statement);
            Ok(())
        })
    }

    fn lock_connection(&self) -> Result<MutexGuard<'_, Connection>, StorageError> {
        self.connection
            .lock()
            .map_err(|_| StorageError::LockPoisoned)
    }
}

fn create_directory(path: &Path, operation: &'static str) -> Result<(), StorageError> {
    fs::create_dir_all(path).map_err(|source| StorageError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    })
}

fn nearest_existing_ancestor(path: &Path) -> Option<&Path> {
    let mut candidate = Some(path);
    while let Some(current) = candidate {
        if current.exists() {
            return Some(current);
        }
        candidate = current.parent();
    }
    None
}

fn run_migrations(connection: &mut Connection) -> Result<(), StorageError> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY NOT NULL,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );",
    )?;

    let current_version: u32 = transaction.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
        [],
        |row| row.get(0),
    )?;
    if current_version > LATEST_SCHEMA_VERSION {
        return Err(StorageError::UnsupportedSchemaVersion {
            found: current_version,
            supported: LATEST_SCHEMA_VERSION,
        });
    }

    if current_version < 1 {
        transaction.execute(
            "INSERT INTO schema_migrations (version) VALUES (?1)",
            [1_u32],
        )?;
    }

    if current_version < 2 {
        transaction.execute_batch(
            "CREATE TABLE application_settings (
                key TEXT PRIMARY KEY NOT NULL,
                value TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );",
        )?;
        transaction.execute(
            "INSERT INTO schema_migrations (version) VALUES (?1)",
            [2_u32],
        )?;
    }

    if current_version < 3 {
        transaction.execute_batch(
            "CREATE TABLE clipboard_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT CHECK (id > 0),
                content_type TEXT NOT NULL CHECK (content_type IN ('text', 'image')),
                text_content TEXT,
                file_path TEXT,
                content_hash TEXT NOT NULL CHECK (
                    length(content_hash) = 64
                    AND content_hash NOT GLOB '*[^0-9a-f]*'
                ),
                source_application TEXT CHECK (
                    source_application IS NULL OR length(source_application) <= 256
                ),
                source_process TEXT CHECK (
                    source_process IS NULL OR length(source_process) <= 512
                ),
                captured_at_ms INTEGER NOT NULL CHECK (
                    captured_at_ms >= 0 AND captured_at_ms <= 9007199254740991
                ),
                byte_size INTEGER NOT NULL CHECK (
                    byte_size >= 0 AND byte_size <= 9007199254740991
                ),
                is_favorite INTEGER NOT NULL DEFAULT 0 CHECK (is_favorite IN (0, 1)),
                CHECK (
                    (
                        content_type = 'text'
                        AND text_content IS NOT NULL
                        AND file_path IS NULL
                        AND length(CAST(text_content AS BLOB)) <= 4194304
                        AND byte_size = length(CAST(text_content AS BLOB))
                    )
                    OR
                    (content_type = 'image' AND text_content IS NULL AND file_path IS NOT NULL)
                ),
                UNIQUE (content_type, content_hash)
            );
            CREATE INDEX clipboard_history_recency_idx
                ON clipboard_history (captured_at_ms DESC, id DESC);
            CREATE INDEX clipboard_history_favorite_recency_idx
                ON clipboard_history (is_favorite, captured_at_ms DESC, id DESC);",
        )?;
        transaction.execute(
            "INSERT INTO schema_migrations (version) VALUES (?1)",
            [3_u32],
        )?;
    }

    if current_version < 4 {
        transaction.execute_batch(
            "ALTER TABLE clipboard_history
                ADD COLUMN content_revision INTEGER NOT NULL DEFAULT 1
                CHECK (content_revision > 0 AND content_revision <= 9007199254740991);
             ALTER TABLE clipboard_history ADD COLUMN source_icon_path TEXT;",
        )?;
        transaction.execute(
            "INSERT INTO schema_migrations (version) VALUES (?1)",
            [4_u32],
        )?;
    }

    if current_version < 5 {
        transaction.execute_batch(
            "ALTER TABLE clipboard_history RENAME TO clipboard_history_v4;
             CREATE TABLE clipboard_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT CHECK (id > 0),
                content_type TEXT NOT NULL CHECK (content_type IN ('text', 'image', 'files')),
                text_content TEXT,
                file_path TEXT,
                content_hash TEXT NOT NULL CHECK (
                    length(content_hash) = 64 AND content_hash NOT GLOB '*[^0-9a-f]*'
                ),
                source_application TEXT CHECK (
                    source_application IS NULL OR length(source_application) <= 256
                ),
                source_process TEXT CHECK (
                    source_process IS NULL OR length(source_process) <= 512
                ),
                captured_at_ms INTEGER NOT NULL CHECK (
                    captured_at_ms >= 0 AND captured_at_ms <= 9007199254740991
                ),
                byte_size INTEGER NOT NULL CHECK (
                    byte_size >= 0 AND byte_size <= 9007199254740991
                ),
                is_favorite INTEGER NOT NULL DEFAULT 0 CHECK (is_favorite IN (0, 1)),
                content_revision INTEGER NOT NULL DEFAULT 1 CHECK (
                    content_revision > 0 AND content_revision <= 9007199254740991
                ),
                source_icon_path TEXT,
                file_paths_json TEXT,
                CHECK (
                    (content_type = 'text' AND text_content IS NOT NULL AND file_path IS NULL
                        AND file_paths_json IS NULL
                        AND length(CAST(text_content AS BLOB)) <= 4194304
                        AND byte_size = length(CAST(text_content AS BLOB)))
                    OR (content_type = 'image' AND text_content IS NULL AND file_path IS NOT NULL
                        AND file_paths_json IS NULL)
                    OR (content_type = 'files' AND text_content IS NULL AND file_path IS NULL
                        AND file_paths_json IS NOT NULL)
                ),
                UNIQUE (content_type, content_hash)
             );
             INSERT INTO clipboard_history (
                id, content_type, text_content, file_path, content_hash,
                source_application, source_process, captured_at_ms, byte_size,
                is_favorite, content_revision, source_icon_path, file_paths_json
             ) SELECT
                id, content_type, text_content, file_path, content_hash,
                source_application, source_process, captured_at_ms, byte_size,
                is_favorite, content_revision, source_icon_path, NULL
             FROM clipboard_history_v4;
             DROP TABLE clipboard_history_v4;
             CREATE INDEX clipboard_history_recency_idx
                ON clipboard_history (captured_at_ms DESC, id DESC);
             CREATE INDEX clipboard_history_favorite_recency_idx
                ON clipboard_history (is_favorite, captured_at_ms DESC, id DESC);",
        )?;
        transaction.execute(
            "INSERT INTO schema_migrations (version) VALUES (?1)",
            [5_u32],
        )?;
    }

    transaction.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn initialization_creates_managed_paths_database_and_migration_metadata() {
        let temp = tempdir().expect("temporary directory should be created");
        let data_root = temp.path().join("app-data");

        let storage =
            StorageService::initialize(&data_root).expect("storage initialization should succeed");

        assert!(storage.data_root().is_dir());
        assert!(storage.files_dir().is_dir());
        assert!(storage.database_path().is_file());
        assert_eq!(storage.migration_version().unwrap(), LATEST_SCHEMA_VERSION);
        assert_eq!(
            storage
                .query_i64("SELECT COUNT(*) FROM schema_migrations", &[])
                .unwrap(),
            i64::from(LATEST_SCHEMA_VERSION)
        );
    }

    #[test]
    fn repeated_initialization_is_idempotent_and_preserves_data() {
        let temp = tempdir().expect("temporary directory should be created");
        let data_root = temp.path().join("app-data");
        let storage = StorageService::initialize(&data_root).unwrap();
        storage
            .transaction(|transaction| {
                transaction.execute_batch(
                    "CREATE TABLE test_records (value INTEGER NOT NULL);
                     INSERT INTO test_records (value) VALUES (7);",
                )?;
                Ok(())
            })
            .unwrap();
        drop(storage);

        let reopened = StorageService::initialize(&data_root).unwrap();

        assert_eq!(reopened.migration_version().unwrap(), LATEST_SCHEMA_VERSION);
        assert_eq!(
            reopened
                .query_i64("SELECT COUNT(*) FROM schema_migrations", &[])
                .unwrap(),
            i64::from(LATEST_SCHEMA_VERSION)
        );
        assert_eq!(
            reopened
                .query_i64("SELECT value FROM test_records", &[])
                .unwrap(),
            7
        );
    }

    #[test]
    fn version_two_database_migrates_to_clipboard_schema_without_losing_settings() {
        let temp = tempdir().unwrap();
        let data_root = temp.path().join("app-data");
        fs::create_dir_all(&data_root).unwrap();
        let database_path = data_root.join(DATABASE_FILE_NAME);
        let connection = Connection::open(&database_path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (
                    version INTEGER PRIMARY KEY NOT NULL,
                    applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                 );
                 INSERT INTO schema_migrations (version) VALUES (1), (2);
                 CREATE TABLE application_settings (
                    key TEXT PRIMARY KEY NOT NULL,
                    value TEXT NOT NULL,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                 );
                 INSERT INTO application_settings (key, value) VALUES ('theme.mode', 'dark');",
            )
            .unwrap();
        drop(connection);

        let storage = StorageService::initialize(&data_root).unwrap();

        assert_eq!(storage.migration_version().unwrap(), 5);
        assert_eq!(
            storage.read_setting("theme.mode").unwrap().as_deref(),
            Some("dark")
        );
        assert_eq!(
            storage
                .query_i64(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'clipboard_history'",
                    &[]
                )
                .unwrap(),
            1
        );
    }

    #[test]
    fn version_three_clipboard_rows_migrate_with_initial_revision_and_no_icon_path() {
        let temp = tempdir().unwrap();
        let data_root = temp.path().join("app-data");
        fs::create_dir_all(&data_root).unwrap();
        let connection = Connection::open(data_root.join(DATABASE_FILE_NAME)).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE schema_migrations (
                    version INTEGER PRIMARY KEY NOT NULL,
                    applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                 );
                 INSERT INTO schema_migrations (version) VALUES (1), (2), (3);
                 CREATE TABLE application_settings (
                    key TEXT PRIMARY KEY NOT NULL,
                    value TEXT NOT NULL,
                    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                 );
                 CREATE TABLE clipboard_history (
                    id INTEGER PRIMARY KEY AUTOINCREMENT CHECK (id > 0),
                    content_type TEXT NOT NULL,
                    text_content TEXT,
                    file_path TEXT,
                    content_hash TEXT NOT NULL,
                    source_application TEXT,
                    source_process TEXT,
                    captured_at_ms INTEGER NOT NULL,
                    byte_size INTEGER NOT NULL,
                    is_favorite INTEGER NOT NULL DEFAULT 0,
                    UNIQUE (content_type, content_hash)
                 );
                 INSERT INTO clipboard_history (
                    content_type, text_content, content_hash, captured_at_ms, byte_size
                 ) VALUES (
                    'text', 'legacy',
                    'c49fea7425fa7f8699897a97c159c6690267d9003bb78c53c61283aab5a767c8',
                    1, 6
                 );",
            )
            .unwrap();
        drop(connection);

        let storage = StorageService::initialize(&data_root).unwrap();

        assert_eq!(storage.migration_version().unwrap(), 5);
        assert_eq!(
            storage
                .query_i64("SELECT content_revision FROM clipboard_history", &[])
                .unwrap(),
            1
        );
        assert_eq!(
            storage
                .query_i64(
                    "SELECT source_icon_path IS NULL FROM clipboard_history",
                    &[]
                )
                .unwrap(),
            1
        );
    }

    #[test]
    fn successful_transaction_commits_changes() {
        let temp = tempdir().unwrap();
        let storage = StorageService::initialize(temp.path()).unwrap();

        storage
            .transaction(|transaction| {
                transaction.execute_batch(
                    "CREATE TABLE committed_records (value INTEGER NOT NULL);
                     INSERT INTO committed_records (value) VALUES (42);",
                )?;
                Ok(())
            })
            .unwrap();

        assert_eq!(
            storage
                .query_i64("SELECT value FROM committed_records", &[])
                .unwrap(),
            42
        );
    }

    #[test]
    fn query_helper_rejects_mutation_statements() {
        let temp = tempdir().unwrap();
        let storage = StorageService::initialize(temp.path()).unwrap();
        storage
            .transaction(|transaction| {
                transaction.execute_batch(
                    "CREATE TABLE query_guard_records (value INTEGER NOT NULL);
                     INSERT INTO query_guard_records (value) VALUES (12);",
                )?;
                Ok(())
            })
            .unwrap();

        let error = storage
            .query_i64("DELETE FROM query_guard_records RETURNING value", &[])
            .unwrap_err();

        assert!(matches!(error, StorageError::QueryMustBeReadOnly));
        assert_eq!(
            storage
                .query_i64("SELECT COUNT(*) FROM query_guard_records", &[])
                .unwrap(),
            1
        );
    }

    #[test]
    fn application_settings_are_parameterized_and_atomically_updated() {
        let temp = tempdir().unwrap();
        let storage = StorageService::initialize(temp.path()).unwrap();

        storage
            .write_settings(&[("theme.mode", "light"), ("quote'key", "first")])
            .unwrap();
        storage
            .write_settings(&[("theme.mode", "dark"), ("quote'key", "second")])
            .unwrap();

        assert_eq!(
            storage.read_setting("theme.mode").unwrap().as_deref(),
            Some("dark")
        );
        assert_eq!(
            storage.read_setting("quote'key").unwrap().as_deref(),
            Some("second")
        );
        assert_eq!(storage.read_setting("missing").unwrap(), None);
    }

    #[test]
    fn failed_transaction_rolls_back_changes() {
        let temp = tempdir().unwrap();
        let storage = StorageService::initialize(temp.path()).unwrap();
        storage
            .transaction(|transaction| {
                transaction
                    .execute("CREATE TABLE rollback_records (value INTEGER NOT NULL)", [])?;
                Ok(())
            })
            .unwrap();

        let error = storage
            .transaction(|transaction| {
                transaction.execute("INSERT INTO rollback_records (value) VALUES (99)", [])?;
                Err::<(), _>(rusqlite::Error::InvalidQuery.into())
            })
            .unwrap_err();

        assert!(matches!(
            error,
            StorageError::Sql(rusqlite::Error::InvalidQuery)
        ));
        assert_eq!(
            storage
                .query_i64("SELECT COUNT(*) FROM rollback_records", &[])
                .unwrap(),
            0
        );
    }

    #[test]
    fn relative_path_resolution_accepts_managed_descendants() {
        let temp = tempdir().unwrap();
        let storage = StorageService::initialize(temp.path()).unwrap();

        let resolved = storage
            .resolve_relative_path(Path::new("files").join("images").join("capture.png"))
            .unwrap();

        assert_eq!(
            resolved,
            storage.data_root().join("files/images/capture.png")
        );
    }

    #[test]
    fn relative_path_resolution_rejects_absolute_parent_and_empty_paths() {
        let temp = tempdir().unwrap();
        let storage = StorageService::initialize(temp.path()).unwrap();
        let absolute = storage.data_root().join("outside.png");

        for path in [
            absolute,
            PathBuf::from("../outside.png"),
            PathBuf::from("files/../../outside.png"),
            PathBuf::from("."),
            PathBuf::new(),
        ] {
            assert!(matches!(
                storage.resolve_relative_path(&path),
                Err(StorageError::InvalidRelativePath(rejected)) if rejected == path
            ));
        }
    }

    #[test]
    fn initialization_fails_deterministically_when_data_root_is_a_file() {
        let temp = tempdir().unwrap();
        let file_path = temp.path().join("not-a-directory");
        fs::write(&file_path, b"occupied").unwrap();

        let error = StorageService::initialize(&file_path).unwrap_err();

        assert!(matches!(
            error,
            StorageError::Io {
                operation: "create application data root",
                path,
                ..
            } if path == file_path
        ));
        assert_eq!(fs::read(&file_path).unwrap(), b"occupied");
    }
}
