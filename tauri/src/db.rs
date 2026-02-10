use std::sync::Arc;
use surrealdb::Surreal;
use tokio::sync::Mutex;

pub struct DbState(pub Arc<Mutex<Surreal<surrealdb::engine::local::Db>>>);

/// Run database migrations
///
/// Note: With the adapter layer pattern, database migrations are no longer needed.
/// The adapter handles all backward compatibility automatically.
pub async fn run_migrations(_db: &Surreal<surrealdb::engine::local::Db>) -> Result<(), String> {
    // No migrations needed - adapter layer handles all compatibility
    Ok(())
}
