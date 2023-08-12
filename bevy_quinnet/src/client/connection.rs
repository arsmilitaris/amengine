use std::{
    collections::HashMap,
    error::Error,
    net::{AddrParseError, IpAddr, SocketAddr},
    sync::Arc,
};

use bevy::prelude::{error, info, Event};
use bytes::Bytes;
use quinn::{ClientConfig, Endpoint};
use quinn_proto::ConnectionStats;

use serde::Deserialize;
use tokio::sync::{
    broadcast,
    mpsc::{
        self,
        error::{TryRecvError, TrySendError},
    },
};

use crate::shared::{
    channel::{
        channels_task, get_channel_id_from_type, reliable_receiver_task, unreliable_receiver_task,
        Channel, ChannelAsyncMessage, ChannelId, ChannelSyncMessage, ChannelType, MultiChannelId,
    },
    InternalConnectionRef, QuinnetError, DEFAULT_KILL_MESSAGE_QUEUE_SIZE,
    DEFAULT_MESSAGE_QUEUE_SIZE,
};

use super::{
    certificate::{
        load_known_hosts_store_from_config, CertificateVerificationMode, SkipServerVerification,
        TofuServerVerification,
    },
    ClientAsyncMessage,
};

pub type ConnectionId = u64;

/// Connection event raised when the client just connected to the server. Raised in the CoreStage::PreUpdate stage.
#[derive(Event)]
pub struct ConnectionEvent {
    pub id: ConnectionId,
}
/// ConnectionLost event raised when the client is considered disconnected from the server. Raised in the CoreStage::PreUpdate stage.
#[derive(Event)]
pub struct ConnectionLostEvent {
    pub id: ConnectionId,
}

/// Configuration of a client connection, used when connecting to a server
#[derive(Debug, Deserialize, Clone)]
pub struct ConnectionConfiguration {
    server_addr: SocketAddr,
    server_hostname: String,
    local_bind_addr: SocketAddr,
}

impl ConnectionConfiguration {
    /// Creates a new ConnectionConfiguration
    ///
    /// # Arguments
    ///
    /// * `server_addr_str` - IP address and port of the server
    /// * `local_bind_addr_str` - Local address and port to bind to separated by `:`. The address should usually be a wildcard like `0.0.0.0` (for an IPv4) or `[::]` (for an IPv6), which allow communication with any reachable IPv4 or IPv6 address. See [`std::net::SocketAddrV4`] and [`std::net::SocketAddrV6`] or [`quinn::endpoint::Endpoint`] for more precision. For the local port to bind to, use 0 to get an OS-assigned port.
    ///
    /// # Examples
    ///
    /// Connect to an IPv4 server hosted on localhost (127.0.0.1), which is listening on port 6000. Use 0 as a local bind port to let the OS assign a port.
    /// ```
    /// use bevy_quinnet::client::connection::ConnectionConfiguration;
    /// let config = ConnectionConfiguration::from_strings(
    ///                 "127.0.0.1:6000",
    ///                 "0.0.0.0:0"
    ///             );
    /// ```
    /// Connect to an IPv6 server hosted on localhost (::1), which is listening on port 6000. Use 0 as a local bind port to let the OS assign a port.
    /// ```
    /// use bevy_quinnet::client::connection::ConnectionConfiguration;
    /// let config = ConnectionConfiguration::from_strings(
    ///                 "[::1]:6000",
    ///                 "[::]:0"
    ///             );
    /// ```
    pub fn from_strings(
        server_addr_str: &str,
        local_bind_addr_str: &str,
    ) -> Result<Self, AddrParseError> {
        let server_addr = server_addr_str.parse()?;
        let local_bind_addr = local_bind_addr_str.parse()?;
        Ok(Self::from_addrs(server_addr, local_bind_addr))
    }

    /// Same as [`ConnectionConfiguration::from_strings`], but with an additional `server_hostname` for certificate verification if it is not just the server IP.
    pub fn from_strings_with_name(
        server_addr_str: &str,
        server_hostname: String,
        local_bind_addr_str: &str,
    ) -> Result<Self, AddrParseError> {
        Ok(Self::from_addrs_with_name(
            server_addr_str.parse()?,
            server_hostname,
            local_bind_addr_str.parse()?,
        ))
    }

