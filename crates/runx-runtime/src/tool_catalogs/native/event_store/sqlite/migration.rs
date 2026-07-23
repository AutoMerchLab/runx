use std::fs;
use std::path::Path;
use std::time::Duration;

use runx_contracts::{JsonValue, MAX_PORTABLE_INTEGER, hex_lower};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};

use crate::RuntimeError;

use super::super::migration::EventStoreMigrationStatus;
use super::super::{input, model};
use super::{database_error, schema};

const OPERATION: &str = "data.migrate_event_store";
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

pub(in crate::tool_catalogs::native::event_store) struct MigrationReport {
    pub(in crate::tool_catalogs::native::event_store) status: EventStoreMigrationStatus,
    pub(in crate::tool_catalogs::native::event_store) source_schema: String,
    pub(in crate::tool_catalogs::native::event_store) target_schema_version: u64,
    pub(in crate::tool_catalogs::native::event_store) source_digest: String,
    pub(in crate::tool_catalogs::native::event_store) backup_digest: Option<String>,
    pub(in crate::tool_catalogs::native::event_store) result_digest: String,
    pub(in crate::tool_catalogs::native::event_store) event_count: u64,
    pub(in crate::tool_catalogs::native::event_store) stream_count: u64,
}

#[derive(Clone, Copy)]
enum Layout {
    Current,
    Legacy(schema::EventSchemaV0),
}

#[derive(Debug, Eq, PartialEq)]
struct Snapshot {
    digest: String,
    event_count: u64,
    stream_count: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StoredEvent {
    data_source_ref: String,
    resource: String,
    aggregate_id: String,
    version: u64,
    idempotency_key: String,
    event_ref: String,
    event_type: String,
    event_digest: String,
    event_json: String,
    committed_at: String,
}

pub(in crate::tool_catalogs::native::event_store) fn migrate_event_store_database(
    database: &Path,
    backup: &Path,
    data_source_ref: &str,
) -> Result<MigrationReport, RuntimeError> {
    let mut connection = open(database)?;
    acquire_offline_lock(&connection)?;
    let version = schema_version(&connection)?;
    if version == schema::SCHEMA_VERSION {
        schema::validate_current_schema(OPERATION, &connection)?;
        let current = snapshot(&connection, Layout::Current, data_source_ref, true)?;
        return Ok(MigrationReport {
            status: EventStoreMigrationStatus::Current,
            source_schema: "v1".to_owned(),
            target_schema_version: schema::SCHEMA_VERSION as u64,
            source_digest: current.digest.clone(),
            backup_digest: None,
            result_digest: current.digest,
            event_count: current.event_count,
            stream_count: current.stream_count,
        });
    }
    if version != 0 || !schema::event_store_tables_exist(&connection, OPERATION)? {
        return Err(unsupported());
    }
    let legacy = match schema::legacy_schema(OPERATION, &connection) {
        Ok(schema) => schema,
        Err(RuntimeError::SkillFailed { message, .. })
            if message.contains("unsupported legacy schema") =>
        {
            return Err(unsupported());
        }
        Err(error) => return Err(error),
    };
    if backup.exists() {
        return Err(invalid(format!(
            "backup target {} already exists; choose a new --backup path",
            backup.display()
        )));
    }
    if let Some(parent) = backup.parent() {
        fs::create_dir_all(parent).map_err(|source| {
            RuntimeError::io(format!("creating backup directory {}", parent.display()), source)
        })?;
    }

    let source = snapshot(&connection, Layout::Legacy(legacy), data_source_ref, false)?;
    connection
        .backup(rusqlite::MAIN_DB, backup, None)
        .map_err(|error| database_error(OPERATION, "creating consistent SQLite backup", error))?;
    let backup_connection = open(backup)?;
    let backup_legacy = schema::legacy_schema(OPERATION, &backup_connection)?;
    if backup_legacy != legacy {
        return Err(invalid("backup schema fingerprint differs from the locked source"));
    }
    let backup_snapshot = snapshot(
        &backup_connection,
        Layout::Legacy(backup_legacy),
        data_source_ref,
        false,
    )?;
    if backup_snapshot != source {
        return Err(invalid("backup verification did not reproduce the locked source"));
    }
    drop(backup_connection);

    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Exclusive)
        .map_err(|error| database_error(OPERATION, "starting migration transaction", error))?;
    install_current_schema(&transaction, legacy, data_source_ref)?;
    transaction
        .pragma_update(None, "user_version", schema::SCHEMA_VERSION)
        .map_err(|error| database_error(OPERATION, "sealing migrated schema version", error))?;
    schema::validate_current_schema(OPERATION, &transaction)?;
    let result = snapshot(&transaction, Layout::Current, data_source_ref, true)?;
    if result != source {
        return Err(invalid(
            "migrated event counts, streams, or content digest differ from the source",
        ));
    }
    transaction
        .commit()
        .map_err(|error| database_error(OPERATION, "committing verified migration", error))?;

