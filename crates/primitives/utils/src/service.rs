//! Madara Services Architecture
//!
//! Madara follows a [microservice](microservices) architecture to simplify the
//! composability and parallelism of its services. That is to say services can
//! be started in different orders, at different points in the program's
//! execution, stopped and even restarted. The advantage in parallelism arises
//! from the fact that each services runs as its own non-blocking asynchronous
//! task which allows for high throughput. Inter-service communication is done
//! via [tokio::sync] or more often through direct database reads and writes.
//!
//! # The [Service] trait
//!
//! This is the backbone of Madara service and serves as a common interface to
//! all services. The [Service] trait specifies how a service must start as well
//! as how to _identify_ it. For reasons of atomicity, services are currently
//! identified as a single [std::sync::atomic::AtomicU8]. More about this later.
//!
//! Services are started from [Service::start] using [ServiceRunner::service_loop].
//! [ServiceRunner::service_loop] is a function which takes in a future: this
//! future represents the main loop of your service, and should run until your
//! service completes or is canceled.
//!
//! > **Note**
//! > It is assumed that services can and might be restarted. You have the
//! > responsibility to ensure this is possible. This means you should make sure
//! > not to use the like of [std::mem::take] or similar on your service inside
//! > [Service::start]. In general, make sure your service still contains all
//! > the necessary information it needs to restart. This might mean certain
//! > attributes need to be stored as a [std::sync::Arc] and cloned so that the
//! > future in [ServiceRunner::service_loop] can safely take ownership of them.
//!
//! It is part of the contract of the [Service] trait that calls to
//! [ServiceRunner::service_loop] should not complete until the service has
//! _finished_ execution (this should be evident by the name) as this is used
//! to mark a service as complete and therefore ready to restart. Services where
//! [ServiceRunner::service_loop] completes _before_ the service has finished
//! execution will be automatically marked for shutdown as a safety mechanism.
//! This is done as a safeguard to avoid an invalid state where it would be
//! impossible for the node to shutdown.
//!
//! ## An incorrect implementation of the [Service] trait
//!
//! ```rust
//! pub struct MyService;
//!
//! impl Service for MyService {
//!     async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
//!         runner.service_loop(move |ctx| async {
//!             tokio::task::spawn(async {
//!                 tokio::time::sleep(std::time::Duration::MAX).await;
//!             });
//!
//!             // This is incorrect, as the future passed to service_loop will
//!             // resolve before the task spawned above completes, meaning
//!             // Madara will incorrectly mark this service as ready to restart.
//!             // In a more complex scenario, this means we might enter an
//!             // invalid state!
//!             anyhow::Ok(());
//!         });
//!
//!         anyhow::Ok(())
//!     }
//!
//!     fn id(&self) -> MadaraService {
//!         MadaraService::Monitor
//!     }
//! }
//! ```
//!
//! ## A correct implementation of the [Service] trait
//!
//! ```rust
//! pub struct MyService;
//!
//! impl Service for MyService {
//!     async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
//!         runner.service_loop(move |ctx| async {
//!             tokio::time::sleep(std::time::Duration::MAX).await;
//!
//!             // This is correct, as the future passed to service_loop will
//!             // only resolve once the task above completes, so Madara can
//!             // correctly mark this service as ready to restart.
//!             anyhow::Ok(());
//!         });
//!
//!         anyhow::Ok(())
//!     }
//!
//!     fn id(&self) -> MadaraService {
//!         MadaraService::Monitor
//!     }
//! }
//! ```
//!
//! Or if you really need to spawn a background task:
//!
//! ```rust
//! pub struct MyService;
//!
//! impl Service for MyService {
//!     async fn start<'a>(&mut self, runner: ServiceRunner<'a>) -> anyhow::Result<()> {
//!         runner.service_loop(move |ctx| async {
//!             let ctx1 = ctx.clone();
//!             tokio::task::spawn(async move {
//!                 tokio::select! {
//!                     _ = tokio::time::sleep(std::time::Duration::MAX) = {},
//!                     _ = ctx1.cancelled() => {},
//!                 }
//!             });
//!
//!             ctx.cancelled().await;
//!
//!             // This is correct, as even though we are spawning a background
//!             // task we have implemented a cancellation mechanism with ctx
//!             // and are waiting for that cancellation in service_loop.
//!             anyhow::Ok(());
//!         });
//!
//!         anyhow::Ok(())
//!     }
//!
//!     fn id(&self) -> MadaraService {
//!         MadaraService::Monitor
//!     }
//! }
//! ```
//!
//! This sort of problem generally arises in cases similar to the example above,
//! where the service's role is to spawn another background task. This is can
//! happen when the service needs to start a server for example. Either avoid
//! spawning a detached task or use mechanisms such as [ServiceContext::cancelled]
//! to await for the service's completion.
//!
//! Note that by design service shutdown is designed to be manual. We still
//! implement a [SERVICE_GRACE_PERIOD] which is the maximum duration a service
//! is allowed to take to shutdown, after which it is forcefully canceled. This
//! should not happen in practice but helps avoid cases where someone forgets to
//! implement a cancellation check. More on this in the next section.
//!
//! # Cancellation status and inter-process requests
//!
//! # Service orchestration
//!
//! # Atomic status updates
//!
//! ## Service status updates
//!
//! ## Node status update
//!
//! [microservices]: https://en.wikipedia.org/wiki/Microservices

