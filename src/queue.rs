use std::sync::Mutex;

/// Tracks which target is currently being built.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct BuildStatus {
    pub current: Option<String>,
}

pub struct BuildLock {
    pub inner: tokio::sync::Mutex<()>,
    pub status: Mutex<BuildStatus>,
}

impl BuildLock {
    pub fn new() -> Self {
        Self {
            inner: tokio::sync::Mutex::new(()),
            status: Mutex::new(BuildStatus::default()),
        }
    }

    pub fn set_current(&self, name: Option<String>) {
        let mut st = self.status.lock().unwrap();
        st.current = name;
    }

    pub fn status(&self) -> BuildStatus {
        self.status.lock().unwrap().clone()
    }
}
