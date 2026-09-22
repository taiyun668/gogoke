use tauri::AppHandle;
use tokio::net::TcpStream;

use crate::release_policy;

use super::transport::{
    spawn_transport_io, RemoteTransport, RemoteTransportConfig, TransportFuture,
};

pub(crate) struct TcpTransport;

impl RemoteTransport for TcpTransport {
    fn connect(&self, app: AppHandle, config: RemoteTransportConfig) -> TransportFuture {
        Box::pin(async move {
            let RemoteTransportConfig::Tcp { host, auth_token } = config;

            release_policy::validate_legacy_loopback(&host, auth_token.as_deref(), false)?;

            let stream = TcpStream::connect(host.clone())
                .await
                .map_err(|err| format!("Failed to connect to remote backend at {host}: {err}"))?;
            let (reader, writer) = stream.into_split();
            Ok(spawn_transport_io(app, reader, writer))
        })
    }
}
