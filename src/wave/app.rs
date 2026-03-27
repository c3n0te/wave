use anyhow::anyhow;
use cpal::SupportedStreamConfig;
use crossterm::event::{KeyCode, KeyEventKind};
use dasp::ring_buffer;
use dasp_interpolate::sinc::Sinc;
use dasp_signal::Signal;
use fundsp::prelude::*;
use ndarray::s;
use non_empty_slice::non_empty_vec;
use ratatui::symbols::Marker;
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Stylize},
    text::Line,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType},
};
use rustfft::{FftPlanner, num_complex::Complex};
use spectrograms::{LinearHz, Power, Spectrogram, audio::*, nzu};
use std::collections::HashMap;
use std::sync::mpsc;

pub enum Event {
    Input(crossterm::event::KeyEvent),
    Audio(Vec<f32>),
    Config(SupportedStreamConfig),
}

#[derive(Debug, Clone, PartialEq, PartialOrd)]
struct Peak {
    time: f64,
    frequency: f64,
    value: f64,
}

impl Peak {
    pub fn new(x: f64, y: f64, val: f64) -> Peak {
        Self {
            time: x,
            frequency: y,
            value: val,
        }
    }
}

#[derive(Debug)]
pub struct WaveApp {
    exit: bool,
    config: Option<SupportedStreamConfig>,
    db: rusqlite::Connection,
    raw_data: Vec<f32>,
    record: bool,
    recorded_data: Vec<f32>,
    downsample_rate: f64,
}

impl WaveApp {
    pub fn new(path: &str, downsample_rate: f64) -> Result<WaveApp, anyhow::Error> {
        let db = rusqlite::Connection::open(path)?;
        Ok(Self {
            exit: false,
            config: None,
            db: db,
            raw_data: vec![],
            record: false,
            recorded_data: vec![],
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

        let time_dataset = Dataset::default()
            .marker(Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Color::Green)
            .data(&time_data);

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

    fn record_data(&mut self) {
        self.recorded_data.extend(self.raw_data.clone());
    }

    fn clear_recorded(&mut self) {
        self.recorded_data.clear();
    }

    fn downsample(&self) -> Result<Vec<f32>, anyhow::Error> {
        let Some(cfg) = self.config.clone() else {
            tracing::error!("Failed to unwrap cpal device config");
            return Err(anyhow!("Failed to unwrap cpal device config"));
        };

        let source = dasp_signal::from_iter(self.recorded_data.iter().map(|&x| x as f64));
        let scale = self.downsample_rate / cfg.sample_rate() as f64;
        let rbuf = ring_buffer::Fixed::from(vec![0.0; 70]);
        let sinc = Sinc::new(rbuf);
        let num_samples = (scale * self.recorded_data.len() as f64).round() as usize;
        let signal = source
            .scale_hz(sinc, scale)
            .take(num_samples)
            .map(|x| x as f32)
            .collect::<Vec<_>>();
        Ok(signal)
    }

    fn bandpass(&self, signal: &mut Vec<f32>, low_cutoff: f64, high_cutoff: f64, q_factor: f64) {
        let mut filter = highpass_hz(low_cutoff, q_factor) >> lowpass_hz(high_cutoff, q_factor);
        filter.set_sample_rate(self.downsample_rate);
        signal
            .iter_mut()
            .for_each(|sample| *sample = filter.filter_mono(*sample));
    }

    fn spectrogram(&self, signal: Vec<f32>) -> Result<Spectrogram<LinearHz, Power>, anyhow::Error> {
        let mut samples = non_empty_vec![0.0; nzu!(1)];
        for sample in signal {
            samples.push(sample as f64);
        }

        let stft = StftParams::new(nzu!(512), nzu!(256), WindowType::Hanning, true)?;
        let params = SpectrogramParams::new(stft, self.downsample_rate)?;
        let spec = LinearPowerSpectrogram::compute(&samples, &params, None)?;
        Ok(spec)
    }

    fn extract_peaks(
        &self,
        spec: &Spectrogram<LinearHz, Power>,
    ) -> Result<Vec<Peak>, anyhow::Error> {
        let mut freq_bins = vec![];
        let ordered_freq_map = spec
            .frequencies()
            .iter()
            .enumerate()
            .map(|(i, &x)| (i, x))
            .collect::<Vec<(_, _)>>();

        let map_len = ordered_freq_map.len();
        let (_, ff) = spec.frequency_range();
        let inc_freq = ff / 6.0;
        let mut last_freq = 0.0;
        let mut last_idx = 0;
        for (idx, freq) in ordered_freq_map {
            if (freq - last_freq) > inc_freq || idx == (map_len - 1) {
                freq_bins.push((last_idx, idx));
                last_idx = idx;
                last_freq = freq;
            }
        }

        let time_map = spec
            .times()
            .iter()
            .enumerate()
            .map(|(i, &x)| (i, x))
            .collect::<HashMap<_, _>>();

        let freq_map = spec
            .frequencies()
            .iter()
            .enumerate()
            .map(|(i, &x)| (i, x))
            .collect::<HashMap<_, _>>();

        let mut max_values: HashMap<(&usize, &usize), Vec<Peak>> =
            HashMap::with_capacity(freq_bins.len());

        let mut col_idx = 0;
        for col in spec.data().axis_iter(ndarray::Axis(1)) {
            let mut row_idx = 0;
            for (idx0, idxf) in &freq_bins {
                let mut max = 0.0;
                for (idx, &val) in col.slice(s![*idx0..*idxf]).iter().enumerate() {
                    if val > max {
                        max = val;
                        row_idx = *idx0 + idx;
                    }
                }

                let Some(&freq) = freq_map.get(&row_idx) else {
                    return Err(anyhow!("Failed to retrieve frequency row idx"));
                };

                let Some(&time) = time_map.get(&col_idx) else {
                    return Err(anyhow!("Failed to retrieve time col idx"));
                };

                let pk = Peak::new(time, freq, max);
                if let Some(maxs) = max_values.get_mut(&(idx0, idxf)) {
                    maxs.push(pk);
                } else {
                    max_values.insert((idx0, idxf), vec![pk]);
                }
            }
            col_idx += 1;
        }

        let mut peaks = vec![];
        for ((_, _), maxs) in max_values.iter_mut() {
            let sum = maxs.iter().map(|pk| pk.value).sum::<f64>();
            let maxs_len = maxs.len() as f64;
            let avg = sum / maxs_len;
            maxs.retain(|pk| pk.value > avg);
            peaks.extend_from_slice(maxs);
        }

        Ok(peaks)
    }

    fn search(&self) -> Result<(), anyhow::Error> {
        let mut signal = self.downsample()?;
        self.bandpass(&mut signal, 20.0, 20000.0, 1.0);
        let spectrogram = self.spectrogram(signal)?;
        let peaks = self.extract_peaks(&spectrogram)?;
        tracing::info!("Peaks: {:?}", peaks);
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
                self.record_data();
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
            self.clear_recorded();
        }

        if key_event.kind == KeyEventKind::Press && key_event.code == KeyCode::Char('s') {
            self.search()?;
        }

        Ok(())
    }
}
