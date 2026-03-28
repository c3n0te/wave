use crate::wave::app::{Event, WaveApp};
use crate::wave::log::initialize_logging;
use crate::wave::utils::{handle_input, stream_audio};
use anyhow::anyhow;
use std::sync::mpsc;
use std::thread;
mod wave;

fn main() -> Result<(), anyhow::Error> {
    initialize_logging()?;
    let mut wave = WaveApp::new("wave.db", 16000.0)?;
    let mut terminal = ratatui::init();
    let (event_tx, event_rx) = mpsc::channel::<Event>();
    let tx_clone = event_tx.clone();
    tracing::info!("Starting wavetui");

    thread::spawn(move || {
        let Ok(_) = handle_input(tx_clone) else {
            return Err(anyhow!("Failed to read keyboard input event"));
        };

        Ok(())
    });

    thread::spawn(move || {
        let Ok(_) = stream_audio(event_tx, 15) else {
            return Err(anyhow!("Failed to read audio input event"));
        };

        Ok(())
    });

    wave.run(&mut terminal, event_rx)?;
    ratatui::restore();
    Ok(())
}
