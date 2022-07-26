use actix_web::web::{Bytes, Data};
use actix_web::Error;

use actix_ws::{Closed, Session};
use futures::Stream;
use std::pin::Pin;
use tokio::sync::mpsc::error::SendError;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::Mutex;

use std::task::{Context, Poll};

use crate::HttpMode;

/// Generic sender for new lines
#[derive(Clone)]
pub enum LineSender {
    Sse(Sender<Bytes>),
    Ws(Session),
}

/// Errors encountered in broadcasting lines
pub enum LineSendError {
    Sse(SendError<Bytes>),
    Ws(Closed),
}

impl LineSender {
    /// Try send some text to sender
    async fn try_send(&self, text: String) -> Result<(), LineSendError> {
        match &self {
            LineSender::Sse(sender) => sender
                .send(Bytes::from(text))
                .await
                .map_err(LineSendError::Sse),
            LineSender::Ws(session) => {
                let mut session = session.clone();
                session.text(text).await.map_err(LineSendError::Ws)
            }
        }
    }
}

/// Handles broadcasting for all modes
pub struct Broadcaster {
    clients: Vec<LineSender>,
    mode: HttpMode,
}

impl Broadcaster {
    /// Build [Data] ready instance of broadcaster
    pub fn create(mode: HttpMode) -> Data<Mutex<Self>> {
        Data::new(Mutex::new(Broadcaster::new(mode)))
    }

    /// Get the length of cliets
    pub fn clients_len(&self) -> usize {
        self.clients.len()
    }

    fn new(mode: HttpMode) -> Self {
        Broadcaster {
            clients: Vec::new(),
            mode,
        }
    }
    /// Remove clients that have not responded to a ping
    pub async fn remove_stale_clients(&mut self) -> bool {
        let mut ok_clients: Vec<LineSender> = Vec::new();
        for client in self.clients.iter() {
            let result = match &client {
                LineSender::Sse(client) => client
                    .clone()
                    .send(Bytes::from("data: ping\n\n"))
                    .await
                    .map_err(LineSendError::Sse),
                LineSender::Ws(session) => {
                    let mut session = session.clone();
                    session.ping(b"").await.map_err(LineSendError::Ws)
                }
            };

            if let Ok(()) = result {
                ok_clients.push(client.clone());
            }
        }
        self.clients = ok_clients;
        true
    }

    /// Create a new Sse [Client]
    pub fn new_sse_client(&mut self) -> Client {
        let (tx, rx) = channel(100);

        tx.try_send(Bytes::from("data: connected\n\n")).unwrap();

        self.clients.push(LineSender::Sse(tx));
        Client(rx)
    }

    /// Add Websocket client
    pub fn add_ws_client(&mut self, session: Session) {
        self.clients.push(LineSender::Ws(session));
    }

    /// Send some text to all members
    pub async fn send(&self, msg: &str) {
        let msg = match self.mode {
            HttpMode::Ws => msg.to_string(),
            HttpMode::Sse => ["data: ", msg, "\n\n"].concat(),
        };
        log::trace!("Sending to all: {:?}", msg);

        for client in self.clients.iter() {
            client.clone().try_send(msg.clone()).await.unwrap_or(());
        }
    }
}

// wrap Receiver in own type
pub struct Client(Receiver<Bytes>);

impl Stream for Client {
    type Item = Result<Bytes, Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.0).poll_recv(cx) {
            Poll::Ready(Some(v)) => Poll::Ready(Some(Ok(v))),
            Poll::Ready(None) => Poll::Ready(None),
            Poll::Pending => Poll::Pending,
        }
    }
}
