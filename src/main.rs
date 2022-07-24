mod broadcaster;
mod command;
use std::sync::Mutex;

use broadcaster::Broadcaster;

use command::{execute_command, pipe_stdin, Spawn};
use futures::{future, StreamExt};

use actix_files::Files;
use actix_web::{
    web::{self, Data},
    App, HttpRequest, HttpResponse, HttpServer, Responder,
};

use clap::Parser;

#[derive(clap::ValueEnum, Clone, Debug)]
pub enum HttpMode {
    Ws,
    Sse,
}

/// A daemon for exposing output in websockets and sse
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

async fn sse(broadcaster: Data<Mutex<Broadcaster>>) -> impl Responder {
    let rx = broadcaster.lock().unwrap().new_sse_client();

    let mut res = HttpResponse::Ok()
        .append_header(("content-type", "text/event-stream"))
        .no_chunking(0)
        .streaming(rx);

    res.headers_mut()
        .remove(actix_web::http::header::CONTENT_LENGTH);
    res
}

use actix_ws::Message;
async fn ws(
    req: HttpRequest,
    body: web::Payload,
    broadcaster: Data<Mutex<Broadcaster>>,
) -> impl Responder {
    let (response, mut session, mut msg_stream) = actix_ws::handle(&req, body).unwrap();
    broadcaster.lock().unwrap().add_ws_client(session.clone());

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

    println!("{:?}", args);
    let command = &args.command;
    let runner = async {
        match command {
            Some(spawn) => execute_command(broadcaster.clone(), spawn.clone()).await,
            None => pipe_stdin(broadcaster.clone()).await,
        }
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

    future::try_join(http, runner).await?;
    Ok(())
}
