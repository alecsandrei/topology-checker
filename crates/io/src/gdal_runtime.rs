//! Keep bundled PROJ's database with the executable, independent of build paths.
#[cfg(feature = "gdal")]
use std::sync::{Mutex, OnceLock};

#[cfg(feature = "gdal")]
static DATA: Mutex<Option<tempfile::TempDir>> = Mutex::new(None);
#[cfg(feature = "gdal")]
static INITIALIZED: OnceLock<Result<(), String>> = OnceLock::new();

#[cfg(feature = "gdal")]
extern "C" fn cleanup() {
    // Exit callbacks cannot unwind. Removing an absent directory is harmless.
    if let Ok(mut data) = DATA.lock() {
        // Unix destroys C++ thread-local PROJ caches before atexit callbacks.
        // Calling OSRCleanup there would access an already destroyed cache.
        // Windows uses GDAL's TLS storage instead and needs database handles
        // closed before the temporary directory can be deleted.
        #[cfg(windows)]
        // SAFETY: process exit has begun and Windows' GDAL TLS is still live.
        unsafe {
            gdal_sys::OSRCleanup();
        }
        data.take();
    }
}

pub(crate) fn initialize() -> anyhow::Result<()> {
    #[cfg(feature = "gdal")]
    {
        let initialized = INITIALIZED.get_or_init(|| {
            let init = || -> anyhow::Result<()> {
                let directory = tempfile::Builder::new()
                    .prefix("topology-proj-")
                    .tempdir()?;
                let database = include_bytes!(env!("TOPOLOGY_PROJ_DATABASE"));
                std::fs::write(directory.path().join("proj.db"), database)?;
                let text = directory.path().to_str();
                let text = text.ok_or_else(|| anyhow::anyhow!("invalid PROJ data path"))?;
                let path = std::ffi::CString::new(text)?;
                let paths = [path.as_ptr(), std::ptr::null()];
                // SAFETY: OSR copies the null-terminated paths. Initialization is
                // serialized, and the directory survives until process exit.
                unsafe {
                    gdal_sys::OSRSetPROJSearchPaths(paths.as_ptr());
                }
                let mut data = DATA
                    .lock()
                    .map_err(|_| anyhow::anyhow!("PROJ initialization lock poisoned"))?;
                *data = Some(directory);
                // SAFETY: cleanup has the required C signature and never unwinds.
                let registered = unsafe { libc::atexit(cleanup) };
                anyhow::ensure!(registered == 0, "registering PROJ data cleanup");
                Ok(())
            };
            init().map_err(|e| e.to_string())
        });
        initialized.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    Ok(())
}