    /// Creates a new ConnectionConfiguration
    ///
    /// # Arguments
    ///
    /// * `server_ip` - IP address of the server
    /// * `server_port` - Port of the server
    /// * `local_bind_ip` - Local IP address to bind to. The address should usually be a wildcard like `0.0.0.0` (for an IPv4) or `0:0:0:0:0:0:0:0` (for an IPv6), which allow communication with any reachable IPv4 or IPv6 address. See [`std::net::Ipv4Addr`] and [`std::net::Ipv6Addr`] for more precision.
    /// * `local_bind_port` - Local port to bind to. Use 0 to get an OS-assigned port.
    ///
    /// # Examples
    ///
    /// Connect to an IPv4 server hosted on localhost (127.0.0.1), which is listening on port 6000. Use 0 as a local bind port to let the OS assign a port.
    /// ```
    /// use std::net::{IpAddr, Ipv4Addr};
    /// use bevy_quinnet::client::connection::ConnectionConfiguration;
    /// let config = ConnectionConfiguration::from_ips(
    ///                 IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
    ///                 6000,
    ///                 IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
    ///                 0
    ///             );
    /// ```
    pub fn from_ips(
        server_ip: IpAddr,
        server_port: u16,
        local_bind_ip: IpAddr,
        local_bind_port: u16,
    ) -> Self {
        Self::from_addrs(
            SocketAddr::new(server_ip, server_port),
            SocketAddr::new(local_bind_ip, local_bind_port),
        )
    }

    /// Same as [`ConnectionConfiguration::from_ips`], but with an additional `server_hostname` for certificate verification if it is not just the server IP.
    pub fn from_ips_with_name(
        server_ip: IpAddr,
        server_port: u16,
        server_hostname: String,
        local_bind_ip: IpAddr,
        local_bind_port: u16,
    ) -> Self {
        Self::from_addrs_with_name(
            SocketAddr::new(server_ip, server_port),
            server_hostname,
            SocketAddr::new(local_bind_ip, local_bind_port),
        )
    }

    /// Creates a new ConnectionConfiguration
    ///
    /// # Arguments
    ///
    /// * `server_addr` - IP address and port of the server
    /// * `local_bind_addr` - Local address and port to bind to. For the local port to bind to, use 0 to get an OS-assigned port.
    ///
    /// # Examples
    ///
    /// Connect to an IPv4 server hosted on localhost (127.0.0.1), which is listening on port 6000. Use 0 as a local bind port to let the OS assign a port.
    /// ```
    /// use bevy_quinnet::client::connection::ConnectionConfiguration;
    /// use std::{net::{IpAddr, Ipv4Addr, SocketAddr}};
    /// let config = ConnectionConfiguration::from_addrs(
    ///        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 6000),
    ///        SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 0),
    ///    );
    /// ```
    pub fn from_addrs(server_addr: SocketAddr, local_bind_addr: SocketAddr) -> Self {
        Self {
            server_addr,
            server_hostname: server_addr.ip().to_string(),
            local_bind_addr,
        }
    }

    /// Same as [`ConnectionConfiguration::from_addrs`], but with an additional `server_hostname` for certificate verification if it is not just the server IP.
    pub fn from_addrs_with_name(
        server_addr: SocketAddr,
        server_hostname: String,
        local_bind_addr: SocketAddr,
    ) -> Self {
        Self {
            server_addr,
            server_hostname,
            local_bind_addr,
        }
    }
}

/// Current state of a client connection
#[derive(Debug)]
pub(crate) enum ConnectionState {
    Connecting,
    Connected(InternalConnectionRef),
    Disconnected,
}

#[derive(Debug)]
pub struct Connection {
    pub(crate) state: ConnectionState,
    channels: HashMap<ChannelId, Channel>,
    default_channel: Option<ChannelId>,
    last_gen_id: MultiChannelId,
    bytes_from_server_recv: mpsc::Receiver<Bytes>,
    close_sender: broadcast::Sender<()>,

    pub(crate) from_async_client_recv: mpsc::Receiver<ClientAsyncMessage>,
    pub(crate) to_channels_send: mpsc::Sender<ChannelSyncMessage>,
    pub(crate) from_channels_recv: mpsc::Receiver<ChannelAsyncMessage>,
}

impl Connection {
    pub(crate) fn new(
        bytes_from_server_recv: mpsc::Receiver<Bytes>,
        close_sender: broadcast::Sender<()>,
        from_async_client_recv: mpsc::Receiver<ClientAsyncMessage>,
        to_channels_send: mpsc::Sender<ChannelSyncMessage>,
        from_channels_recv: mpsc::Receiver<ChannelAsyncMessage>,
    ) -> Self {
        Self {
            state: ConnectionState::Connecting,
            channels: HashMap::new(),
            last_gen_id: 0,
            default_channel: None,
            bytes_from_server_recv,
            close_sender,
            from_async_client_recv,
            to_channels_send,
            from_channels_recv,
        }
    }

