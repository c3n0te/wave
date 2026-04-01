use crate::wave::shazam::{bandpass, downsample, extract_peaks, fingerprint, spectrogram};
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
    style::{Color, Style, Stylize},
    symbols,
    text::Line,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, List, ListItem, Tabs},
};
use rodio::{Decoder, Source};
use rusqlite::Connection;
use rustfft::{FftPlanner, num_complex::Complex};
use std::{
    fs::{File, read_dir},
    sync::{Arc, Mutex, mpsc},
    thread,
};

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
    search: bool,
    selected_tab: usize,
}

impl WaveApp {
    pub fn new(path: &str, downsample_rate: f64) -> Result<WaveApp, anyhow::Error> {
        let db = Arc::new(Mutex::new(Connection::open(path)?));
        Ok(Self {
            exit: false,
            config: None,
            db: db,
            raw_data: vec![],
            record: false,
            recorded_data: Arc::new(Mutex::new(vec![])),
            downsample_rate: downsample_rate,
            search: false,
            selected_tab: 0,
        })
    }

    pub fn migrate(&self) -> Result<(), anyhow::Error> {
        let db_clone = Arc::clone(&self.db);
        let fingerprints = r#"
            CREATE TABLE IF NOT EXISTS Fingerprints (
                song_id INTEGER,
                hash INTEGER,
                anchor_time FLOAT
            );
        "#;

        let songs = r#"
            CREATE TABLE IF NOT EXISTS Songs (
                song_id INTEGER PRIMARY KEY,
                title TEXT,
                artist TEXT
            );
        "#;

        let index = r#"
            CREATE INDEX IF NOT EXISTS hash_idx ON Fingerprints (hash);
        "#;

        let Ok(conn) = db_clone.lock() else {
            tracing::info!("Failed to acquire db mutex");
            return Err(anyhow!("Failed to acquire db mutex"));
        };

        let mut fing_stmt = conn.prepare(fingerprints)?;
        let mut song_stmt = conn.prepare(songs)?;
        fing_stmt.execute([])?;
        song_stmt.execute([])?;
        conn.execute(index, [])?;
        Ok(())
    }

    pub fn init_db(&self, data_path: String) -> Result<(), anyhow::Error> {
        let db_clone = Arc::clone(&self.db);

        thread::spawn(move || {
            {
                let Ok(conn) = db_clone.lock() else {
                    tracing::info!("Failed to acquire db mutex");
                    return Err(anyhow!("Failed to acquire db mutex"));
                };

                let finger_count: i64 =
                    conn.query_row("SELECT COUNT(*) FROM Fingerprints", [], |row| row.get(0))?;

                let song_count: i64 =
                    conn.query_row("SELECT COUNT(*) FROM Songs", [], |row| row.get(0))?;

                if finger_count > 0 || song_count > 0 {
                    return Ok(());
                }
            }

            let downsample_rate = 16000.0;
            let mut song_id = 0;
            let files = read_dir(data_path)?;
            for entry in files {
                let entry = entry?;
                let wav_file = File::open(entry.path())?;
                let decoder = Decoder::new_wav(wav_file)?;
                let sample_rate = decoder.sample_rate().get() as f64;
                let recording = decoder.record();
                let samples = recording.into_iter().collect::<Vec<f32>>();
                let mut signal = downsample(&samples, downsample_rate, sample_rate)?;
                bandpass(&mut signal, downsample_rate, 20.0, 20000.0, 1.0);
                let spectrogram = spectrogram(&signal, downsample_rate)?;
                let peaks = extract_peaks(&spectrogram)?;
                let fingerprints = fingerprint(&peaks, 1.0, 1500.0, 5)?;
                tracing::info!(
                    "file: {:?}\nnum fingerprints: {:?}",
                    entry.path(),
                    fingerprints.len(),
                );

                let fpath = entry.path();
                let Some(fname) = fpath.to_str() else {
                    return Err(anyhow!("Failed to convert file name to str"));
                };

                let tokens = fname.split(" by ").collect::<Vec<&str>>();
                let title = tokens[0].split("\\").collect::<Vec<&str>>();
                let Some(title) = title.get(3) else {
                    return Err(anyhow!("Failed to parse song title"));
                };

                let artist = tokens[1].split(".wav").collect::<Vec<&str>>();
                let Some(artist) = artist.get(0) else {
                    return Err(anyhow!("Failed to parse song artist"));
                };

                tracing::info!(
                    "song id: {:?}; title: {:?}; artist: {:?}",
                    song_id,
                    title,
                    artist
                );

                let Ok(mut conn) = db_clone.lock() else {
                    return Err(anyhow!("Failed to acquire db mutex"));
                };

                let tx = conn.transaction()?;
                tx.execute(
                    "INSERT INTO Songs (song_id, title, artist) VALUES (?1, ?2, ?3)",
                    rusqlite::params![song_id, title, artist],
                )?;
                tx.commit()?;

                let tx = conn.transaction()?;
                for (hash, anchor_time) in fingerprints {
                    tx.execute(
                        "INSERT INTO Fingerprints (song_id, hash, anchor_time) VALUES (?1, ?2, ?3)",
                        rusqlite::params![song_id, hash, anchor_time],
                    )?;
                }

                tx.commit()?;
                song_id += 1;
            }

            Ok(())
        });

        Ok(())
    }

    fn draw(&mut self, frame: &mut Frame) {
        if self.selected_tab == 0 {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(2),
                    Constraint::Percentage(49),
                    Constraint::Percentage(49),
                ])
                .split(frame.area());

            let instructions = Line::from(vec![
                " Change Tab ".into(),
                "<T>".blue().bold(),
                " Start/Stop Recording ".into(),
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

            let tabs = Tabs::new(vec!["Audio Visualizer", "Search Results"])
                .style(Color::White)
                .highlight_style(Style::default().green().on_black().bold())
                .select(self.selected_tab)
                .divider(symbols::DOT)
                .padding(" ", " ");

            frame.render_widget(tabs, chunks[0]);
            frame.render_widget(time, chunks[1]);
            frame.render_widget(freq, chunks[2]);
        } else if self.selected_tab == 1 {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(2), Constraint::Percentage(98)])
                .split(frame.area());

            let items = [
                ListItem::new("Song 1"),
                ListItem::new("Song 2"),
                ListItem::new("Song 3"),
            ];

            let list = List::new(items)
                .block(Block::bordered().title("Top Ranked Songs"))
                .style(Style::new().white())
                .highlight_style(Style::new().reversed());

            let tabs = Tabs::new(vec!["Audio Visualizer", "Search Results"])
                .style(Color::White)
                .highlight_style(Style::default().green().on_black().bold())
                .select(self.selected_tab)
                .divider(symbols::DOT)
                .padding(" ", " ");

            frame.render_widget(tabs, chunks[0]);
            frame.render_widget(list, chunks[1]);
        }
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
            let fingerprints = fingerprint(&peaks, 1.0, 1500.0, 5)?;
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

        if key_event.kind == KeyEventKind::Press
            && key_event.code == KeyCode::Char('t')
            && self.selected_tab == 0
        {
            self.selected_tab = 1;
        } else if key_event.kind == KeyEventKind::Press
            && key_event.code == KeyCode::Char('t')
            && self.selected_tab == 1
        {
            self.selected_tab = 0;
        }

        Ok(())
    }
}
