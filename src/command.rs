use tokio::sync::Mutex;

use actix_web::web::Data;
use futures::StreamExt;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
};
use tokio_process_stream::ProcessLineStream;

use crate::broadcaster::Broadcaster;

/// Commands that can be spawned
#[derive(clap::Subcommand, Debug, Clone)]
pub enum Spawn {
    #[clap(external_subcommand)]
    Start(Vec<String>),
}

/// Broadcast lines from spawned command
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
        broadcaster.lock().await.send(&item.to_string()).await;
    }
    Ok(())
}

/// Broadcast lines from stdin
pub(crate) async fn pipe_stdin(broadcaster: Data<Mutex<Broadcaster>>) -> std::io::Result<()> {
    let stdin = tokio::io::stdin();

    let stdin = BufReader::new(stdin);
    let mut lines = stdin.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        broadcaster.lock().await.send(&line).await;
    }
    Ok(())
}
