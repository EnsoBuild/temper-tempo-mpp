//! Error handling for HTTP responses

use serde::{Deserialize, Serialize};
use std::{convert::Infallible, error::Error};
use temper_core::simulation::SimulationError;
use warp::{body::BodyDeserializeError, hyper::StatusCode, reject::Reject, Rejection, Reply};

#[derive(Serialize, Deserialize, Debug)]
pub struct ErrorMessage {
    pub code: u16,
    pub message: String,
}

#[derive(Debug)]
pub struct NoURLForChainIdError;

impl Reject for NoURLForChainIdError {}

#[derive(Debug)]
pub struct IncorrectChainIdError();

impl Reject for IncorrectChainIdError {}

#[derive(Debug)]
pub struct MultipleChainIdsError();

impl Reject for MultipleChainIdsError {}

#[derive(Debug)]
pub struct MultipleBlockNumbersError();

impl Reject for MultipleBlockNumbersError {}

#[derive(Debug)]
pub struct InvalidBlockNumbersError();

impl Reject for InvalidBlockNumbersError {}

#[derive(Debug)]
pub struct StateNotFound();

impl Reject for StateNotFound {}

#[derive(Debug)]
pub struct OverrideError;

impl Reject for OverrideError {}

#[derive(Debug)]
pub struct EvmError(pub eyre::Report);

impl Reject for EvmError {}

#[derive(Debug)]
pub struct SimulationNotFound;

impl Reject for SimulationNotFound {}

#[derive(Debug)]
pub struct DatabaseError(pub String);

impl Reject for DatabaseError {}

#[derive(Debug)]
pub struct TraceFetchError(pub String);

impl Reject for TraceFetchError {}

/// Convert SimulationError to warp Rejection
pub fn simulation_error_to_rejection(err: SimulationError) -> Rejection {
    match err {
        SimulationError::NoRpcUrlForChain(_) => warp::reject::custom(NoURLForChainIdError),
        SimulationError::IncorrectChainId { .. } => {
            warp::reject::custom(IncorrectChainIdError())
        }
        SimulationError::MultipleChainIds => warp::reject::custom(MultipleChainIdsError()),
        SimulationError::InvalidBlockNumbers => {
            warp::reject::custom(InvalidBlockNumbersError())
        }
        SimulationError::OverrideError => warp::reject::custom(OverrideError),
        SimulationError::EvmError(e) => warp::reject::custom(EvmError(e)),
        SimulationError::MissingToAddress => warp::reject::custom(IncorrectChainIdError()),
        SimulationError::StateNotFound => warp::reject::custom(StateNotFound()),
    }
}

pub async fn handle_rejection(err: Rejection) -> Result<impl Reply, Infallible> {
    let code;
    let message: String;

    log::debug!("Handling rejection: {:?}", err);

    // MPP 402 Payment Required — must be checked first to attach WWW-Authenticate header
    if let Some(pr) = err.find::<crate::mpp_gate::PaymentRequired>() {
        let json = warp::reply::json(&ErrorMessage {
            code: 402,
            message: "PAYMENT_REQUIRED".to_string(),
        });
        let mut resp = warp::reply::with_status(json, StatusCode::PAYMENT_REQUIRED).into_response();
        resp.headers_mut().insert("WWW-Authenticate", pr.www_authenticate.parse().unwrap());
        resp.headers_mut().insert("Cache-Control", "no-store".parse().unwrap());
        return Ok(resp);
    }

    if err.is_not_found() {
        code = StatusCode::NOT_FOUND;
        message = "NOT_FOUND".to_string();
    } else if let Some(_e) = err.find::<SimulationNotFound>() {
        code = StatusCode::NOT_FOUND;
        message = "SIMULATION_NOT_FOUND".to_string();
    } else if let Some(_e) = err.find::<StateNotFound>() {
        code = StatusCode::NOT_FOUND;
        message = "STATE_NOT_FOUND".to_string();
    } else if let Some(NoURLForChainIdError) = err.find() {
        code = StatusCode::BAD_REQUEST;
        message = "CHAIN_ID_NOT_SUPPORTED".to_string();
    } else if let Some(_e) = err.find::<IncorrectChainIdError>() {
        code = StatusCode::BAD_REQUEST;
        message = "INCORRECT_CHAIN_ID".to_string();
    } else if let Some(_e) = err.find::<MultipleChainIdsError>() {
        code = StatusCode::BAD_REQUEST;
        message = "MULTIPLE_CHAIN_IDS".to_string();
    } else if let Some(_e) = err.find::<MultipleBlockNumbersError>() {
        code = StatusCode::BAD_REQUEST;
        message = "MULTIPLE_BLOCK_NUMBERS".to_string();
    } else if let Some(_e) = err.find::<InvalidBlockNumbersError>() {
        code = StatusCode::BAD_REQUEST;
        message = "INVALID_BLOCK_NUMBERS".to_string();
    } else if let Some(_e) = err.find::<OverrideError>() {
        code = StatusCode::INTERNAL_SERVER_ERROR;
        message = "OVERRIDE_ERROR".to_string();
    } else if let Some(_e) = err.find::<EvmError>() {
        let full_error = format!("{:#}", _e.0);
        if full_error.contains("CallGasCostMoreThanGasLimit")
            || full_error.contains("call gas cost")
            || full_error.contains("exceeds the gas limit")
        {
            code = StatusCode::BAD_REQUEST;
            message = "OUT_OF_GAS".to_string();
        } else {
            code = StatusCode::INTERNAL_SERVER_ERROR;
            message = "EVM_ERROR".to_string();
        }
    } else if let Some(_e) = err.find::<DatabaseError>() {
        code = StatusCode::INTERNAL_SERVER_ERROR;
        message = "DATABASE_ERROR".to_string();
    } else if let Some(e) = err.find::<TraceFetchError>() {
        code = StatusCode::BAD_REQUEST;
        message = format!("TRACE_FETCH_ERROR: {}", e.0);
    } else if let Some(e) = err.find::<BodyDeserializeError>() {
        message = match e.source() {
            Some(cause) => format!("BAD REQUEST: {cause}"),
            None => "BAD_REQUEST".to_string(),
        };
        code = StatusCode::BAD_REQUEST;
    } else if err.find::<warp::reject::MethodNotAllowed>().is_some() {
        code = StatusCode::METHOD_NOT_ALLOWED;
        message = "METHOD_NOT_ALLOWED".to_string();
    } else if err.find::<warp::reject::MissingHeader>().is_some() {
        code = StatusCode::UNAUTHORIZED;
        message = "UNAUTHORIZED".to_string();
    } else {
        log::error!("unhandled rejection: {err:?}");
        code = StatusCode::INTERNAL_SERVER_ERROR;
        message = "UNHANDLED_REJECTION".to_string();
    }

    let json = warp::reply::json(&ErrorMessage {
        code: code.as_u16(),
        message,
    });

    Ok(warp::reply::with_status(json, code).into_response())
}