    Ok(MigrationReport {
        status: EventStoreMigrationStatus::Migrated,
        source_schema: legacy.label().to_owned(),
        target_schema_version: schema::SCHEMA_VERSION as u64,
        source_digest: source.digest,
        backup_digest: Some(backup_snapshot.digest),
        result_digest: result.digest,
        event_count: result.event_count,
        stream_count: result.stream_count,
    })
}

fn acquire_offline_lock(connection: &Connection) -> Result<(), RuntimeError> {
    connection
        .pragma_update(None, "locking_mode", "EXCLUSIVE")
        .map_err(|error| database_error(OPERATION, "selecting exclusive locking mode", error))?;
    connection
        .execute_batch("BEGIN EXCLUSIVE; COMMIT;")
        .map_err(|error| database_error(OPERATION, "acquiring exclusive migration lock", error))
}

fn install_current_schema(
    connection: &Connection,
    legacy: schema::EventSchemaV0,
    unscoped_data_source_ref: &str,
) -> Result<(), RuntimeError> {
    connection
        .execute_batch(
            "DROP INDEX IF EXISTS runx_events_stream_version_v1;
             DROP INDEX IF EXISTS runx_events_stream_idempotency_v1;
             DROP INDEX IF EXISTS runx_stream_heads_recent_v1;
             DROP INDEX IF EXISTS runx_stream_heads_type_recent_v1;
             ALTER TABLE runx_events RENAME TO runx_events_migration_v0;
             DROP TABLE IF EXISTS runx_stream_heads;
             DROP TABLE IF EXISTS runx_data_store_migrations;",
        )
        .map_err(|error| database_error(OPERATION, "staging SQLite v0 migration", error))?;
    connection.execute_batch(schema::SCHEMA).map_err(|error| {
        database_error(OPERATION, "creating current SQLite event-store schema", error)
    })?;
    let copy = match legacy {
        schema::EventSchemaV0::Unscoped => {
            "INSERT INTO runx_events (data_source_ref, resource, aggregate_id, version, idempotency_key, event_ref, event_type, event_digest, event_json, committed_at)
             SELECT ?1, resource, aggregate_id, version, idempotency_key, event_ref, event_type, event_digest, event_json, committed_at
             FROM runx_events_migration_v0"
        }
        schema::EventSchemaV0::Scoped => {
            "INSERT INTO runx_events (data_source_ref, resource, aggregate_id, version, idempotency_key, event_ref, event_type, event_digest, event_json, committed_at)
             SELECT CASE WHEN trim(data_source_ref) = '' THEN ?1 ELSE data_source_ref END,
                    resource, aggregate_id, version, idempotency_key, event_ref, event_type, event_digest, event_json, committed_at
             FROM runx_events_migration_v0"
        }
    };
    connection
        .execute(copy, params![unscoped_data_source_ref])
        .map_err(|error| database_error(OPERATION, "copying legacy SQLite events", error))?;
    rebuild_stream_heads(connection)?;
    connection
        .execute_batch("DROP TABLE runx_events_migration_v0;")
        .map_err(|error| database_error(OPERATION, "removing migrated SQLite v0 events", error))
}

fn rebuild_stream_heads(connection: &Connection) -> Result<(), RuntimeError> {
    let mut cursor: Option<(String, String, String)> = None;
    while let Some(key) = next_stream_key(connection, cursor.as_ref())? {
        let (head, projection_digest) = read_stream(connection, &key)?;
        insert_stream_head(connection, &head, &projection_digest)?;
        cursor = Some(key);
    }
    Ok(())
}

