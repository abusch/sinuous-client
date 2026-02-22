use tokio::sync::mpsc::error::SendError;
use tokio_tungstenite::tungstenite;

use crate::model::SonosObject;

pub mod conn;
pub mod model;
pub mod sonos;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Rustls(#[from] rustls::Error),
    #[error(transparent)]
    Websocket(#[from] tungstenite::Error),
    #[error("Unexpected object type")]
    UnexpectedObjectType,
    #[error("Connection is closed")]
    ConnectionClosed,
    #[error("Sonos command returned an error")]
    ApiResponse(Box<SonosObject>),
    #[error("Invalid response: {0}")]
    InvalidResponse(String),
    #[error("Error sending websocket message")]
    Send(#[from] SendError<tungstenite::Message>),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}
