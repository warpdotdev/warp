//! Mock implementation of `ServiceCaller` for use in tests.
//!
//! This requires service request to implement `Hash`, `PartialEq`, `Eq` to make assertions about
//! expected requests.
//!
//! If any unexpected request is called, then the caller panics. Additionally, if any expected
//! requests were _not_ actually called, the caller panics upon being dropped.
//!
//! Usage:
//!
//! ```
//! let mock_caller = MockServiceCaller::<MyService>::new();
//! mock_caller.expect_response(MyRequest { foo: "bar" }, MyResponse { bar: "baz" });
//! ```
use std::collections::HashMap;
use std::hash::Hash;

use async_trait::async_trait;
use itertools::Itertools;
use parking_lot::Mutex;

use crate::service::service_id;
use crate::{ClientError, Service, ServiceCaller};

// Use a `Mutex` so we can satisfy the immutable `&self` in the implementation of `ServiceCaller`.
type ExpectationsMap<T, U> = Mutex<HashMap<T, Result<U, ClientError>>>;

#[derive(Default)]
pub struct MockServiceCaller<S>
where
    S: Service,
    <S as Service>::Request: Hash + PartialEq + Eq,
    <S as Service>::Response: Hash + PartialEq + Eq,
{
    expectations: ExpectationsMap<S::Request, S::Response>,
}

impl<S> MockServiceCaller<S>
where
    S: Service,
    <S as Service>::Request: Hash + PartialEq + Eq,
    <S as Service>::Response: Hash + PartialEq + Eq,
{
    pub fn new() -> Self {
        Self {
            expectations: Mutex::new(HashMap::default()),
        }
    }

    pub fn expect_response(
        &mut self,
        expected_request: S::Request,
        response: Result<S::Response, ClientError>,
    ) {
        self.expectations.lock().insert(expected_request, response);
    }
}

#[async_trait]
impl<S> ServiceCaller<S> for MockServiceCaller<S>
where
    S: Service,
    <S as Service>::Request: Hash + PartialEq + Eq,
    <S as Service>::Response: Hash + PartialEq + Eq,
{
    async fn call(&self, request: S::Request) -> Result<S::Response, ClientError> {
        let response = self.expectations.lock().remove(&request);
        match response {
            Some(result) => result,
            None => {
                panic!("Unexpected IPC call with request: {:?}", &request);
            }
        }
    }
}

impl<S> Drop for MockServiceCaller<S>
where
    S: Service,
    <S as Service>::Request: Hash + PartialEq + Eq,
    <S as Service>::Response: Hash + PartialEq + Eq,
{
    fn drop(&mut self) {
        if !self.expectations.lock().is_empty() {
            panic!(
                "ServiceCaller for {} has unmet expectations: {:?}",
                service_id::<S>(),
                self.expectations.lock().drain().collect_vec()
            );
        }
    }
}
