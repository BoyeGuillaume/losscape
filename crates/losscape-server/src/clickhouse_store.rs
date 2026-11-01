//! ClickHouse connection and schema migrations.

use clickhouse::Client;

use crate::config::ClickHouseConfig;

/// (version, name, sql), applied in order and recorded in `schema_migrations`.
const MIGRATIONS: &[(u32, &str, &str)] = &[
    (
        1,
        "metrics",
        include_str!("../migrations/clickhouse/0001_metrics.sql"),
    ),
    (
        2,
        "console",
        include_str!("../migrations/clickhouse/0002_console.sql"),
    ),
];

pub async fn connect_and_migrate(
    config: &ClickHouseConfig,
) -> Result<Client, Box<dyn std::error::Error + Send + Sync>> {
    if !config
        .database
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(format!("invalid ClickHouse database name {:?}", config.database).into());
    }
    let server = Client::default()
        .with_url(&config.url)
        .with_user(&config.user)
        .with_password(&config.password);
    server
        .query(&format!(
            "CREATE DATABASE IF NOT EXISTS `{}`",
            config.database
        ))
        .execute()
        .await?;

    let client = server.with_database(&config.database);
    client
        .query(
            "CREATE TABLE IF NOT EXISTS schema_migrations
             (version UInt32, name String, applied_at DateTime DEFAULT now())
             ENGINE = MergeTree ORDER BY version",
        )
        .execute()
        .await?;
    let applied: Vec<u32> = client
        .query("SELECT version FROM schema_migrations")
        .fetch_all()
        .await?;

    for &(version, name, sql) in MIGRATIONS {
        if applied.contains(&version) {
            continue;
        }
        client.query(sql).execute().await?;
        client
            .query("INSERT INTO schema_migrations (version, name) VALUES (?, ?)")
            .bind(version)
            .bind(name)
            .execute()
            .await?;
        tracing::info!(version, name, "applied ClickHouse migration");
    }
    Ok(client)
}
