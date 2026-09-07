use synthetic_data_sqlite::SqliteStore;

fn database(directory: &tempfile::TempDir) -> (std::path::PathBuf, String) {
    let path = directory.path().join("passive.db");
    let url = format!(
        "sqlite://{}?mode=rwc",
        path.to_string_lossy().replace('\\', "/")
    );
    (path, url)
}

#[tokio::test]
async fn missing_database_is_never_created_even_with_rwc_url() {
    let directory = tempfile::tempdir().unwrap();
    let (path, url) = database(&directory);
    assert!(SqliteStore::connect_read_only(&url).await.is_err());
    assert!(!path.exists());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn passive_connection_sees_live_wal_and_rejects_writes() {
    let directory = tempfile::tempdir().unwrap();
    let (_, url) = database(&directory);
    let writer = SqliteStore::connect(&url).await.unwrap();
    sqlx::query("CREATE TABLE passive_probe (value INTEGER NOT NULL)")
        .execute(writer.pool())
        .await
        .unwrap();
    let reader = SqliteStore::connect_read_only(&url).await.unwrap();
    sqlx::query("INSERT INTO passive_probe VALUES (1)")
        .execute(writer.pool())
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT SUM(value) FROM passive_probe")
            .fetch_one(reader.pool())
            .await
            .unwrap(),
        1
    );
    assert!(
        sqlx::query("INSERT INTO passive_probe VALUES (99)")
            .execute(reader.pool())
            .await
            .is_err()
    );
    sqlx::query("INSERT INTO passive_probe VALUES (2)")
        .execute(writer.pool())
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT SUM(value) FROM passive_probe")
            .fetch_one(reader.pool())
            .await
            .unwrap(),
        3
    );
    reader.pool().close().await;
    writer.pool().close().await;
}

#[tokio::test]
async fn passive_connection_preserves_delete_journal_and_database_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let (path, url) = database(&directory);
    let writer = SqliteStore::connect(&url).await.unwrap();
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(writer.pool())
        .await
        .unwrap();
    writer.pool().close().await;
    let connection = sqlx::SqlitePool::connect(&url).await.unwrap();
    sqlx::query("PRAGMA journal_mode=DELETE")
        .execute(&connection)
        .await
        .unwrap();
    connection.close().await;
    let before = std::fs::read(&path).unwrap();
    let reader = SqliteStore::connect_read_only(&url).await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("PRAGMA journal_mode")
            .fetch_one(reader.pool())
            .await
            .unwrap(),
        "delete"
    );
    reader.pool().close().await;
    assert_eq!(before, std::fs::read(&path).unwrap());
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn incompatible_schema_is_rejected_without_migration_or_repair() {
    for mutation in [
        "DELETE FROM _sqlx_migrations WHERE version=(SELECT MAX(version) FROM _sqlx_migrations)",
        "UPDATE _sqlx_migrations SET checksum=X'00' WHERE version=(SELECT MIN(version) FROM _sqlx_migrations)",
        "UPDATE _sqlx_migrations SET success=0 WHERE version=(SELECT MIN(version) FROM _sqlx_migrations)",
        "UPDATE _sqlx_migrations SET version=999999999 WHERE version=(SELECT MAX(version) FROM _sqlx_migrations)",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let (path, url) = database(&directory);
        let writer = SqliteStore::connect(&url).await.unwrap();
        sqlx::query(mutation).execute(writer.pool()).await.unwrap();
        writer.pool().close().await;
        let before = std::fs::read(&path).unwrap();
        let error = SqliteStore::connect_read_only(&url).await.unwrap_err();
        assert!(error.to_string().contains("database schema does not match"));
        assert_eq!(before, std::fs::read(&path).unwrap());
    }
}
