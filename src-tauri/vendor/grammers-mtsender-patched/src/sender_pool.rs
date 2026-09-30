// Copyright 2020 - developers of the `grammers` project.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use crate::configuration::ConnectionParams;
use crate::errors::ReadError;
use crate::{InvocationError, Sender, ServerAddr, connect, connect_with_auth};
use grammers_mtproto::{mtp, transport};
use grammers_session::Session;
use grammers_session::defs::DcOption;
use grammers_session::updates::UpdatesLike;
use grammers_tl_types::{self as tl, enums};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddrV4, SocketAddrV6};
use std::ops::ControlFlow;
use std::sync::Arc;
use std::{fmt, panic};
use tokio::task::AbortHandle;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinSet,
};

pub(crate) type Transport = transport::Full;

type InvokeResponse = Vec<u8>;

enum Request {
    Invoke {
        dc_id: i32,
        body: Vec<u8>,
        tx: oneshot::Sender<Result<InvokeResponse, InvocationError>>,
    },
    Disconnect {
        dc_id: i32,
    },
    Quit,
}

struct Rpc {
    body: Vec<u8>,
    tx: oneshot::Sender<Result<InvokeResponse, InvocationError>>,
}

struct ConnectionInfo {
    dc_id: i32,
    rpc_tx: mpsc::UnboundedSender<Rpc>,
    abort_handle: AbortHandle,
}

/// Cheaply cloneable handle to interact with its [`SenderPoolRunner`].
#[derive(Clone)]
pub struct SenderPoolHandle(mpsc::UnboundedSender<Request>);

/// Named type holding the actual runner and initial handles. The entry point.
pub struct SenderPool {
    /// The single mutable instance responsible for driving I/O.
    ///
    /// Connections are created on-demand, so any errors while the pool
    /// is running can only be retrieved with one of the [`SenderPool::handle`]s.
    pub runner: SenderPoolRunner,
    /// Starting handle attached to the [`SenderPool::runner`].
    ///
    /// This is the only way to interact with the runner once it's running.
    pub handle: SenderPoolHandle,
    /// The single mutable channel through which updates received
    /// from the network by the [`SenderPool::runner`] are delivered.
    ///
    /// Update handling must be processed in a sequential manner,
    /// so this is a separate instance with no way to clone it.
    pub updates: mpsc::UnboundedReceiver<UpdatesLike>,
}

/// Manages and runs a pool of zero or more [`Sender`]s.
///
/// Use [`SenderPool::new`] to create an instance of this type and associated channels.
pub struct SenderPoolRunner {
    /// The session that will contain and persist the datacenter options.
    pub session: Arc<dyn Session>,
    /// Developer's [Application Identifier](https://core.telegram.org/myapp).
    ///
    /// This parameter required to initialize new connections.
    pub api_id: i32,
    /// Remaining connection parameters used to [`tl::functions::InitConnection`].
    pub connection_params: ConnectionParams,
    request_rx: mpsc::UnboundedReceiver<Request>,
    updates_tx: mpsc::UnboundedSender<UpdatesLike>,
    connections: Vec<ConnectionInfo>,
    connection_pool: JoinSet<Result<(), ReadError>>,
}

impl SenderPoolHandle {
    /// Communicate with the running [`SenderPoolRunner`] instance
    /// to invoke the serialized request body in the specified datacenter.
    pub async fn invoke_in_dc(
        &self,
        dc_id: i32,
        body: Vec<u8>,
    ) -> Result<InvokeResponse, InvocationError> {
        let (tx, rx) = oneshot::channel();
        self.0
            .send(Request::Invoke { dc_id, body, tx })
            .map_err(|_| InvocationError::Dropped)?;
        rx.await.map_err(|_| InvocationError::Dropped)?
    }

    /// Communicate with the running [`SenderPoolRunner`] instance
    /// to drop any active connections to the given datacenter.
    ///
    /// Has no effect if there was no connection to the datacenter.
    ///
    /// This is useful after datacenter migrations during sign in,
    /// when the old connection is known to not be needed anymore.
    pub fn disconnect_from_dc(&self, dc_id: i32) -> bool {
        self.0.send(Request::Disconnect { dc_id }).is_ok()
    }

    /// Communicate with the running [`SenderPoolRunner`] instance
    /// to drop all active connections and gracefully stop running.
    pub fn quit(&self) -> bool {
        self.0.send(Request::Quit).is_ok()
    }
}

