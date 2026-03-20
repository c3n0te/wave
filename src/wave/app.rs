use crossterm::event::{KeyCode, KeyEventKind};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Direction, Layout},
    style::Stylize,
    text::Line,
    widgets::{Axis, Block, Borders, Chart, Dataset},
};
use std::sync::mpsc;

pub enum Event {
    Input(crossterm::event::KeyEvent),
}

#[derive(Debug)]
pub struct WaveApp {
    exit: bool,
    db: rusqlite::Connection,
}

impl WaveApp {
    pub fn new(path: &str) -> Result<WaveApp, anyhow::Error> {
        let db = rusqlite::Connection::open(path)?;
        Ok(Self {
            exit: false,
            db: db,
        })
    }

    fn draw(&self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(frame.area());

        let instructions = Line::from(vec![" Quit ".into(), "<Q> ".blue().bold()]).centered();
        let x_axis = Axis::default()
            .title("time".blue())
            .bounds([0.0, 10.0])
            .labels(["0%", "50%", "100%"]);

        let y_axis = Axis::default()
            .title("Amplitude".blue())
            .bounds([0.0, 10.0])
            .labels(["0", "5", "10"]);

        let time = Chart::new(vec![Dataset::default()])
            .x_axis(x_axis)
            .y_axis(y_axis)
            .block(Block::default().title("Waveform").borders(Borders::ALL));

        let x_axis = Axis::default()
            .title("Frequency (Hz)".blue())
            .bounds([0.0, 10.0])
            .labels(["0%", "50%", "100%"]);

        let y_axis = Axis::default()
            .title("Amplitude (dB)".blue())
            .bounds([0.0, 10.0])
            .labels(["0", "5", "10"]);

        let freq = Chart::new(vec![]).x_axis(x_axis).y_axis(y_axis).block(
            Block::default()
                .title("FFT Spectrum (dB)")
                .title_bottom(instructions)
                .borders(Borders::ALL),
        );

        frame.render_widget(time, chunks[0]);
        frame.render_widget(freq, chunks[1]);
    }

    pub fn run(
        &mut self,
        terminal: &mut DefaultTerminal,
        rx: mpsc::Receiver<Event>,
    ) -> Result<(), anyhow::Error> {
        while !self.exit {
            let event = rx.recv()?;
            match event {
                Event::Input(key_event) => self.handle_key_event(key_event)?,
            }

            terminal.draw(|frame| self.draw(frame))?;
        }

        Ok(())
    }

    fn handle_key_event(
        &mut self,
        key_event: crossterm::event::KeyEvent,
    ) -> Result<(), anyhow::Error> {
        if key_event.kind == KeyEventKind::Press && key_event.code == KeyCode::Char('q') {
            self.exit = true;
        }

        Ok(())
    }
}
