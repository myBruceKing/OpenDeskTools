use std::sync::Arc;

use rusqlite::params;
use thiserror::Error;

use super::storage::{StorageError, StorageService};

const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageAction {
    ClipboardPanel,
    ClipboardQrConversion,
    ScreenshotCapture,
    PinImage,
    ToolMenu,
}

impl UsageAction {
    fn estimated_saved_seconds(self) -> i64 {
        match self {
            Self::ClipboardPanel => 5,
            Self::ClipboardQrConversion => 20,
            Self::ScreenshotCapture => 10,
            Self::PinImage => 5,
            Self::ToolMenu => 5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsageStatisticsSnapshot {
    pub today_triggers: u64,
    pub week_triggers: u64,
    pub month_triggers: u64,
    pub saved_seconds_this_month: u64,
}

#[derive(Debug, Error)]
pub enum UsageStatisticsError {
    #[error("failed to access usage statistics storage: {0}")]
    Storage(#[from] StorageError),
    #[error("usage statistics contains an invalid {field} aggregate: {value}")]
    InvalidAggregate { field: &'static str, value: i64 },
}

#[derive(Debug)]
pub struct UsageStatisticsService {
    storage: Arc<StorageService>,
}

impl UsageStatisticsService {
    pub fn new(storage: Arc<StorageService>) -> Self {
        Self { storage }
    }

    pub fn record_success(&self, action: UsageAction) -> Result<(), UsageStatisticsError> {
        let saved_seconds = action.estimated_saved_seconds();
        self.storage.transaction(|transaction| {
            transaction.execute(
                "INSERT INTO usage_statistics_daily (day, trigger_count, saved_seconds)
                 VALUES (date('now', 'localtime'), 1, ?1)
                 ON CONFLICT(day) DO UPDATE SET
                    trigger_count = MIN(trigger_count + 1, ?2),
                    saved_seconds = MIN(saved_seconds + excluded.saved_seconds, ?2)",
                params![saved_seconds, MAX_SAFE_INTEGER],
            )?;
            Ok(())
        })?;
        Ok(())
    }

    pub fn snapshot(&self) -> Result<UsageStatisticsSnapshot, UsageStatisticsError> {
        let today = self.storage.read(|connection| {
            Ok(
                connection.query_row("SELECT date('now', 'localtime')", [], |row| {
                    row.get::<_, String>(0)
                })?,
            )
        })?;
        self.snapshot_for_day(&today)
    }

    fn snapshot_for_day(
        &self,
        today: &str,
    ) -> Result<UsageStatisticsSnapshot, UsageStatisticsError> {
        let (today_triggers, week_triggers, month_triggers, saved_seconds_this_month) =
            self.storage.read(|connection| {
                Ok(connection.query_row(
                    "WITH bounds AS (
                        SELECT
                            date(?1) AS today,
                            date(
                                ?1,
                                printf(
                                    '-%d days',
                                    (CAST(strftime('%w', ?1) AS INTEGER) + 6) % 7
                                )
                            ) AS week_start,
                            strftime('%Y-%m', ?1) AS month_key
                    )
                    SELECT
                        COALESCE(SUM(
                            CASE WHEN day = bounds.today THEN trigger_count ELSE 0 END
                        ), 0),
                        COALESCE(SUM(
                            CASE
                                WHEN day BETWEEN bounds.week_start AND bounds.today
                                THEN trigger_count
                                ELSE 0
                            END
                        ), 0),
                        COALESCE(SUM(
                            CASE
                                WHEN strftime('%Y-%m', day) = bounds.month_key
                                     AND day <= bounds.today
                                THEN trigger_count
                                ELSE 0
                            END
                        ), 0),
                        COALESCE(SUM(
                            CASE
                                WHEN strftime('%Y-%m', day) = bounds.month_key
                                     AND day <= bounds.today
                                THEN saved_seconds
                                ELSE 0
                            END
                        ), 0)
                    FROM usage_statistics_daily
                    CROSS JOIN bounds",
                    [today],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    },
                )?)
            })?;

        Ok(UsageStatisticsSnapshot {
            today_triggers: to_u64("today trigger", today_triggers)?,
            week_triggers: to_u64("week trigger", week_triggers)?,
            month_triggers: to_u64("month trigger", month_triggers)?,
            saved_seconds_this_month: to_u64("saved seconds this month", saved_seconds_this_month)?,
        })
    }
}

fn to_u64(field: &'static str, value: i64) -> Result<u64, UsageStatisticsError> {
    u64::try_from(value).map_err(|_| UsageStatisticsError::InvalidAggregate { field, value })
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;

    fn service() -> (
        tempfile::TempDir,
        Arc<StorageService>,
        UsageStatisticsService,
    ) {
        let temp = tempdir().unwrap();
        let storage = Arc::new(StorageService::initialize(temp.path().join("data")).unwrap());
        let service = UsageStatisticsService::new(Arc::clone(&storage));
        (temp, storage, service)
    }

    #[test]
    fn records_successful_actions_in_the_current_local_day() {
        let (_temp, _storage, service) = service();

        service.record_success(UsageAction::ClipboardPanel).unwrap();
        service.record_success(UsageAction::ToolMenu).unwrap();
        service
            .record_success(UsageAction::ClipboardQrConversion)
            .unwrap();

        let snapshot = service.snapshot().unwrap();
        assert_eq!(snapshot.today_triggers, 3);
        assert_eq!(snapshot.week_triggers, 3);
        assert_eq!(snapshot.month_triggers, 3);
        assert_eq!(snapshot.saved_seconds_this_month, 30);
    }

    #[test]
    fn snapshot_uses_monday_week_and_excludes_future_rows() {
        let (_temp, storage, service) = service();
        storage
            .transaction(|transaction| {
                for (day, triggers, seconds) in [
                    ("2026-07-19", 2, 20),
                    ("2026-07-20", 3, 30),
                    ("2026-07-22", 5, 50),
                    ("2026-07-23", 7, 70),
                    ("2026-07-24", 11, 110),
                    ("2026-06-30", 13, 130),
                ] {
                    transaction.execute(
                        "INSERT INTO usage_statistics_daily (
                            day, trigger_count, saved_seconds
                         ) VALUES (?1, ?2, ?3)",
                        params![day, triggers, seconds],
                    )?;
                }
                Ok(())
            })
            .unwrap();

        let snapshot = service.snapshot_for_day("2026-07-23").unwrap();
        assert_eq!(snapshot.today_triggers, 7);
        assert_eq!(snapshot.week_triggers, 15);
        assert_eq!(snapshot.month_triggers, 17);
        assert_eq!(snapshot.saved_seconds_this_month, 170);
    }
}