use anyhow::Context;
use futures::Future;
use serde::{Deserialize, Serialize};
use std::{
    fmt::{Debug, Display},
    panic,
    sync::Arc,
    time::Duration,
};
use tokio::task::JoinSet;

pub const SERVICE_COUNT: usize = 8;
pub const SERVICE_GRACE_PERIOD: Duration = Duration::from_secs(10);

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum MadaraService {
    #[default]
    Monitor = 0,
    Database = 1,
    L1Sync = 2,
    L2Sync = 4,
    BlockProduction = 8,
    RpcUser = 16,
    RpcAdmin = 32,
    Gateway = 64,
    Telemetry = 128,
}

impl Display for MadaraService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::Monitor => "none",
                Self::Database => "database",
                Self::L1Sync => "l1 sync",
                Self::L2Sync => "l2 sync",
                Self::BlockProduction => "block production",
                Self::RpcUser => "rpc user",
                Self::RpcAdmin => "rpc admin",
                Self::Gateway => "gateway",
                Self::Telemetry => "telemetry",
            }
        )
    }
}

impl From<u8> for MadaraService {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Monitor,
            1 => Self::Database,
            2 => Self::L1Sync,
            4 => Self::L2Sync,
            8 => Self::BlockProduction,
            16 => Self::RpcUser,
            32 => Self::RpcAdmin,
            64 => Self::Gateway,
            _ => Self::Telemetry,
        }
    }
}

#[derive(PartialEq, Eq, Clone, Copy, Default, Serialize, Deserialize)]
pub enum MadaraServiceStatus {
    On,
    #[default]
    Off,
}

impl Display for MadaraServiceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                Self::On => "on",
                Self::Off => "off",
            }
        )
    }
}

impl std::ops::BitOr for MadaraServiceStatus {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        if self.is_on() || rhs.is_on() {
            MadaraServiceStatus::On
        } else {
            MadaraServiceStatus::Off
        }
    }
}

impl std::ops::BitAnd for MadaraServiceStatus {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        if self.is_on() && rhs.is_on() {
            MadaraServiceStatus::On
        } else {
            MadaraServiceStatus::Off
        }
    }
}

impl From<bool> for MadaraServiceStatus {
    fn from(value: bool) -> Self {
        match value {
            true => Self::On,
            false => Self::Off,
        }
    }
}

impl MadaraServiceStatus {
    pub fn is_on(&self) -> bool {
        self == &MadaraServiceStatus::On
    }

    pub fn is_off(&self) -> bool {
        self == &MadaraServiceStatus::Off
    }
}

#[repr(transparent)]
#[derive(Default)]
pub struct MadaraServiceMask(std::sync::atomic::AtomicU8);

impl MadaraServiceMask {
    #[cfg(feature = "testing")]
    pub fn new_for_testing() -> Self {
        Self(std::sync::atomic::AtomicU8::new(u8::MAX))
    }

    #[inline(always)]
    pub fn status(&self, svcs: u8) -> MadaraServiceStatus {
        (self.0.load(std::sync::atomic::Ordering::SeqCst) & svcs > 0).into()
    }

