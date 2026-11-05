//! X11: per-CRTC gamma through RandR. Each CRTC's original ramps are saved the first time it
//! is seen and put back on restore.

use std::collections::HashMap;

use anyhow::{Context, Result};
use x11rb::connection::Connection;
use x11rb::protocol::randr::{ConnectionExt as _, Crtc};
use x11rb::protocol::xproto::Window;
use x11rb::rust_connection::RustConnection;

use super::Backend;
use crate::color::{ramps, Setting};

type Ramps = (Vec<u16>, Vec<u16>, Vec<u16>);

pub struct X11Backend {
    conn: RustConnection,
    root: Window,
    saved: HashMap<Crtc, Ramps>,
}

impl X11Backend {
    pub fn connect() -> Result<Self> {
        let (conn, screen) = x11rb::connect(None).context("failed to connect to X11")?;
        let root = conn.setup().roots[screen].root;
        let version = conn
            .randr_query_version(1, 3)?
            .reply()
            .context("RandR is not available")?;
        if (version.major_version, version.minor_version) < (1, 3) {
            anyhow::bail!(
                "RandR 1.3 or newer is required (server has {}.{})",
                version.major_version,
                version.minor_version
            );
        }
        Ok(Self {
            conn,
            root,
            saved: HashMap::new(),
        })
    }

    /// CRTCs with a gamma table, and its size.
    fn crtcs(&self) -> Result<Vec<(Crtc, usize)>> {
        let resources = self
            .conn
            .randr_get_screen_resources_current(self.root)?
            .reply()?;
        let mut crtcs = Vec::new();
        for crtc in resources.crtcs {
            let size = self.conn.randr_get_crtc_gamma_size(crtc)?.reply()?.size;
            if size > 0 {
                crtcs.push((crtc, usize::from(size)));
            }
        }
        Ok(crtcs)
    }
}

impl Backend for X11Backend {
    fn name(&self) -> &'static str {
        "x11"
    }

    fn apply(&mut self, setting: Setting, gamma: f64) -> Result<()> {
        for (crtc, size) in self.crtcs()? {
            if !self.saved.contains_key(&crtc) {
                let original = self.conn.randr_get_crtc_gamma(crtc)?.reply()?;
                self.saved
                    .insert(crtc, (original.red, original.green, original.blue));
            }
            let [r, g, b] = ramps(size, setting, gamma);
            self.conn.randr_set_crtc_gamma(crtc, &r, &g, &b)?;
        }
        self.conn.flush()?;
        Ok(())
    }

    fn poll(&mut self) -> Result<bool> {
        // Mode changes and other programs can reset gamma, so re-apply on every tick.
        Ok(true)
    }

    fn restore(&mut self) -> Result<()> {
        for (crtc, (r, g, b)) in &self.saved {
            // The CRTC may be gone; that's fine.
            let _ = self.conn.randr_set_crtc_gamma(*crtc, r, g, b);
        }
        self.conn.flush()?;
        Ok(())
    }
}
