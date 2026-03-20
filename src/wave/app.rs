use crossterm::event::{KeyCode, KeyEventKind};
use ratatui::symbols::Marker;
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Stylize},
    text::Line,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType},
};
use std::sync::mpsc;

pub enum Event {
    Input(crossterm::event::KeyEvent),
    Audio(Vec<f32>),
}

#[derive(Debug)]
pub struct WaveApp {
    exit: bool,
    db: rusqlite::Connection,
    data: Vec<f32>,
}

impl WaveApp {
    pub fn new(path: &str) -> Result<WaveApp, anyhow::Error> {
        let db = rusqlite::Connection::open(path)?;
        Ok(Self {
            exit: false,
            db: db,
            data: vec![],
        })
    }

    fn draw(&self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(frame.area());

        let instructions = Line::from(vec![" Quit ".into(), "<Q> ".blue().bold()]).centered();
        let x_axis = Axis::default()
            .title("time".yellow())
            .bounds([0.0, 10.0])
            .labels(["0%", "50%", "100%"]);

        let y_axis = Axis::default()
            .title("Amplitude".yellow())
            .bounds([0.0, 1.0])
            .labels(["0", "5", "10"]);

        let data = self
            .data
            .iter()
            .enumerate()
            .map(|(i, &y)| (i as f64, y as f64))
            .collect::<Vec<_>>();

        let dataset = Dataset::default()
            .name("Amplitude")
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Color::Blue)
            .data(&data);

        let time = Chart::new(vec![dataset])
            .x_axis(x_axis)
            .y_axis(y_axis)
            .block(Block::default().title("Waveform").borders(Borders::ALL));

        let x_axis = Axis::default()
            .title("Frequency (Hz)".yellow())
            .bounds([0.0, 10.0])
            .labels(["0%", "50%", "100%"]);

        let y_axis = Axis::default()
            .title("Amplitude (dB)".yellow())
            .bounds([0.0, 10.0])
            .labels(["0", "5", "10"]);

        let freq = Chart::new(vec![Dataset::default()])
            .x_axis(x_axis)
            .y_axis(y_axis)
            .block(
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
                Event::Audio(data) => self.handle_audio_event(data)?,
            }

            terminal.draw(|frame| self.draw(frame))?;
        }

        Ok(())
    }

    fn handle_audio_event(&mut self, data: Vec<f32>) -> Result<(), anyhow::Error> {
        self.data = data;
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