    #[inline(always)]
    pub fn is_active_some(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::SeqCst) > 0
    }

    #[inline(always)]
    pub fn activate(&self, svc: MadaraService) -> MadaraServiceStatus {
        let prev = self.0.fetch_or(svc as u8, std::sync::atomic::Ordering::SeqCst);
        (prev & svc as u8 > 0).into()
    }

    #[inline(always)]
    pub fn deactivate(&self, svc: MadaraService) -> MadaraServiceStatus {
        let svc = svc as u8;
        let prev = self.0.fetch_and(!svc, std::sync::atomic::Ordering::SeqCst);
        (prev & svc > 0).into()
    }

    fn active_set(&self) -> Vec<MadaraService> {
        let mut i = MadaraService::Telemetry as u8;
        let state = self.0.load(std::sync::atomic::Ordering::SeqCst);
        let mut set = Vec::with_capacity(SERVICE_COUNT);

        while i > 0 {
            let mask = i & state;

            if mask > 0 {
                set.push(MadaraService::from(mask));
            }

            i >>= 1;
        }

        set
    }
}

#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum MadaraState {
    #[default]
    Starting,
    Warp,
    Running,
    Shutdown,
}

impl From<u8> for MadaraState {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Starting,
            1 => Self::Warp,
            2 => Self::Running,
            _ => Self::Shutdown,
        }
    }
}

#[derive(Clone, Copy)]
pub struct ServiceTransport {
    pub svc: MadaraService,
    pub status: MadaraServiceStatus,
}

/// Atomic state and cancellation context associated to a Service.
///
/// # Scope
///
/// You can create a hierarchy of services by calling `ServiceContext::branch_local`.
/// Services are said to be in the same _local scope_ if they inherit the same
/// `token_local` cancellation token. You can think of services being local
/// if they can cancel each other without affecting the rest of the app (this
/// is not exact but it serves as a good mental model).
///
/// All services which descend from the same context are also said to be in the
/// same _global scope_, that is to say any service in this scope can cancel
/// _all_ other services in the same scope (including child services) at any
/// time. This is true of services in the same [ServiceGroup] for example.
///
/// # Services
///
/// - A services is said to be a _child service_ if it uses a context created
///   with `ServiceContext::branch_local`
///
/// - A service is said to be a _parent service_ if it uses a context which was
///   used to create child services.
///
/// > A parent services can always cancel all of its child services, but a child
/// > service cannot cancel its parent service.
pub struct ServiceContext {
    token_global: tokio_util::sync::CancellationToken,
    token_local: Option<tokio_util::sync::CancellationToken>,
    services: Arc<MadaraServiceMask>,
    service_update_sender: Arc<tokio::sync::broadcast::Sender<ServiceTransport>>,
    service_update_receiver: Option<tokio::sync::broadcast::Receiver<ServiceTransport>>,
    state: Arc<std::sync::atomic::AtomicU8>,
    id: MadaraService,
}

impl Clone for ServiceContext {
    fn clone(&self) -> Self {
        Self {
            token_global: self.token_global.clone(),
            token_local: self.token_local.clone(),
            services: Arc::clone(&self.services),
            service_update_sender: Arc::clone(&self.service_update_sender),
            service_update_receiver: None,
            state: Arc::clone(&self.state),
            id: self.id,
        }
    }
}

impl Default for ServiceContext {
    fn default() -> Self {
        Self {
            token_global: tokio_util::sync::CancellationToken::new(),
            token_local: None,
            services: Arc::new(MadaraServiceMask::default()),
            service_update_sender: Arc::new(tokio::sync::broadcast::channel(SERVICE_COUNT).0),
            service_update_receiver: None,
            state: Arc::new(std::sync::atomic::AtomicU8::new(MadaraState::default() as u8)),
            id: MadaraService::Monitor,
        }
    }
}