impl SenderPool {
    /// Creates a new sender pool instance with default configuration,
    /// attached to the given session and using the provided
    /// [Application Identifier](https://core.telegram.org/myapp)
    /// belonging to the developer.
    ///
    /// Session instance **should not** be reused by multiple pools at the same time.
    /// The session instance will only be used to query datacenter options and persist
    /// any permanent Authorization Keys generated for previously-unconncected datacenters.
    pub fn new<S: Session + 'static>(session: Arc<S>, api_id: i32) -> Self {
        Self::with_configuration(session, api_id, Default::default())
    }

    /// Creates a new sender pool with non-[`ConnectionParams::default`] configuration.
    pub fn with_configuration<S: Session + 'static>(
        session: Arc<S>,
        api_id: i32,
        connection_params: ConnectionParams,
    ) -> Self {
        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let (updates_tx, updates_rx) = mpsc::unbounded_channel();

        Self {
            runner: SenderPoolRunner {
                session: session as Arc<dyn Session>,
                api_id,
                connection_params,
                request_rx,
                updates_tx,
                connections: Vec::new(),
                connection_pool: JoinSet::new(),
            },
            handle: SenderPoolHandle(request_tx),
            updates: updates_rx,
        }
    }
}

impl SenderPoolRunner {
    /// Run the sender pool until [`SenderPoolHandle::quit`] is called or the returned future is dropped.
    ///
    /// Connections will be initiated on-demand whenever the first request to a datacenter is made.
    pub async fn run(mut self) {
        loop {
            tokio::select! {
                biased;
                completion = self.connection_pool.join_next(), if !self.connection_pool.is_empty() => {
                    if let Err(err) = completion.unwrap() {
                        if let Ok(reason) = err.try_into_panic() {
                            panic::resume_unwind(reason);
                        }
                    }
                    self.connections
                        .retain(|connection| !connection.abort_handle.is_finished());
                }
                request = self.request_rx.recv() => {
                    let flow = if let Some(request) = request {
                        self.process_request(request).await
                    } else {
                        ControlFlow::Break(())
                    };
                    match flow {
                        ControlFlow::Continue(_) => continue,
                        ControlFlow::Break(_) => break,
                    }
                }
            }
        }

        self.connections.clear(); // drop all channels to cause the `run_sender` loops to stop
        self.connection_pool.join_all().await;
    }

    async fn process_request(&mut self, request: Request) -> ControlFlow<()> {
        match request {
            Request::Invoke { dc_id, body, tx } => {
                let Some(mut dc_option) = self.session.dc_option(dc_id) else {
                    let _ = tx.send(Err(InvocationError::InvalidDc));
                    return ControlFlow::Continue(());
                };

                let connection = match self
                    .connections
                    .iter()
                    .find(|connection| connection.dc_id == dc_id)
                {
                    Some(connection) => connection,
                    None => {
                        let sender = match self.connect_sender(&dc_option).await {
                            Ok(t) => t,
                            Err(e) => {
                                let _ = tx.send(Err(e));
                                return ControlFlow::Continue(());
                            }
                        };

                        dc_option.auth_key = Some(sender.auth_key());
                        self.session.set_dc_option(&dc_option);

                        let (rpc_tx, rpc_rx) = mpsc::unbounded_channel();
                        let abort_handle = self.connection_pool.spawn(run_sender(
                            sender,
                            rpc_rx,
                            self.updates_tx.clone(),
                        ));
                        self.connections.push(ConnectionInfo {
                            dc_id,
                            rpc_tx,
                            abort_handle,
                        });
                        self.connections.last().unwrap()
                    }
                };
                let _ = connection.rpc_tx.send(Rpc { body, tx });
                ControlFlow::Continue(())
            }
            Request::Disconnect { dc_id } => {
                self.connections.retain(|connection| {
                    if connection.dc_id == dc_id {
                        connection.abort_handle.abort();
                        false
                    } else {
                        true
                    }
                });
                ControlFlow::Continue(())
            }
            Request::Quit => ControlFlow::Break(()),
        }
    }

