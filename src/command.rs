use std::sync::Mutex;

use actix_web::web::Data;
use futures::StreamExt;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};
use tokio_process_stream::ProcessLineStream;

use crate::broadcaster::Broadcaster;

#[derive(clap::Subcommand, Debug, Clone)]
pub enum Spawn {
    #[clap(external_subcommand)]
    Start(Vec<String>),
}

pub(crate) async fn execute_command(
    broadcaster: Data<Mutex<Broadcaster>>,
    spawn: Spawn,
) -> std::io::Result<()> {
    let (first, args) = match spawn {
        Spawn::Start(mut args) => {
            let first = args.remove(0);

            (first, args)
        }
    };

    let mut stream = ProcessLineStream::try_from(Command::new(first).args(args.clone()))?;
    // TODO: Allow websocket input to stdin
    while let Some(item) = stream.next().await {
        broadcaster.lock().unwrap().send(&item.to_string()).await;
    }
    Ok(())
}

pub(crate) async fn pipe_stdin(broadcaster: Data<Mutex<Broadcaster>>) -> std::io::Result<()> {
    let stdin = tokio::io::stdin();

    let stdin = BufReader::new(stdin);
    let mut lines = stdin.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        broadcaster.lock().unwrap().send(&line).await;
    }
    Ok(())
}