    pub fn receive_message<T: serde::de::DeserializeOwned>(
        &mut self,
    ) -> Result<Option<T>, QuinnetError> {
        match self.receive_payload()? {
            Some(payload) => match bincode::deserialize(&payload) {
                Ok(msg) => Ok(Some(msg)),
                Err(_) => Err(QuinnetError::Deserialization),
            },
            None => Ok(None),
        }
    }

    /// Same as [Connection::receive_message] but will log the error instead of returning it
    pub fn try_receive_message<T: serde::de::DeserializeOwned>(&mut self) -> Option<T> {
        match self.receive_message() {
            Ok(message) => message,
            Err(err) => {
                error!("try_receive_message: {}", err);
                None
            }
        }
    }

    pub fn send_message<T: serde::Serialize>(&self, message: T) -> Result<(), QuinnetError> {
        match self.default_channel {
            Some(channel) => self.send_message_on(channel, message),
            None => Err(QuinnetError::NoDefaultChannel),
        }
    }

    pub fn send_message_on<T: serde::Serialize>(
        &self,
        channel_id: ChannelId,
        message: T,
    ) -> Result<(), QuinnetError> {
        match &self.state {
            ConnectionState::Disconnected => Err(QuinnetError::ConnectionClosed),
            _ => match self.channels.get(&channel_id) {
                Some(channel) => match bincode::serialize(&message) {
                    Ok(payload) => channel.send_payload(payload.into()),
                    Err(_) => Err(QuinnetError::Serialization),
                },
                None => Err(QuinnetError::UnknownChannel(channel_id)),
            },
        }
    }

    /// Same as [Connection::send_message] but will log the error instead of returning it
    pub fn try_send_message<T: serde::Serialize>(&self, message: T) {
        match self.send_message(message) {
            Ok(_) => {}
            Err(err) => error!("try_send_message: {}", err),
        }
    }

    /// Same as [Connection::send_message_on] but will log the error instead of returning it
    pub fn try_send_message_on<T: serde::Serialize>(&self, channel_id: ChannelId, message: T) {
        match self.send_message_on(channel_id, message) {
            Ok(_) => {}
            Err(err) => error!("try_send_message_on: {}", err),
        }
    }

    pub fn send_payload<T: Into<Bytes>>(&self, payload: T) -> Result<(), QuinnetError> {
        match self.default_channel {
            Some(channel) => self.send_payload_on(channel, payload),
            None => Err(QuinnetError::NoDefaultChannel),
        }
    }

    pub fn send_payload_on<T: Into<Bytes>>(
        &self,
        channel_id: ChannelId,
        payload: T,
    ) -> Result<(), QuinnetError> {
        match &self.state {
            ConnectionState::Disconnected => Err(QuinnetError::ConnectionClosed),
            _ => match self.channels.get(&channel_id) {
                Some(channel) => channel.send_payload(payload.into()),
                None => Err(QuinnetError::UnknownChannel(channel_id)),
            },
        }
    }

    /// Same as [Connection::send_payload] but will log the error instead of returning it
    pub fn try_send_payload<T: Into<Bytes>>(&self, payload: T) {
        match self.send_payload(payload) {
            Ok(_) => {}
            Err(err) => error!("try_send_payload: {}", err),
        }
    }

    /// Same as [Connection::send_payload_on] but will log the error instead of returning it
    pub fn try_send_payload_on<T: Into<Bytes>>(&self, channel_id: ChannelId, payload: T) {
        match self.send_payload_on(channel_id, payload) {
            Ok(_) => {}
            Err(err) => error!("try_send_payload_on: {}", err),
        }
    }

    pub fn receive_payload(&mut self) -> Result<Option<Bytes>, QuinnetError> {
        match &self.state {
            ConnectionState::Disconnected => Err(QuinnetError::ConnectionClosed),
            _ => match self.bytes_from_server_recv.try_recv() {
                Ok(msg_payload) => Ok(Some(msg_payload)),
                Err(err) => match err {
                    TryRecvError::Empty => Ok(None),
                    TryRecvError::Disconnected => Err(QuinnetError::InternalChannelClosed),
                },
            },
        }
    }

