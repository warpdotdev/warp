use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;

use crate::protocol::Message;
use crate::{Client, ClientError};

pub(crate) type ServiceId = String;

/// Returns a unique ID for the `Service` implementation specified as a type parameter.
pub(crate) fn service_id<S: Service>() -> ServiceId {
    std::any::type_name::<S>().to_owned()
}

/// A typed IPC service interface.
///
/// Implementations should implement `ServiceImpl` and be registered on the server, while clients
/// can use `ServiceCaller` to call the service.
#[async_trait]
pub trait Service: Send + Sync + 'static {
    type Request: Message + 'static;
    type Response: Message + 'static;
}

/// To be implemented for each IPC service, where a service has a defined request/response type.
///
/// This should be implemented and is registered on the "Server" side.
///
/// Though it is technically up to users to determine whether to use a collection of `Service`s
/// or implement a single service that delegates internally, prefer the former.
#[async_trait]
pub trait ServiceImpl: 'static + Send + Sync + Clone {
    type Service: Service;

    async fn handle_request(
        &self,
        request: <<Self as ServiceImpl>::Service as Service>::Request,
    ) -> <<Self as ServiceImpl>::Service as Service>::Response;
}

/// Provides an typed interface to call an underlying `Service`.
///
/// Usage:
///
/// ```ignore
/// let client = Arc::new(
///     Client::connect(connection_address, executor)
///         .await
///         .expect("Failed to connect client."),
/// );
/// let foo = ServiceCaller::<FooService>::new(client);
/// let response = foo.call(FooRequest {}).await;
/// ```
#[async_trait]
pub trait ServiceCaller<S: Service>: Send + Sync {
    async fn call(&self, request: S::Request) -> Result<S::Response, ClientError>;
}

/// Returns a `ServiceCaller` implementation for the service `S`.
pub fn service_caller<S: Service>(client: Arc<Client>) -> Box<dyn ServiceCaller<S>> {
    Box::new(RealServiceCaller::new(client))
}

/// Real `ServiceCaller` implementation.
struct RealServiceCaller<S> {
    client: Arc<Client>,
    _service_type_marker: PhantomData<S>,
}

impl<S> RealServiceCaller<S>
where
    S: Service,
{
    fn new(client: Arc<Client>) -> Self {
        Self {
            client,
            _service_type_marker: PhantomData,
        }
    }
}

#[async_trait]
impl<S> ServiceCaller<S> for RealServiceCaller<S>
where
    S: Service,
{
    /// Sends the given request and returns a `Result` containing its response.
    async fn call(&self, request: S::Request) -> Result<S::Response, ClientError> {
        let request_bytes = bincode::serialize(&request).expect("Failed to serialize request.");
        self.client
            .send_request::<S>(request_bytes)
            .await
            .map(|response_bytes| {
                bincode::deserialize::<S::Response>(&response_bytes[..])
                    .expect("Failed to deserialize response.")
            })
    }
}
