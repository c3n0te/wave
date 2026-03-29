use crate::wave::shazam::{bandpass, extract_peaks, fingerprint, spectrogram};
use anyhow::anyhow;
use cpal::SupportedStreamConfig;
use crossterm::event::{KeyCode, KeyEventKind};
use dasp::ring_buffer;
use dasp_interpolate::sinc::Sinc;
use dasp_signal::Signal;
use ratatui::symbols::Marker;
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Stylize},
    text::Line,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType},
};
use rustfft::{FftPlanner, num_complex::Complex};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

pub enum Event {
    Input(crossterm::event::KeyEvent),
    Audio(Vec<f32>),
    Config(SupportedStreamConfig),
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct WaveApp {
    exit: bool,
    config: Option<SupportedStreamConfig>,
    db: Arc<Mutex<rusqlite::Connection>>,
    raw_data: Vec<f32>,
    record: bool,
    recorded_data: Arc<Mutex<Vec<f32>>>,
    downsample_rate: f64,
}

impl WaveApp {
    pub fn new(path: &str, downsample_rate: f64) -> Result<WaveApp, anyhow::Error> {
        let db = Arc::new(Mutex::new(rusqlite::Connection::open(path)?));
        Ok(Self {
            exit: false,
            config: None,
            db: db,
            raw_data: vec![],
            record: false,
            recorded_data: Arc::new(Mutex::new(vec![])),
            downsample_rate: downsample_rate,
        })
    }

    fn draw(&self, frame: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(frame.area());

        let instructions = Line::from(vec![
            " Record ".into(),
            "<R>".blue().bold(),
            " Clear Recorded Data ".into(),
            "<C>".blue().bold(),
            " Search ".into(),
            "<S>".blue().bold(),
            " Quit ".into(),
            "<Q> ".blue().bold(),
        ])
        .centered();

        let time_data = self.time_series();
        let time_data_len = time_data.len();
        let x_axis = Axis::default()
            .title("time".yellow())
            .bounds([0.0, time_data_len as f64])
            .labels(["0%", "50%", "100%"]);

        let y_axis = Axis::default()
            .title("Amplitude".yellow())
            .bounds([-0.5, 0.5])
            .labels(["-0.5", "0.0", "0.5"]);

        let mut time_dataset = Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Color::Green)
            .data(&time_data);

        if self.record {
            time_dataset = Dataset::default()
                .name("🔴 Recording")
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Color::Green)
                .data(&time_data);
        }

        let time = Chart::new(vec![time_dataset])
            .x_axis(x_axis)
            .y_axis(y_axis)
            .block(Block::default().title("Waveform").borders(Borders::ALL));

        let freq_data = self.fft_series();
        let freq_data_len = freq_data.len();
        let x_axis = Axis::default()
            .title("Frequency (Hz)".yellow())
            .bounds([0.0, freq_data_len as f64 / 2.0])
            .labels(["0%", "50%", "100%"]);

        let y_axis = Axis::default()
            .title("Magnitude".yellow())
            .bounds([0.0, 10.0])
            .labels(["0", "5.0", "10.0"]);

