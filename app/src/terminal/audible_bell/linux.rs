//! Module containing an implementation of an audible bell for x11 backends.

use anyhow::bail;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::rust_connection::RustConnection;

/// An X11 backed implementation of an audible bell.
pub(super) struct AudibleBell {
    connection: Option<RustConnection>,
}

impl AudibleBell {
    pub fn new() -> Self {
        let connection = RustConnection::connect(None)
            .ok()
            .map(|(connection, _)| connection);
        Self { connection }
    }

    pub fn ring(&self) -> anyhow::Result<()> {
        let Some(connection) = &self.connection else {
            bail!("Unable to establish connection to x11 server")
        };
        // Play the bell at 0%. By using 0%, we indicate to the x server that the bell should be played at the user's
        // current volume. See https://www.x.org/releases/X11R7.7/doc/xproto/x11protocol.html#requests:Bell for more
        // details.
        let cookie = connection.bell(0)?;
        cookie.check()?;

        Ok(())
    }
}
