mod broadcaster;
mod command;
use std::time::Duration;

use tokio::sync::Mutex;

use broadcaster::Broadcaster;

use command::{execute_command, pipe_stdin, Spawn};
use futures::{future, StreamExt};

use actix_files::Files;
use actix_web::{
    http::{self, header::ContentEncoding},
    web::{self, Data},
    App, HttpRequest, HttpResponse, HttpServer, Responder,
};

use clap::Parser;

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum HttpMode {
    Ws,
    Sse,
}

/// A command-line tool for exposing a wrapped cli program's standard IO to WebSockets/SSE
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Args {
    /// Host address to bind
    #[clap(short, long, default_value = "127.0.0.1")]
    host: String,

    /// Port to bind
    #[clap(short, long, default_value_t = 9000)]
    port: u16,

    /// Url path to setup
    #[clap(long, default_value = "/events")]
    path: String,

    /// Optional folder path to serve static files
    #[clap(long)]
    serve: Option<String>,

    /// Mode to expose events
    #[clap(value_enum, short, long)]
    mode: HttpMode,

    /// Command to run, If empty stdin is used
    #[clap(subcommand)]
    command: Option<Spawn>,
}

/// SSE handler
async fn sse(broadcaster: Data<Mutex<Broadcaster>>) -> impl Responder {
    let rx = broadcaster.lock().await.new_sse_client();

    HttpResponse::Ok()
        .insert_header((http::header::CONTENT_TYPE, "text/event-stream"))
        .insert_header(ContentEncoding::Identity)
        .streaming(rx)
}

/// Websocket handler
async fn ws(
    req: HttpRequest,
    body: web::Payload,
    broadcaster: Data<Mutex<Broadcaster>>,
) -> impl Responder {
    use actix_ws::Message;
    let (response, mut session, mut msg_stream) = actix_ws::handle(&req, body).unwrap();
    broadcaster.lock().await.add_ws_client(session.clone());

    actix_web::rt::spawn(async move {
        while let Some(Ok(msg)) = msg_stream.next().await {
            match msg {
                Message::Ping(bytes) => {
                    if session.pong(&bytes).await.is_err() {
                        return;
                    }
                }
                Message::Text(s) => {
                    log::trace!("Got text, {}", s);
                    // let mut stdout = io::stdout().lock();
                    // stdout.write_all(s)?;
                }
                _ => break,
            }
        }

        let _ = session.close(None).await;
    });

    response
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    env_logger::init();
    let args = Args::parse();
    let mode = args.mode.clone();
    let broadcaster = Broadcaster::create(mode);
    let command = &args.command;
    let command_runner = async {
        match command {
            Some(spawn) => execute_command(broadcaster.clone(), spawn.clone()).await,
            None => pipe_stdin(broadcaster.clone()).await,
        }
    };
    let ping_clients = async {
        let mut wait = tokio::time::interval(Duration::from_secs(10));
        loop {
            wait.tick().await;
            let mut b = broadcaster.lock().await;
            log::trace!("Broadcaster has {} listeners", b.clients_len());
            if b.remove_stale_clients().await {
                break;
            }
        }
        Ok(())
    };

    let broadcaster = broadcaster.clone();
    let path = args.path.clone();
    let serve = args.serve.clone();
    let mode = args.mode.clone();
    let http = HttpServer::new(move || {
        let mut app = App::new().app_data(broadcaster.clone());
        match mode {
            HttpMode::Ws => {
                app = app.service(web::resource(path.clone()).route(web::to(ws)));
            }
            HttpMode::Sse => {
                app = app.service(web::resource(path.clone()).route(web::to(sse)));
            }
        };
        if serve.is_some() {
            app = app.service(Files::new("/", serve.as_ref().unwrap()).index_file("index.html"));
        }
        app
    })
    .bind((args.host, args.port))?
    .run();
    let runner = future::try_join(command_runner, ping_clients);
    future::try_join(http, runner).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use actix_web::{
        body::MessageBody as _,
        rt::pin,
        test,
        web::{self, Bytes},
        App,
    };
    use actix_web_actors::ws;
    use futures::future;

    #[actix_web::test]
    async fn sse_payload() {
        let broadcaster = Broadcaster::create(HttpMode::Sse);

        let app = test::init_service(
            App::new()
                .app_data(broadcaster.clone())
                .route("/", web::get().to(sse)),
        )
        .await;
        let req = test::TestRequest::get().to_request();

        let resp = test::call_service(&app, req).await;
        assert!(resp.status().is_success());

        let body = resp.into_body();
        pin!(body);

        // first chunk
        let bytes = future::poll_fn(|cx| body.as_mut().poll_next(cx)).await;
        assert_eq!(
            bytes.unwrap().unwrap(),
            web::Bytes::from_static(b"data: connected\n\n")
        );
        execute_command(
            broadcaster.clone(),
            Spawn::Start(vec!["echo".to_string(), "Hello from cmdpiped".to_string()]),
        )
        .await
        .unwrap();
        // second chunk
        let bytes = future::poll_fn(|cx| body.as_mut().poll_next(cx)).await;
        assert_eq!(
            bytes.unwrap().unwrap(),
            web::Bytes::from_static(b"data: Hello from cmdpiped\n\n")
        );
    }

    #[actix_web::test]
    async fn ws_payload() {
        let broadcaster = Broadcaster::create(HttpMode::Ws);
        let b = broadcaster.clone();
        let mut srv = actix_test::start(move || {
            App::new()
                .app_data(b.clone())
                .service(web::resource("/").to(ws))
        });

        let mut framed = srv.ws().await.unwrap();
        execute_command(
            broadcaster.clone(),
            Spawn::Start(vec!["echo".to_string(), "Hello from cmdpiped".to_string()]),
        )
        .await
        .unwrap();
        let item = framed.next().await.unwrap().unwrap();
        assert_eq!(
            item,
            ws::Frame::Text(Bytes::from_static(b"Hello from cmdpiped"))
        );
    }
}
