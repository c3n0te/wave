use crate::wave::app::{Event, WaveApp};
use crate::wave::utils::handle_input;
use anyhow::anyhow;
use std::sync::mpsc;
use std::thread;
mod wave;

fn main() -> Result<(), anyhow::Error> {
    env_logger::init();
    let mut wave = WaveApp::new("wave.db")?;
    let mut terminal = ratatui::init();
    log::info!("{:?}", wave);

    let (event_tx, event_rx) = mpsc::channel::<Event>();
    let tx_clone = event_tx.clone();

    thread::spawn(move || {
        let Ok(_) = handle_input(tx_clone) else {
            return Err(anyhow!("Failed to read input event"));
        };

        Ok(())
    });

    let _ = wave.run(&mut terminal, event_rx)?;
    ratatui::restore();
    Ok(())
}
