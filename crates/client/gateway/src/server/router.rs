use std::{convert::Infallible, sync::Arc};

use hyper::{Body, Method, Request, Response};
use mc_db::MadaraBackend;
use mp_rpc::AddTransactionProvider;

use super::handler::{
    handle_add_transaction, handle_get_block, handle_get_class_by_hash, handle_get_compiled_class_by_class_hash,
    handle_get_contract_addresses, handle_get_public_key, handle_get_signature, handle_get_state_update,
};
use super::helpers::{not_found_response, service_unavailable_response};

// Main router to redirect to the appropriate sub-router
pub(crate) async fn main_router(
    req: Request<Body>,
    backend: Arc<MadaraBackend>,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
    feeder_gateway_enable: bool,
    gateway_enable: bool,
) -> Result<Response<Body>, Infallible> {
    let path = req.uri().path().split('/').filter(|segment| !segment.is_empty()).collect::<Vec<_>>().join("/");
    match (path.as_ref(), feeder_gateway_enable, gateway_enable) {
        ("health", _, _) => Ok(Response::new(Body::from("OK"))),
        (path, true, _) if path.starts_with("feeder_gateway/") => feeder_gateway_router(req, path, backend).await,
        (path, _, true) if path.starts_with("feeder/") => gateway_router(req, path, add_transaction_provider).await,
        (path, false, _) if path.starts_with("feeder_gateway/") => Ok(service_unavailable_response("Feeder Gateway")),
        (path, _, false) if path.starts_with("feeder/") => Ok(service_unavailable_response("Feeder")),
        _ => {
            log::debug!(target: "feeder_gateway", "Main router received invalid request: {path}");
            Ok(not_found_response())
        }
    }
}

// Router for requests related to feeder_gateway
async fn feeder_gateway_router(
    req: Request<Body>,
    path: &str,
    backend: Arc<MadaraBackend>,
) -> Result<Response<Body>, Infallible> {
    match (req.method(), path) {
        (&Method::GET, "feeder_gateway/get_block") => {
            Ok(handle_get_block(req, backend).await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_signature") => {
            Ok(handle_get_signature(req, backend).await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_state_update") => {
            Ok(handle_get_state_update(req, backend).await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_class_by_hash") => {
            Ok(handle_get_class_by_hash(req, backend).await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_compiled_class_by_class_hash") => {
            Ok(handle_get_compiled_class_by_class_hash(req, backend).await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_contract_addresses") => {
            Ok(handle_get_contract_addresses(backend).await.unwrap_or_else(Into::into))
        }
        (&Method::GET, "feeder_gateway/get_public_key") => {
            Ok(handle_get_public_key(backend).await.unwrap_or_else(Into::into))
        }
        _ => {
            log::debug!(target: "feeder_gateway", "Feeder gateway received invalid request: {path}");
            Ok(not_found_response())
        }
    }
}

// Router for requests related to feeder
async fn gateway_router(
    req: Request<Body>,
    path: &str,
    add_transaction_provider: Arc<dyn AddTransactionProvider>,
) -> Result<Response<Body>, Infallible> {
    match (req.method(), req.uri().path()) {
        (&Method::POST, "feeder/add_transaction") => {
            Ok(handle_add_transaction(req, add_transaction_provider).await.unwrap_or_else(Into::into))
        }
        _ => {
            log::debug!(target: "feeder_gateway", "Gateway received invalid request: {path}");
            Ok(not_found_response())
        }
    }
}
