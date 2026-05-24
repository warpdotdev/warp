use http::StatusCode;

use super::{register_error, ErrorExt};

impl ErrorExt for websocket::tungstenite::Error {
    fn is_actionable(&self) -> bool {
        match self {
            Self::Http(res) => {
                // Capacity errors from the server aren't actionable client-side.
                if res.status() == StatusCode::TOO_MANY_REQUESTS {
                    return false;
                }

                // Internal server errors (5xx) are server-side issues that we can't act upon from the client.
                if res.status().is_server_error() {
                    return false;
                }

                true
            }
            _ => true,
        }
    }
}
register_error!(websocket::tungstenite::Error);
