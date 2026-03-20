use crate::Event;
use std::sync::mpsc;

pub fn handle_input(tx: mpsc::Sender<Event>) -> Result<(), anyhow::Error> {
    loop {
        match crossterm::event::read()? {
            crossterm::event::Event::Key(key_event) => tx.send(Event::Input(key_event))?,
            _ => {}
        }
    }
}