    /// Same as [Connection::receive_payload] but will log the error instead of returning it
    pub fn try_receive_payload(&mut self) -> Option<Bytes> {
        match self.receive_payload() {
            Ok(payload) => payload,
            Err(err) => {
                error!("try_receive_payload: {}", err);
                None
            }
        }
    }

    /// Immediately prevents new messages from being sent on the connection and signal the connection to closes all its background tasks. Before trully closing, the connection will wait for all buffered messages in all its opened channels to be properly sent according to their respective channel type.
    pub(crate) fn disconnect(&mut self) -> Result<(), QuinnetError> {
        match &self.state {
            ConnectionState::Disconnected => Ok(()),
            _ => {
                self.state = ConnectionState::Disconnected;
                match self.close_sender.send(()) {
                    Ok(_) => Ok(()),
                    Err(_) => {
                        // The only possible error for a send is that there is no active receivers, meaning that the tasks are already terminated.
                        Err(QuinnetError::ConnectionAlreadyClosed)
                    }
                }
            }
        }
    }

    pub(crate) fn try_disconnect(&mut self) {
        match &self.disconnect() {
            Ok(_) => (),
            Err(err) => error!("Failed to properly close clonnection: {}", err),
        }
    }

    pub fn is_connected(&self) -> bool {
        match self.state {
            ConnectionState::Connected(_) => true,
            _ => false,
        }
    }

    /// Returns statistics about the current connection if connected.
    pub fn stats(&self) -> Option<ConnectionStats> {
        match &self.state {
            ConnectionState::Connected(connection) => Some(connection.stats()),
            _ => None,
        }
    }

    /// Opens a channel of the requested [ChannelType] and returns its [ChannelId].
    ///
    /// By default, when starting a [Connection]], Quinnet creates 1 channel instance of each [ChannelType], each with their own [ChannelId]. Among those, there is a `default` channel which will be used when you don't specify the channel. At startup, this default channel is a [ChannelType::OrderedReliable] channel.
    ///
    /// If no channels were previously opened, the opened channel will be the new default channel.
    ///
    /// Can fail if the Connection is closed.
    pub fn open_channel(&mut self, channel_type: ChannelType) -> Result<ChannelId, QuinnetError> {
        let channel_id = get_channel_id_from_type(channel_type, || {
            self.last_gen_id += 1;
            self.last_gen_id
        });
        match self.channels.contains_key(&channel_id) {
            true => Ok(channel_id),
            false => self.create_channel(channel_id),
        }
    }

    /// Closes the channel with the corresponding [ChannelId].
    ///
    /// No new messages will be able to be sent on this channel, however, the channel will properly try to send all the messages that were previously pushed to it, according to its [ChannelType], before fully closing.
    ///
    /// If the closed channel is the current default channel, the default channel gets set to `None`.
    ///
    /// Can fail if the [ChannelId] is unknown, or if the channel is already closed.
    pub fn close_channel(&mut self, channel_id: ChannelId) -> Result<(), QuinnetError> {
        match self.channels.remove(&channel_id) {
            Some(channel) => {
                if Some(channel_id) == self.default_channel {
                    self.default_channel = None;
                }
                channel.close()
            }
            None => Err(QuinnetError::UnknownChannel(channel_id)),
        }
    }

    /// Set the default channel
    pub fn set_default_channel(&mut self, channel_id: ChannelId) {
        self.default_channel = Some(channel_id);
    }

    /// Get the default Channel Id
    pub fn get_default_channel(&self) -> Option<ChannelId> {
        self.default_channel
    }

    fn create_channel(&mut self, channel_id: ChannelId) -> Result<ChannelId, QuinnetError> {
        let (bytes_to_channel_send, bytes_to_channel_recv) =
            mpsc::channel::<Bytes>(DEFAULT_MESSAGE_QUEUE_SIZE);
        let (channel_close_send, channel_close_recv) =
            mpsc::channel(DEFAULT_KILL_MESSAGE_QUEUE_SIZE);

        match self
            .to_channels_send
            .try_send(ChannelSyncMessage::CreateChannel {
                channel_id,
                bytes_to_channel_recv,
                channel_close_recv,
            }) {
            Ok(_) => {
                let channel = Channel::new(bytes_to_channel_send, channel_close_send);
                self.channels.insert(channel_id, channel);
                if self.default_channel.is_none() {
                    self.default_channel = Some(channel_id);
                }

                Ok(channel_id)
            }
            Err(err) => match err {
                TrySendError::Full(_) => Err(QuinnetError::FullQueue),
                TrySendError::Closed(_) => Err(QuinnetError::InternalChannelClosed),
            },
        }
    }
}

