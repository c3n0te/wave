use crossterm::event::{KeyCode, KeyEventKind};
use ratatui::symbols::Marker;
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Stylize},
    text::Line,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType},
};
use rustfft::{FftPlanner, num_complex::Complex};
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
        let time_data = self.time_series();
        let x_axis = Axis::default()
            .title("time".yellow())
            .bounds([0.0, time_data.len() as f64])
            .labels(["0%", "50%", "100%"]);

        let y_axis = Axis::default()
            .title("Amplitude".yellow())
            .bounds([-0.5, 0.5])
            .labels(["-0.5", "0.0", "0.5"]);

        let time_dataset = Dataset::default()
            .name("Amplitude")
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Color::Green)
            .data(&time_data);

        let time = Chart::new(vec![time_dataset])
            .x_axis(x_axis)
            .y_axis(y_axis)
            .block(Block::default().title("Waveform").borders(Borders::ALL));

        let freq_data = self.fft_series();
        let x_axis = Axis::default()
            .title("Frequency (Hz)".yellow())
            .bounds([0.0, freq_data.len() as f64])
            .labels(["0%", "50%", "100%"]);

        let y_axis = Axis::default()
            .title("Magnitude".yellow())
            .bounds([-5.0, 5.0])
            .labels(["-5.0", "0", "5.0"]);

        let freq_dataset = Dataset::default()
            .name("Amplitude")
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Color::Green)
            .data(&freq_data);

        let freq = Chart::new(vec![freq_dataset])
            .x_axis(x_axis)
            .y_axis(y_axis)
            .block(
                Block::default()
                    .title("FFT Spectrum")
                    .title_bottom(instructions)
                    .borders(Borders::ALL),
            );

        frame.render_widget(time, chunks[0]);
        frame.render_widget(freq, chunks[1]);
    }

    fn time_series(&self) -> Vec<(f64, f64)> {
        self.data
            .iter()
            .enumerate()
            .map(|(i, &y)| (i as f64, y as f64))
            .collect::<Vec<_>>()
    }

    fn fft_series(&self) -> Vec<(f64, f64)> {
        let samples = self.data.len();
        let mut buffer: Vec<Complex<f32>> = self
            .data
            .iter()
            .map(|&x| Complex::new(x as f32, 0.0))
            .collect();

        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(samples);
        fft.process(&mut buffer);
        buffer
            .iter()
            .enumerate()
            .map(|(i, &y)| (i as f64, y.re as f64))
            .collect()
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