impl ServiceContext {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(feature = "testing")]
    pub fn new_for_testing() -> Self {
        Self { services: Arc::new(MadaraServiceMask::new_for_testing()), ..Default::default() }
    }

    pub fn new_with_services(services: Arc<MadaraServiceMask>) -> Self {
        Self { services, ..Default::default() }
    }

    /// Stops all services under the same global context scope.
    pub fn cancel_global(&self) {
        tracing::info!("🔌 Gracefully shutting down node");

        self.token_global.cancel();
    }

    /// Stops all services under the same local context scope.
    ///
    /// A local context is created by calling `branch_local` and allows you to
    /// reduce the scope of cancellation only to those services which will use
    /// the new context.
    pub fn cancel_local(&self) {
        self.token_local.as_ref().unwrap_or(&self.token_global).cancel();
    }

    /// A future which completes when the service associated to this
    /// [ServiceContext] is canceled.
    ///
    /// A service is canceled after calling [ServiceContext::cancel_local],
    /// [ServiceContext::cancel_global] or if it is marked as disabled with
    /// [ServiceContext::service_remove].
    ///
    /// Use this to race against other futures in a [tokio::select] for example.
    #[inline(always)]
    pub async fn cancelled(&mut self) {
        if self.state() != MadaraState::Shutdown {
            if self.service_update_receiver.is_none() {
                self.service_update_receiver = Some(self.service_update_sender.subscribe());
            }

            let mut rx = self.service_update_receiver.take().expect("Receiver was set above");
            let token_global = &self.token_global;
            let token_local = self.token_local.as_ref().unwrap_or(&self.token_global);

            loop {
                // We keep checking for service status updates until a token has
                // been canceled or this service was deactivated
                let res = tokio::select! {
                    svc = rx.recv() => svc.ok(),
                    _ = token_global.cancelled() => break,
                    _ = token_local.cancelled() => break
                };

                if let Some(ServiceTransport { svc, status }) = res {
                    if svc == self.id && status == MadaraServiceStatus::Off {
                        return;
                    }
                }
            }
        }
    }

    /// Checks if the service associated to this [ServiceContext] was canceled.
    ///
    /// This happens after calling [ServiceContext::cancel_local],
    /// [ServiceContext::cancel_global].or [ServiceContext::service_remove].
    ///
    /// # Limitations
    ///
    /// This function should _not_ be used when waiting on potentially
    /// blocking futures which can be canceled without entering an invalid
    /// state. The latter is important, so let's break this down.
    ///
    /// - _blocking future_: this is blocking at a service level, not at the
    ///   node level. A blocking task in this sense in a task which prevents a
    ///   service from making progress in its execution, but not necessarily the
    ///   rest of the node. A prime example of this is when you are waiting on
    ///   a channel, and updates to that channel are sparse, or even unique.
    ///
    /// - _entering an invalid state_: the entire point of [ServiceContext] is
    ///   to allow services to gracefully shutdown. We do not want to be, for
    ///   example, racing each service against a global cancellation future, as
    ///   not every service might be cancellation safe. Put differently, we do
    ///   not want to stop in the middle of a critical computation before it has
    ///   been saved to disk.
    ///
    /// Putting this together, [ServiceContext::is_cancelled] is only suitable
    /// for checking cancellation alongside tasks which will not block the
    /// running service, or in very specific circumstances where waiting on a
    /// blocking future has higher precedence than shutting down the node.
    ///
    /// Examples of when to use [ServiceContext::is_cancelled]:
    ///
    /// - All your computation does is sleep or tick away a short period of
    ///   time.
    /// - You are checking for cancellation inside of synchronous code.
    ///
    /// If this does not describe your usage, and you are waiting on a blocking
    /// future, which is cancel-safe and which does not risk putting the node
    /// in an invalid state if cancelled, then you should be using
    /// [ServiceContext::cancelled] instead.
    #[inline(always)]
    pub fn is_cancelled(&self) -> bool {
        self.token_global.is_cancelled()
            || self.token_local.as_ref().map(|t| t.is_cancelled()).unwrap_or(false)
            || self.services.status(self.id as u8) == MadaraServiceStatus::Off
            || self.state() == MadaraState::Shutdown
    }

    /// The id of service associated to this [ServiceContext]
    pub fn id(&self) -> MadaraService {
        self.id
    }

    /// Copies the context, maintaining its scope but with a new id.
    pub fn with_id(mut self, id: MadaraService) -> Self {
        self.id = id;
        self
    }

    /// Copies the context into a new local scope.
    ///
    /// Any service which uses this new context will be able to cancel the
    /// services in the same local scope as itself, and any further child
    /// services, without affecting the rest of the global scope.
    pub fn child(&self) -> Self {
        let token_local = self.token_local.as_ref().unwrap_or(&self.token_global).child_token();

        Self { token_local: Some(token_local), ..Clone::clone(self) }
    }

    /// Atomically checks if a set of services are running.
    ///
    /// You can combine multiple [MadaraService] into a single bitmask to
    /// check the state of multiple services at once.
    ///
    /// This will return [MadaraServiceStatus::On] if _any_ of the services in
    /// the bitmask are active.
    #[inline(always)]
    pub fn service_status(&self, svc: u8) -> MadaraServiceStatus {
        self.services.status(svc)
    }

    /// Atomically marks a service as active
    ///
    /// This will immediately be visible to all services in the same global
    /// scope. This is true across threads.
    #[inline(always)]
    pub fn service_add(&self, svc: MadaraService) -> MadaraServiceStatus {
        let res = self.services.activate(svc);

        // TODO: make an internal server error out of this
        let _ = self.service_update_sender.send(ServiceTransport { svc, status: MadaraServiceStatus::On });

        res
    }

    /// Atomically marks a service as inactive
    ///
    /// This will immediately be visible to all services in the same global
    /// scope. This is true across threads.
    #[inline(always)]
    pub fn service_remove(&self, svc: MadaraService) -> MadaraServiceStatus {
        let res = self.services.deactivate(svc);
        let _ = self.service_update_sender.send(ServiceTransport { svc, status: MadaraServiceStatus::Off });

        res
    }

    pub async fn service_subscribe(&mut self) -> Option<ServiceTransport> {
        if self.service_update_receiver.is_none() {
            self.service_update_receiver = Some(self.service_update_sender.subscribe());
        }

        let mut rx = self.service_update_receiver.take().expect("Receiver was set above");
        let token_global = &self.token_global;
        let token_local = self.token_local.as_ref().unwrap_or(&self.token_global);

        let res = tokio::select! {
            svc = rx.recv() => svc.ok(),
            _ = token_global.cancelled() => None,
            _ = token_local.cancelled() => None
        };

        self.service_update_receiver = Some(rx);
        res
    }

    /// Atomically checks if the service associated to this [ServiceContext] is
    /// active.
    ///
    /// This can be updated across threads by calling [ServiceContext::service_remove]
    /// or [ServiceContext::service_add]
    #[inline(always)]
    pub fn status(&self) -> MadaraServiceStatus {
        self.services.status(self.id as u8)
    }

    /// Atomically checks the state of the node
    #[inline(always)]
    pub fn state(&self) -> MadaraState {
        self.state.load(std::sync::atomic::Ordering::SeqCst).into()
    }

    /// Atomically sets the state of the node
    ///
    /// This will immediately be visible to all services in the same global
    /// scope. This is true across threads.
    pub fn state_advance(&mut self) -> MadaraState {
        let state = self.state.load(std::sync::atomic::Ordering::SeqCst).saturating_add(1);
        self.state.store(state, std::sync::atomic::Ordering::SeqCst);
        state.into()
    }
}

