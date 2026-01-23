use std::io::Write;
use std::{fs::File, os::unix::net::UnixStream};

use clap::Parser;

use crate::{message::IpcMessage, wayland::wayland_loop};

mod message;
mod wayland;

#[derive(Parser)]
struct Cli {
    #[arg(short, long)]
    monitor: Option<usize>,
    #[arg(short, long)]
    gamma: Option<f32>,
    #[arg(short, long)]
    daemon: bool,
}

pub const SOCKET_PATH: &str = "/tmp/hulk-gamma/fifo.sock";
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let monitor = cli.monitor.unwrap_or(0);
    let gamma = cli.gamma.unwrap_or(1.0);

    if let Ok(mut socket) = UnixStream::connect(SOCKET_PATH) {
        println!("Opening socket");
        let message = IpcMessage {
            output: Some(monitor),
            gamma: Some(gamma),
        };
        println!("Writing {message:?} to socket");
        write!(socket, "{}", ron::ser::to_string(&message)?)?;
    } else {
        wayland_loop(monitor, gamma)?;
        std::fs::remove_file(SOCKET_PATH)?;
    }
    Ok(())
}