    async fn connect_sender(
        &mut self,
        dc_option: &DcOption,
    ) -> Result<Sender<transport::Full, mtp::Encrypted>, InvocationError> {
        let transport = transport::Full::new;

        #[cfg(feature = "proxy")]
        let addr = || {
            if let Some(proxy) = self.connection_params.proxy_url.clone() {
                ServerAddr::Proxied {
                    address: dc_option.ipv4.into(),
                    proxy,
                }
            } else {
                ServerAddr::Tcp {
                    address: dc_option.ipv4.into(),
                }
            }
        };
        #[cfg(not(feature = "proxy"))]
        let addr = || ServerAddr::Tcp {
            address: dc_option.ipv4.into(),
        };

        let init_connection = tl::functions::InvokeWithLayer {
            layer: tl::LAYER,
            query: tl::functions::InitConnection {
                api_id: self.api_id,
                device_model: self.connection_params.device_model.clone(),
                system_version: self.connection_params.system_version.clone(),
                app_version: self.connection_params.app_version.clone(),
                system_lang_code: self.connection_params.system_lang_code.clone(),
                lang_pack: "".into(),
                lang_code: self.connection_params.lang_code.clone(),
                proxy: None,
                params: None,
                query: tl::functions::help::GetConfig {},
            },
        };

        let mut sender = if let Some(auth_key) = dc_option.auth_key {
            connect_with_auth(transport(), addr(), auth_key).await?
        } else {
            connect(transport(), addr()).await?
        };

        let enums::Config::Config(remote_config) = match sender.invoke(&init_connection).await {
            Ok(config) => config,
            Err(InvocationError::Transport(transport::Error::BadStatus { status: 404 })) => {
                sender = connect(transport(), addr()).await?;
                sender.invoke(&init_connection).await?
            }
            Err(e) => return Err(dbg!(e).into()),
        };

        self.update_config(remote_config);

        Ok(sender)
    }

    fn update_config(&mut self, config: tl::types::Config) {
        config
            .dc_options
            .iter()
            .map(|tl::enums::DcOption::Option(option)| option)
            .filter(|option| !option.media_only && !option.tcpo_only && option.r#static)
            .for_each(|option| {
                let mut dc_option = self
                    .session
                    .dc_option(option.id)
                    .unwrap_or_else(|| DcOption {
                        id: option.id,
                        ipv4: SocketAddrV4::new(Ipv4Addr::from_bits(0), 0),
                        ipv6: SocketAddrV6::new(Ipv6Addr::from_bits(0), 0, 0, 0),
                        auth_key: None,
                    });
                if option.ipv6 {
                    dc_option.ipv6 = SocketAddrV6::new(
                        option
                            .ip_address
                            .parse()
                            .expect("Telegram to return a valid IPv6 address"),
                        option.port as _,
                        0,
                        0,
                    );
                } else {
                    dc_option.ipv4 = SocketAddrV4::new(
                        option
                            .ip_address
                            .parse()
                            .expect("Telegram to return a valid IPv4 address"),
                        option.port as _,
                    );
                    if dc_option.ipv6.ip().to_bits() == 0 {
                        dc_option.ipv6 = SocketAddrV6::new(
                            dc_option.ipv4.ip().to_ipv6_mapped(),
                            dc_option.ipv4.port(),
                            0,
                            0,
                        )
                    }
                }
            });
    }
}

async fn run_sender(
    mut sender: Sender<Transport, grammers_mtproto::mtp::Encrypted>,
    mut rpc_rx: mpsc::UnboundedReceiver<Rpc>,
    updates: mpsc::UnboundedSender<UpdatesLike>,
) -> Result<(), ReadError> {
    loop {
        tokio::select! {
            step = sender.step() => match step {
                Ok(all_new_updates) => all_new_updates.into_iter().for_each(|new_updates| {
                    let _ = updates.send(new_updates);
                }),
                Err(err) => break Err(err),
            },
            rpc = rpc_rx.recv() => match rpc {
                Some(rpc) => sender.enqueue_body(rpc.body, rpc.tx),
                None => break Ok(()),
            },
        }
    }
}

impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invoke { dc_id, body, tx } => f
                .debug_struct("Invoke")
                .field("dc_id", dc_id)
                .field(
                    "request",
                    &body[..4]
                        .try_into()
                        .map(|constructor_id| tl::name_for_id(u32::from_le_bytes(constructor_id)))
                        .unwrap_or("?"),
                )
                .field("tx", tx)
                .finish(),
            Self::Disconnect { dc_id } => {
                f.debug_struct("Disconnect").field("dc_id", dc_id).finish()
            }
            Self::Quit => write!(f, "Quit"),
        }
    }
}
