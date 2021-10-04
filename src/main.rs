#[macro_use]
extern crate smart_default;

#[macro_use]
mod macros;

mod event;
mod greeter;
mod info;
mod ipc;
mod keyboard;
mod power;
mod ui;

use std::{error::Error, io, process, sync::Arc};

use greetd_ipc::Request;
use termion::raw::IntoRawMode;
use tokio::sync::RwLock;
use tui::{backend::TermionBackend, Terminal};

pub use self::greeter::*;
use self::{event::Events, ipc::Ipc};

#[tokio::main]
async fn main() {
  if let Err(error) = run().await {
    if let Some(AuthStatus::Success) = error.downcast_ref::<AuthStatus>() {
      return;
    }

    process::exit(1);
  }
}

async fn run() -> Result<(), Box<dyn Error>> {
  let greeter = Greeter::new().await;

  let stdout = io::stdout().into_raw_mode()?;
  let backend = TermionBackend::new(stdout);
  let mut terminal = Terminal::new(backend)?;

  terminal.clear()?;

  let mut events = Events::new().await;
  let ipc = Ipc::new();

  if greeter.remember && !greeter.username.is_empty() {
    ipc.send(Request::CreateSession { username: greeter.username.clone() }).await;
  }

  let greeter = Arc::new(RwLock::new(greeter));

  tokio::task::spawn({
    let greeter = greeter.clone();
    let mut ipc = ipc.clone();

    async move {
      loop {
        let _ = ipc.handle(greeter.clone()).await;
      }
    }
  });

  tokio::task::spawn({
    let greeter = greeter.clone();

    async move {
      loop {
        let command = greeter.write().await.power_command.take();

        if let Some(mut command) = command {
          greeter.write().await.mode = Mode::Processing;

          let message = match tokio::spawn(async move { command.status().await }).await {
            Ok(result) => match result {
              Ok(status) if status.success() => None,
              Ok(status) => Some(format!("Command exited with {}", status)),
              Err(err) => Some(format!("Command failed: {}", err)),
            },

            Err(_) => Some("Command failed".to_string()),
          };

          let mode = greeter.read().await.previous_mode;

          let mut greeter = greeter.write().await;

          greeter.mode = mode;
          greeter.message = message;
        }
      }
    }
  });

  loop {
    if let Some(ref mut rx) = greeter.write().await.exit_rx {
      if let Ok(status) = rx.try_recv() {
        return Err(status.into());
      }
    }

    ui::draw(greeter.clone(), &mut terminal).await?;
    keyboard::handle(greeter.clone(), &mut events, ipc.clone()).await?;
  }
}

pub async fn exit(mut greeter: &mut Greeter, status: AuthStatus) {
  match status {
    AuthStatus::Success => {}
    AuthStatus::Cancel | AuthStatus::Failure => Ipc::cancel(&mut greeter).await,
  }

  clear_screen();

  if let Some(tx) = greeter.exit_tx.take() {
    let _ = tx.send(status);
  }
}

pub fn clear_screen() {
  let backend = TermionBackend::new(io::stdout());

  if let Ok(mut terminal) = Terminal::new(backend) {
    let _ = terminal.clear();
  }
}

#[cfg(debug_assertions)]
pub fn log(msg: &str) {
  use std::io::Write;

  let mut file = std::fs::OpenOptions::new().create(true).append(true).open("/tmp/tuigreet.log").unwrap();
  file.write_all(msg.as_ref()).unwrap();
  file.write_all("\n".as_bytes()).unwrap();
}
