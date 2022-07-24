mod broadcaster;
use std::sync::Mutex;

use broadcaster::Broadcaster;
use futures::{future, StreamExt};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};
use tokio_process_stream::ProcessLineStream;

use actix_files::Files;
use actix_web::{
    web::{self, Data},
    App, HttpResponse, HttpServer, Responder,
};

use clap::Parser;

async fn sse(broadcaster: Data<Mutex<Broadcaster>>) -> impl Responder {
    let rx = broadcaster.lock().unwrap().new_client();

    let mut res = HttpResponse::Ok()
        .append_header(("content-type", "text/event-stream"))
        .no_chunking(0)
        .streaming(rx);

    res.headers_mut()
        .remove(actix_web::http::header::CONTENT_LENGTH);
    res
}

async fn execute_command(
    broadcaster: Data<Mutex<Broadcaster>>,
    cmd: Command,
) -> std::io::Result<()> {
    let mut stream = ProcessLineStream::try_from(cmd).unwrap();

    while let Some(item) = stream.next().await {
        broadcaster.lock().unwrap().send(&item.to_string());
    }
    Ok(())
}

async fn pipe_stdin(broadcaster: Data<Mutex<Broadcaster>>) -> std::io::Result<()> {
    let stdin = tokio::io::stdin();

    let stdin = BufReader::new(stdin);
    let mut lines = stdin.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        broadcaster.lock().unwrap().send(&line);
    }
    Ok(())
}

#[derive(clap::ValueEnum, Clone, Debug)]
enum HttpMode {
    Ws,
    Sse,
}

/// A daemon for exposing output in websockets and sse
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Host address to bind
    #[clap(short, long)]
    host: String,

    /// Port to bind
    #[clap(short, long, default_value_t = 9000)]
    port: u32,

    /// Url path to setup
    #[clap(long, default_value = "/events")]
    path: String,

    /// Folder path to serve static files
    #[clap(long)]
    serve: Option<String>,

    /// Mode to expose events
    #[clap(value_enum, short, long)]
    mode: HttpMode,

    /// Command to run, If empty stdin is used
    #[clap(long, short = 'c')]
    command: Option<String>,
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let args = Args::parse();
    let _mode = args.mode.clone();
    let broadcaster = Broadcaster::create();
    let b = broadcaster.clone();
    let runner = async move {
        match args.command {
            Some(cmd) => {
                let cmd = Command::new(cmd);
                execute_command(b.clone(), cmd).await
            }
            None => pipe_stdin(b.clone()).await,
        }
    };

    let http = HttpServer::new(move || {
        App::new()
            .app_data(broadcaster.clone())
            .service(web::resource("/events").route(web::to(sse)))
            .service(Files::new("/", "./public").index_file("index.html"))
    })
    .bind(("127.0.0.1", 8080))?
    .run();

    future::try_join(http, runner).await?;
    Ok(())
}
