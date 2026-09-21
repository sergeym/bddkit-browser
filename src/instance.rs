//! One browser session for one feature file: opened on the file's first
//! browser step, reset between its scenarios, closed when the file ends.

use std::path::PathBuf;
use std::sync::Arc;
#[cfg(unix)]
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use url::Url;

use crate::bidi::Bidi;
use crate::config::{InstanceConfig, Mode};
#[cfg(unix)]
use crate::managed::{self, ManagedDriver};
use crate::webdriver::{Driver, Session, SessionInfo};

/// What `open` holds onto for the driver process it started, managed mode
/// only — `ManagedDriver` on Unix, nothing at all elsewhere, so the match in
/// `open` still has a concrete type for its tuple on every platform even
/// though only one arm ever produces `Some`.
#[cfg(unix)]
type DriverHandle = ManagedDriver;
#[cfg(not(unix))]
type DriverHandle = ();

pub struct Instance {
    pub config: InstanceConfig,
    pub session: Session,
    pub info: SessionInfo,
    pub bidi: Option<Bidi>,
    /// The driver process this instance started, managed mode only.
    #[cfg(unix)]
    driver: Mutex<Option<ManagedDriver>>,
}

/// Storage is cleared while the page is still on the application's origin:
/// on `about:blank` the access throws, and a scenario that never navigated
/// is on `about:blank` — hence the try/catch, so a reset is never failed by
/// a page that had nothing to clear.
const CLEAR_STORAGE: &str =
    "try { window.localStorage.clear(); window.sessionStorage.clear(); } catch (e) {}";

impl Instance {
    /// `debug` seeds the driver's initial trace state only; every dispatch
    /// afterward follows the request through `Driver::set_debug`.
    pub fn open(config: InstanceConfig, debug: bool) -> Result<Self, String> {
        let (url, binary, _driver): (Url, Option<PathBuf>, Option<DriverHandle>) = match &config
            .mode
        {
            Mode::Remote { url } => (url.clone(), None, None),
            #[cfg(unix)]
            Mode::Managed(m) => {
                let resolved = managed::resolve(config.browser, m)?;
                let process =
                    ManagedDriver::start(&resolved.driver_path, debug, Duration::from_secs(10))?;
                (process.url().clone(), resolved.browser_path, Some(process))
            }
            #[cfg(not(unix))]
            Mode::Managed(_) => {
                return Err(
                    "managed mode is not supported on this platform; set \"url\" to a WebDriver endpoint"
                        .to_string(),
                );
            }
        };
        let driver_client = Arc::new(Driver::new(url.clone(), debug));
        let binary = binary.map(|p| p.display().to_string());
        let (session, info) = driver_client
            .new_session(&config.session_capabilities(binary.as_deref()))
            .map_err(|e| {
                format!(
                    "cannot open a {} session at {url}: {e}",
                    config.browser.browser_name()
                )
            })?;
        let mut instance = Self {
            config,
            session,
            info,
            bidi: None,
            #[cfg(unix)]
            driver: Mutex::new(_driver),
        };
        let (w, h) = instance.config.window;
        if let Err(e) = instance.session.set_window_rect(w, h) {
            let _ = instance.session.delete();
            return Err(format!("cannot size the window to {w}x{h}: {e}"));
        }
        let bidi = match &instance.info.websocket_url {
            Some(ws) => Some(Bidi::connect(ws, debug).map_err(|e| {
                let _ = instance.session.delete();
                format!("the session advertised BiDi at {ws} but it cannot be used: {e}")
            })?),
            None => None,
        };
        instance.bidi = bidi;
        if debug {
            eprintln!(
                "[browser] session {} on {} {}",
                instance.session.id, instance.info.browser_name, instance.info.browser_version
            );
        }
        Ok(instance)
    }

    /// `HttpState::reset` for a browser. Order matters — see `CLEAR_STORAGE`.
    pub fn reset(&self) -> Result<(), String> {
        self.session
            .execute(CLEAR_STORAGE, vec![])
            .map_err(|e| format!("clearing web storage: {e}"))?;
        self.session
            .delete_all_cookies()
            .map_err(|e| format!("deleting cookies: {e}"))?;
        self.session
            .navigate("about:blank")
            .map_err(|e| format!("leaving the page: {e}"))?;
        if let Some(bidi) = &self.bidi
            && let Ok(mut b) = bidi.buffers().lock()
        {
            b.clear();
        }
        Ok(())
    }

    pub fn close(&self) -> Result<(), String> {
        if let Some(bidi) = &self.bidi {
            bidi.close();
        }
        let result = self
            .session
            .delete()
            .map_err(|e| format!("closing the session: {e}"));
        #[cfg(unix)]
        {
            let mut guard = self.driver.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(mut d) = guard.take() {
                d.stop();
            }
        }
        result
    }

    /// `doctor --live`: open, read the browser's name and version, close.
    pub fn probe(config: &InstanceConfig) -> Result<(), String> {
        let instance = Self::open(config.clone(), false)?;
        let result = if instance.info.browser_name.is_empty() {
            Err("the session opened but reported no browserName".to_string())
        } else {
            Ok(())
        };
        instance.close()?;
        result
    }
}