/// The app is divided into services, with each service having a different responsability within the app.
/// Depending on the startup configuration, some services are enabled and some are disabled.
///
/// This trait enables launching nested services and groups.
#[async_trait::async_trait]
pub trait Service: 'static + Send + Sync {
    /// Default impl does not start any task.
    async fn start<'a>(&mut self, _runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        Ok(())
    }

    fn id(&self) -> MadaraService;
}

#[async_trait::async_trait]
impl Service for Box<dyn Service> {
    async fn start<'a>(&mut self, _runner: ServiceRunner<'a>) -> anyhow::Result<()> {
        self.as_mut().start(_runner).await
    }

    fn id(&self) -> MadaraService {
        self.as_ref().id()
    }
}

pub struct ServiceRunner<'a> {
    ctx: ServiceContext,
    join_set: &'a mut JoinSet<anyhow::Result<MadaraService>>,
}

impl<'a> ServiceRunner<'a> {
    fn new(ctx: ServiceContext, join_set: &'a mut JoinSet<anyhow::Result<MadaraService>>) -> Self {
        Self { ctx, join_set }
    }

    pub fn ctx(&self) -> &ServiceContext {
        &self.ctx
    }

    pub fn service_loop<F, E>(self, runner: impl FnOnce(ServiceContext) -> F + Send + 'static)
    where
        F: Future<Output = Result<(), E>> + Send + 'static,
        E: Into<anyhow::Error> + Send,
    {
        let Self { ctx, join_set } = self;
        join_set.spawn(async move {
            let id = ctx.id();
            if id != MadaraService::Monitor {
                tracing::debug!("Starting {id}");
            }

            // If a service is implemented correctly, `stopper` should never
            // cancel first. This is a safety measure in case someone forgets to
            // implement a cancellation check along some branch of the service's
            // execution, or if they don't read the docs :D
            let ctx1 = ctx.clone();
            tokio::select! {
                res = runner(ctx) => res.map_err(Into::into)?,
                _ = Self::stopper(ctx1, &id) => {},
            }

            if id != MadaraService::Monitor {
                tracing::debug!("Shutting down {id}");
            }

            Ok(id)
        });
    }

    async fn stopper(mut ctx: ServiceContext, id: &MadaraService) {
        ctx.cancelled().await;
        tokio::time::sleep(SERVICE_GRACE_PERIOD).await;

        tracing::info!("forcefully shutting down {id}");
    }
}

pub struct ServiceMonitor {
    services: [Option<Box<dyn Service>>; SERVICE_COUNT],
    join_set: JoinSet<anyhow::Result<MadaraService>>,
    status_request: Arc<MadaraServiceMask>,
    status_actual: Arc<MadaraServiceMask>,
}

impl Default for ServiceMonitor {
    fn default() -> Self {
        Self {
            services: Default::default(),
            join_set: Default::default(),
            status_request: Default::default(),
            status_actual: Arc::new(MadaraServiceMask(std::sync::atomic::AtomicU8::new(u8::MAX))),
        }
    }
}

impl ServiceMonitor {
    pub fn with(mut self, svc: impl Service) -> anyhow::Result<Self> {
        let idx = (svc.id() as u8).to_be().leading_zeros() as usize;
        self.services[idx] = match self.services[idx] {
            Some(_) => anyhow::bail!("Services has already been added"),
            None => Some(Box::new(svc)),
        };

        anyhow::Ok(self)
    }