        let freq_dataset = Dataset::default()
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
        self.raw_data
            .iter()
            .enumerate()
            .map(|(i, &y)| (i as f64, y as f64))
            .collect::<Vec<_>>()
    }

    fn fft_series(&self) -> Vec<(f64, f64)> {
        let samples = self.raw_data.len();
        let mut buffer: Vec<Complex<f32>> = self
            .raw_data
            .iter()
            .map(|&x| Complex::new(x as f32, 0.0))
            .collect();

        let mut planner = FftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(samples);
        fft.process(&mut buffer);
        buffer
            .iter()
            .enumerate()
            .map(|(i, &y)| (i as f64, y.re.abs() as f64))
            .collect()
    }

    fn record_data(&mut self) -> Result<(), anyhow::Error> {
        {
            let Ok(mut data) = self.recorded_data.lock() else {
                return Err(anyhow!("Failed to acquire recorded data mutex"));
            };

            data.extend(self.raw_data.clone());
        }
        Ok(())
    }

    fn clear_recorded(&mut self) -> Result<(), anyhow::Error> {
        {
            let Ok(mut data) = self.recorded_data.lock() else {
                return Err(anyhow!("Failed to acquire recorded data mutex"));
            };

            data.clear();
        }
        Ok(())
    }

    fn downsample(&self, tx: mpsc::Sender<Vec<f32>>) -> Result<(), anyhow::Error> {
        let recorded_data_clone = Arc::clone(&self.recorded_data);
        let downsample_rate = self.downsample_rate.clone();
        let Some(cfg) = self.config.clone() else {
            tracing::error!("Failed to unwrap cpal device config");
            return Err(anyhow!("Failed to unwrap cpal device config"));
        };

        thread::spawn(move || {
            let Ok(recorded_data) = recorded_data_clone.lock() else {
                return Err(anyhow!("Failed to acquire recorded data mutex"));
            };

            let source = dasp_signal::from_iter(recorded_data.iter().map(|&x| x as f64));
            let scale = downsample_rate / cfg.sample_rate() as f64;
            let rbuf = ring_buffer::Fixed::from(vec![0.0; 70]);
            let sinc = Sinc::new(rbuf);
            let num_samples = (scale * recorded_data.len() as f64).round() as usize;
            let signal = source
                .scale_hz(sinc, scale)
                .take(num_samples)
                .map(|x| x as f32)
                .collect::<Vec<_>>();

            tx.send(signal)?;
            Ok(())
        });

        Ok(())
    }

    fn search(&self) -> Result<(), anyhow::Error> {
        let downsample_rate = self.downsample_rate.clone();
        let (tx, rx) = mpsc::channel();
        self.downsample(tx)?;

        thread::spawn(move || -> Result<(), anyhow::Error> {
            let mut signal = rx.recv()?;
            bandpass(&mut signal, downsample_rate, 20.0, 20000.0, 1.0);
            let spectrogram = spectrogram(&signal, downsample_rate)?;
            let peaks = extract_peaks(&spectrogram)?;
            let fingerprints = fingerprint(&peaks, 1.0, 1500.0, 8)?;
            tracing::info!("duration: {:?}", spectrogram.duration());
            tracing::info!("num fingerprints: {:?}", fingerprints.len());
            tracing::info!("fingerprints: {:?}", fingerprints);
            Ok(())
        });

        Ok(())
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
                Event::Config(config) => self.handle_config_event(config)?,
            }

            terminal.draw(|frame| self.draw(frame))?;

            if self.record {
                self.record_data()?;
            }
        }

        Ok(())
    }

    fn handle_config_event(&mut self, config: SupportedStreamConfig) -> Result<(), anyhow::Error> {
        self.config = Some(config);
        Ok(())
    }

    fn handle_audio_event(&mut self, data: Vec<f32>) -> Result<(), anyhow::Error> {
        self.raw_data = data;
        Ok(())
    }

    fn handle_key_event(
        &mut self,
        key_event: crossterm::event::KeyEvent,
    ) -> Result<(), anyhow::Error> {
        if key_event.kind == KeyEventKind::Press && key_event.code == KeyCode::Char('q') {
            self.exit = true;
        }

        if key_event.kind == KeyEventKind::Press
            && key_event.code == KeyCode::Char('r')
            && !self.record
        {
            self.record = true;
        } else if key_event.kind == KeyEventKind::Press
            && key_event.code == KeyCode::Char('r')
            && self.record
        {
            self.record = false;
        }

        if key_event.kind == KeyEventKind::Press && key_event.code == KeyCode::Char('c') {
            self.clear_recorded()?;
        }

        if key_event.kind == KeyEventKind::Press && key_event.code == KeyCode::Char('s') {
            self.search()?;
        }

        Ok(())
    }
}