pub(crate) async fn connection_task(
    connection_id: ConnectionId,
    config: ConnectionConfiguration,
    cert_mode: CertificateVerificationMode,
    to_sync_client_send: mpsc::Sender<ClientAsyncMessage>,
    to_channels_recv: mpsc::Receiver<ChannelSyncMessage>,
    from_channels_send: mpsc::Sender<ChannelAsyncMessage>,
    close_recv: broadcast::Receiver<()>,
    bytes_from_server_send: mpsc::Sender<Bytes>,
) {
    info!(
        "Connection {} trying to connect to server on: {} ...",
        connection_id, config.server_addr
    );

    let client_cfg = configure_client(cert_mode, to_sync_client_send.clone())
        .expect("Failed to configure client");

    let mut endpoint =
        Endpoint::client(config.local_bind_addr).expect("Failed to create client endpoint");
    endpoint.set_default_client_config(client_cfg);

    let connection = endpoint
        .connect(config.server_addr, &config.server_hostname)
        .expect("Failed to connect: configuration error")
        .await;
    match connection {
        Err(e) => error!(
            "Connection {}, error while connecting: {}",
            connection_id, e
        ),
        Ok(connection) => {
            info!(
                "Connection {} connected to {}",
                connection_id,
                connection.remote_address()
            );

            to_sync_client_send
                .send(ClientAsyncMessage::Connected(connection.clone()))
                .await
                .expect("Failed to signal connection to sync client");

            // Spawn a task to listen for the underlying connection being closed
            {
                let conn = connection.clone();
                let to_sync_client = to_sync_client_send.clone();
                tokio::spawn(async move {
                    let conn_err = conn.closed().await;
                    info!("Connection {} disconnected: {}", connection_id, conn_err);
                    // If we requested the connection to close, channel may have been closed already.
                    if !to_sync_client.is_closed() {
                        to_sync_client
                            .send(ClientAsyncMessage::ConnectionClosed(conn_err))
                            .await
                            .expect("Failed to signal connection lost to sync client");
                    }
                })
            };

            // Spawn a task to listen for streams opened by the server
            {
                let close_recv = close_recv.resubscribe();
                let connection_handle = connection.clone();
                let bytes_incoming_send = bytes_from_server_send.clone();
                tokio::spawn(async move {
                    reliable_receiver_task(
                        connection_id,
                        connection_handle,
                        close_recv,
                        bytes_incoming_send,
                    )
                    .await
                });
            }

            // Spawn a task to listen for datagrams sent by the server
            {
                let close_recv = close_recv.resubscribe();
                let connection_handle = connection.clone();
                let bytes_incoming_send = bytes_from_server_send.clone();
                tokio::spawn(async move {
                    unreliable_receiver_task(
                        connection_id,
                        connection_handle,
                        close_recv,
                        bytes_incoming_send,
                    )
                    .await
                });
            }

            // Spawn a task to handle send channels for this connection
            tokio::spawn(async move {
                channels_task(connection, close_recv, to_channels_recv, from_channels_send).await
            });
        }
    }
}

fn configure_client(
    cert_mode: CertificateVerificationMode,
    to_sync_client: mpsc::Sender<ClientAsyncMessage>,
) -> Result<ClientConfig, Box<dyn Error>> {
    match cert_mode {
        CertificateVerificationMode::SkipVerification => {
            let crypto = rustls::ClientConfig::builder()
                .with_safe_defaults()
                .with_custom_certificate_verifier(SkipServerVerification::new())
                .with_no_client_auth();

            Ok(ClientConfig::new(Arc::new(crypto)))
        }
        CertificateVerificationMode::SignedByCertificateAuthority => {
            Ok(ClientConfig::with_native_roots())
        }
        CertificateVerificationMode::TrustOnFirstUse(config) => {
            let (store, store_file) = load_known_hosts_store_from_config(config.known_hosts)?;
            let crypto = rustls::ClientConfig::builder()
                .with_safe_defaults()
                .with_custom_certificate_verifier(TofuServerVerification::new(
                    store,
                    config.verifier_behaviour,
                    to_sync_client,
                    store_file,
                ))
                .with_no_client_auth();
            Ok(ClientConfig::new(Arc::new(crypto)))
        }
    }
}