fn next_stream_key(
    connection: &Connection,
    after: Option<&(String, String, String)>,
) -> Result<Option<(String, String, String)>, RuntimeError> {
    let row = if let Some((source, resource, aggregate_id)) = after {
        connection
            .query_row(
                "SELECT data_source_ref, resource, aggregate_id FROM runx_events
                 WHERE (data_source_ref, resource, aggregate_id) > (?1, ?2, ?3)
                 ORDER BY data_source_ref, resource, aggregate_id LIMIT 1",
                params![source, resource, aggregate_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
    } else {
        connection
            .query_row(
                "SELECT data_source_ref, resource, aggregate_id FROM runx_events
                 ORDER BY data_source_ref, resource, aggregate_id LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
    };
    row.map_err(|error| database_error(OPERATION, "paging migrated stream identities", error))
}

fn read_stream(
    connection: &Connection,
    key: &(String, String, String),
) -> Result<(StoredEvent, String), RuntimeError> {
    let mut statement = connection
        .prepare(
            "SELECT data_source_ref, resource, aggregate_id, version, idempotency_key, event_ref, event_type, event_digest, event_json, committed_at
             FROM runx_events WHERE data_source_ref = ?1 AND resource = ?2 AND aggregate_id = ?3
             ORDER BY version",
        )
        .map_err(|error| database_error(OPERATION, "preparing migrated stream", error))?;
    let mut rows = statement
        .query(params![key.0, key.1, key.2])
        .map_err(|error| database_error(OPERATION, "reading migrated stream", error))?;
    let mut projection_digest = empty_projection_digest()?;
    let mut previous_version = 0_u64;
    let mut head = None;
    while let Some(row) = rows
        .next()
        .map_err(|error| database_error(OPERATION, "iterating migrated stream", error))?
    {
        let event = decode_event(row)?;
        validate_event(&event)?;
        if event.version != previous_version.saturating_add(1) {
            return Err(invalid("event stream versions are not contiguous"));
        }
        projection_digest = advance_projection_digest(
            event.version,
            &projection_digest,
            &event.event_digest,
        )?;
        previous_version = event.version;
        head = Some(event);
    }
    head.map(|head| (head, projection_digest))
        .ok_or_else(|| invalid("migrated stream identity had no events"))
}

fn insert_stream_head(
    connection: &Connection,
    event: &StoredEvent,
    projection_digest: &str,
) -> Result<(), RuntimeError> {
    connection
        .execute(
            "INSERT INTO runx_stream_heads (data_source_ref, resource, aggregate_id, version, event_ref, event_type, event_digest, idempotency_key, event_json, committed_at, projection_digest)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                event.data_source_ref,
                event.resource,
                event.aggregate_id,
                i64::try_from(event.version)
                    .map_err(|_| invalid("event version exceeds SQLite limits"))?,
                event.event_ref,
                event.event_type,
                event.event_digest,
                event.idempotency_key,
                event.event_json,
                event.committed_at,
                projection_digest,
            ],
        )
        .map(|_| ())
        .map_err(|error| database_error(OPERATION, "rebuilding SQLite stream head", error))
}

fn open(path: &Path) -> Result<Connection, RuntimeError> {
    let connection = Connection::open(path)
        .map_err(|error| database_error(OPERATION, &format!("opening {}", path.display()), error))?;
    connection
        .busy_timeout(BUSY_TIMEOUT)
        .map_err(|error| database_error(OPERATION, "setting SQLite busy timeout", error))?;
    Ok(connection)
}

fn schema_version(connection: &Connection) -> Result<i64, RuntimeError> {
    connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(|error| database_error(OPERATION, "reading SQLite schema version", error))
}

fn snapshot(
    connection: &Connection,
    layout: Layout,
    default_source: &str,
    verify_heads: bool,
) -> Result<Snapshot, RuntimeError> {
    let sql = match layout {
        Layout::Legacy(schema::EventSchemaV0::Unscoped) => {
            "SELECT ?1, resource, aggregate_id, version, idempotency_key, event_ref, event_type, event_digest, event_json, committed_at FROM runx_events ORDER BY resource, aggregate_id, version"
        }
        Layout::Legacy(schema::EventSchemaV0::Scoped) => {
            "SELECT CASE WHEN trim(data_source_ref) = '' THEN ?1 ELSE data_source_ref END, resource, aggregate_id, version, idempotency_key, event_ref, event_type, event_digest, event_json, committed_at FROM runx_events ORDER BY 1, resource, aggregate_id, version"
        }
        Layout::Current => {
            "SELECT data_source_ref, resource, aggregate_id, version, idempotency_key, event_ref, event_type, event_digest, event_json, committed_at FROM runx_events ORDER BY data_source_ref, resource, aggregate_id, version"
        }
    };
    let mut statement = connection
        .prepare(sql)
        .map_err(|error| database_error(OPERATION, "preparing migration verification", error))?;
    let mut rows = match layout {
        Layout::Current => statement.query([]),
        Layout::Legacy(_) => statement.query(params![default_source]),
    }
    .map_err(|error| database_error(OPERATION, "reading migration verification rows", error))?;
    let mut digest = Sha256::new();
    digest.update(b"runx.event-store.content.v1\0");
    let mut event_count = 0_u64;
    let mut stream_count = 0_u64;
    let mut current_key: Option<(String, String, String)> = None;
    let mut previous_version = 0_u64;
    let mut projection_digest = empty_projection_digest()?;
    let mut head: Option<StoredEvent> = None;

    while let Some(row) = rows
        .next()
        .map_err(|error| database_error(OPERATION, "iterating migration verification", error))?
    {
        let event = decode_event(row)?;
        validate_event(&event)?;
        let key = event.stream_key();
        if current_key.as_ref() != Some(&key) {
            if verify_heads {
                verify_head(connection, head.as_ref(), &projection_digest)?;
            }
            current_key = Some(key);
            previous_version = 0;
            projection_digest = empty_projection_digest()?;
            stream_count = stream_count.saturating_add(1);
        }
        if event.version != previous_version.saturating_add(1) {
            return Err(invalid("event stream versions are not contiguous"));
        }
        projection_digest = advance_projection_digest(
            event.version,
            &projection_digest,
            &event.event_digest,
        )?;
        previous_version = event.version;
        event_count = event_count.saturating_add(1);
        hash_event(&mut digest, &event)?;
        head = Some(event);
    }
    drop(rows);
    drop(statement);
    if verify_heads {
        verify_head(connection, head.as_ref(), &projection_digest)?;
        let head_count = connection
            .query_row("SELECT COUNT(*) FROM runx_stream_heads", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|error| database_error(OPERATION, "counting verified stream heads", error))?;
        if u64::try_from(head_count).ok() != Some(stream_count) {
            return Err(invalid("stream-head count differs from the event streams"));
        }
    }
    Ok(Snapshot {
        digest: format!("sha256:{}", hex_lower(&digest.finalize())),
        event_count,
        stream_count,
    })
}

fn decode_event(row: &rusqlite::Row<'_>) -> Result<StoredEvent, RuntimeError> {
    let version = row
        .get::<_, i64>(3)
        .map_err(|error| database_error(OPERATION, "decoding event version", error))?;
    Ok(StoredEvent {
        data_source_ref: row.get(0).map_err(decode_error)?,
        resource: row.get(1).map_err(decode_error)?,
        aggregate_id: row.get(2).map_err(decode_error)?,
        version: u64::try_from(version).map_err(|_| invalid("event version is negative"))?,
        idempotency_key: row.get(4).map_err(decode_error)?,
        event_ref: row.get(5).map_err(decode_error)?,
        event_type: row.get(6).map_err(decode_error)?,
        event_digest: row.get(7).map_err(decode_error)?,
        event_json: row.get(8).map_err(decode_error)?,
        committed_at: row.get(9).map_err(decode_error)?,
    })
}

fn decode_error(error: rusqlite::Error) -> RuntimeError {
    database_error(OPERATION, "decoding event-store row", error)
}

fn validate_event(event: &StoredEvent) -> Result<(), RuntimeError> {
    input::ReadProjectionInput {
        data_source_ref: event.data_source_ref.clone(),
        resource: event.resource.clone(),
        aggregate_id: event.aggregate_id.clone(),
    }
    .validate()?;
    input::validate_event_type(OPERATION, &event.event_type)?;
    if event.version == 0 || event.version > MAX_PORTABLE_INTEGER {
        return Err(invalid("event version is outside the portable range"));
    }
    if event.event_ref != format!("{}:{}:{}", event.resource, event.aggregate_id, event.version)
        || event.idempotency_key.trim().is_empty()
        || event.idempotency_key.len() > 256
        || event.idempotency_key.chars().any(char::is_control)
    {
        return Err(invalid("event identity is invalid"));
    }
    let body: JsonValue = serde_json::from_str(&event.event_json)
        .map_err(|source| RuntimeError::json("decoding migrated event JSON", source))?;
    let object = body
        .as_object()
        .ok_or_else(|| invalid("event JSON must be an object"))?;
    if model::event_type(object) != event.event_type || model::digest(&body)? != event.event_digest {
        return Err(invalid("event type or digest does not match its canonical JSON"));
    }
    if model::normalize_time(&event.committed_at)? != event.committed_at {
        return Err(invalid("event commit time is not canonical RFC 3339"));
    }
    Ok(())
}

fn hash_event(digest: &mut Sha256, event: &StoredEvent) -> Result<(), RuntimeError> {
    for bytes in [
        event.data_source_ref.as_bytes(),
        event.resource.as_bytes(),
        event.aggregate_id.as_bytes(),
        &event.version.to_be_bytes(),
        event.idempotency_key.as_bytes(),
        event.event_ref.as_bytes(),
        event.event_type.as_bytes(),
        event.event_digest.as_bytes(),
        event.committed_at.as_bytes(),
    ] {
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes);
    }
    let body: JsonValue = serde_json::from_str(&event.event_json)
        .map_err(|source| RuntimeError::json("canonicalizing migrated event JSON", source))?;
    let bytes = serde_json::to_vec(&body)
        .map_err(|source| RuntimeError::json("serializing migrated event JSON", source))?;
    digest.update((bytes.len() as u64).to_be_bytes());
    digest.update(bytes);
    Ok(())
}

fn verify_head(
    connection: &Connection,
    event: Option<&StoredEvent>,
    projection_digest: &str,
) -> Result<(), RuntimeError> {
    let Some(event) = event else {
        return Ok(());
    };
    let actual = connection
        .query_row(
            "SELECT data_source_ref, resource, aggregate_id, version, idempotency_key, event_ref, event_type, event_digest, event_json, committed_at, projection_digest FROM runx_stream_heads WHERE data_source_ref = ?1 AND resource = ?2 AND aggregate_id = ?3",
            params![event.data_source_ref, event.resource, event.aggregate_id],
            |row| {
                let stored = decode_head_event(row)?;
                let digest = row.get::<_, String>(10)?;
                Ok((stored, digest))
            },
        )
        .optional()
        .map_err(|error| database_error(OPERATION, "verifying migrated stream head", error))?;
    if actual.as_ref() != Some(&(event.clone(), projection_digest.to_owned())) {
        return Err(invalid("stream head or projection digest failed readback"));
    }
    Ok(())
}

fn decode_head_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredEvent> {
    let version = row.get::<_, i64>(3)?;
    let version = u64::try_from(version).map_err(|source| {
        rusqlite::Error::FromSqlConversionFailure(
            3,
            rusqlite::types::Type::Integer,
            Box::new(source),
        )
    })?;
    Ok(StoredEvent {
        data_source_ref: row.get(0)?,
        resource: row.get(1)?,
        aggregate_id: row.get(2)?,
        version,
        idempotency_key: row.get(4)?,
        event_ref: row.get(5)?,
        event_type: row.get(6)?,
        event_digest: row.get(7)?,
        event_json: row.get(8)?,
        committed_at: row.get(9)?,
    })
}

impl StoredEvent {
    fn stream_key(&self) -> (String, String, String) {
        (
            self.data_source_ref.clone(),
            self.resource.clone(),
            self.aggregate_id.clone(),
        )
    }
}

fn empty_projection_digest() -> Result<String, RuntimeError> {
    model::digest(&JsonValue::Object(runx_contracts::JsonObject::from([
        (
            "version".to_owned(),
            JsonValue::Number(runx_contracts::JsonNumber::U64(0)),
        ),
        ("event_digest".to_owned(), JsonValue::Null),
    ])))
}

fn advance_projection_digest(
    version: u64,
    previous_projection_digest: &str,
    event_digest: &str,
) -> Result<String, RuntimeError> {
    model::digest(&JsonValue::Object(runx_contracts::JsonObject::from([
        (
            "version".to_owned(),
            JsonValue::Number(runx_contracts::JsonNumber::U64(version)),
        ),
        (
            "previous_projection_digest".to_owned(),
            JsonValue::String(previous_projection_digest.to_owned()),
        ),
        (
            "event_digest".to_owned(),
            JsonValue::String(event_digest.to_owned()),
        ),
    ])))
}

fn invalid(message: impl Into<String>) -> RuntimeError {
    RuntimeError::SkillFailed {
        skill_name: OPERATION.to_owned(),
        message: message.into(),
    }
}

fn unsupported() -> RuntimeError {
    invalid(
        "event store is neither the current schema nor a recognized complete legacy schema; the database was not modified",
    )
}