    pub fn activate(&self, id: MadaraService) {
        self.status_request.activate(id);
    }

    pub async fn start(mut self) -> anyhow::Result<()> {
        let mut ctx = ServiceContext::new_with_services(Arc::clone(&self.status_request));

        for svc in self.services.iter_mut() {
            match svc {
                Some(svc) if self.status_request.status(svc.id() as u8) == MadaraServiceStatus::On => {
                    let id = svc.id();
                    self.status_actual.activate(id);
                    self.status_request.activate(id);

                    let ctx = ctx.child().with_id(id);
                    let runner = ServiceRunner::new(ctx, &mut self.join_set);
                    svc.start(runner).await.context("Starting service")?;
                }
                _ => continue,
            }
        }

        let runner = ServiceRunner::new(ctx.clone(), &mut self.join_set);
        runner.service_loop(|ctx| async move {
            tokio::signal::ctrl_c().await.expect("Failed to listen for event");
            ctx.cancel_global();

            anyhow::Ok(())
        });

        while self.status_request.is_active_some() {
            tokio::select! {
                Some(result) = self.join_set.join_next() => {
                    match result {
                        Ok(result) => {
                            let id = result?;
                            tracing::debug!("service {id} has shut down");
                            self.status_actual.deactivate(id);
                            self.status_request.deactivate(id);
                        }
                        Err(panic_error) if panic_error.is_panic() => {
                            // bubble up panics too
                            panic::resume_unwind(panic_error.into_panic());
                        }
                        Err(_task_cancelled_error) => {}
                    }
                },
                Some(ServiceTransport { svc, status }) = ctx.service_subscribe() => {
                    if status == MadaraServiceStatus::On {
                        if let Some(svc) = self.services[svc as usize].as_mut() {
                            let id = svc.id();
                            if self.status_actual.status(id as u8) == MadaraServiceStatus::Off {
                                self.status_actual.activate(id);
                                self.status_request.activate(id);

                                let ctx = ctx.child().with_id(id);
                                let runner = ServiceRunner::new(ctx, &mut self.join_set);
                                svc.start(runner)
                                    .await
                                    .context("Starting service")?;
                            }
                        }
                    }
                },
                else => continue
            };

            tracing::debug!("Services still active: {:?}", self.status_request.active_set());
        }

        Ok(())
    }
}
