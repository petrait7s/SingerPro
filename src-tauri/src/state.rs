use singerpro_core::auth::SessionStore;
use singerpro_core::authz::Principal;
use singerpro_core::recovery::{DataPaths, RestoreOutcome};
use singerpro_core::{auth, CoreError, CoreResult};
use sqlx::SqlitePool;
use std::sync::Mutex;

/// The session token lives ONLY here, in Rust. The webview never sees or sends it, so a
/// compromised page cannot steal or forge a session.
pub struct AppState {
    pub pool: SqlitePool,
    pub paths: DataPaths,
    pub store: SessionStore,
    token: Mutex<Option<String>>,
    pub startup_notice: Mutex<Option<RestoreOutcome>>,
}

impl AppState {
    pub fn new(pool: SqlitePool, paths: DataPaths, notice: Option<RestoreOutcome>) -> Self {
        Self {
            pool,
            paths,
            store: SessionStore::standard(),
            token: Mutex::new(None),
            startup_notice: Mutex::new(notice),
        }
    }

    pub fn set_token(&self, t: Option<String>) {
        let mut g = self.token.lock().expect("token lock");
        if let Some(old) = g.take() {
            self.store.revoke(&old);
        }
        *g = t;
    }

    pub fn token(&self) -> Option<String> {
        self.token.lock().expect("token lock").clone()
    }

    /// Authenticates the current session; permissions are reloaded from the DB on every call.
    pub async fn principal(&self) -> CoreResult<Principal> {
        let t = self.token().ok_or(CoreError::Unauthenticated)?;
        match auth::authenticate(&self.pool, &self.store, &t).await {
            Ok(p) => Ok(p),
            Err(e) => {
                *self.token.lock().expect("token lock") = None;
                Err(e)
            }
        }
    }
}
