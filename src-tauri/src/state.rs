use singerpro_core::db::Pool;
use std::path::PathBuf;

pub struct AppState {
    pub pool: Pool,
    pub data_dir: PathBuf,
}
