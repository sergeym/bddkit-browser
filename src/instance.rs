//! One browser session for one feature file: opened on the file's first
//! browser step, reset between its scenarios, closed when the file ends.

use std::sync::Arc;

use crate::config::{InstanceConfig, Mode};
use crate::webdriver::{Driver, Session, SessionInfo};

pub struct Instance {
    pub config: InstanceConfig,
    pub session: Session,
    pub info: SessionInfo,
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
        let Mode::Remote { url } = &config.mode;
        let driver = Arc::new(Driver::new(url.clone(), debug));
        let (session, info) = driver
            .new_session(&config.session_capabilities(None))
            .map_err(|e| {
                format!(
                    "cannot open a {} session at {url}: {e}",
                    config.browser.browser_name()
                )
            })?;
        let instance = Self {
            config,
            session,
            info,
        };
        let (w, h) = instance.config.window;
        if let Err(e) = instance.session.set_window_rect(w, h) {
            let _ = instance.session.delete();
            return Err(format!("cannot size the window to {w}x{h}: {e}"));
        }
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
        Ok(())
    }

    pub fn close(&self) -> Result<(), String> {
        self.session
            .delete()
            .map_err(|e| format!("closing the session: {e}"))
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
