use crate::Event;
use anyhow::anyhow;
use cpal::Sample;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

pub fn handle_input(tx: mpsc::Sender<Event>) -> Result<(), anyhow::Error> {
    loop {
        match crossterm::event::read()? {
            crossterm::event::Event::Key(key_event) => tx.send(Event::Input(key_event))?,
            _ => {}
        }
    }
}

pub fn get_audio(tx: mpsc::Sender<Event>, period_ms: u64) -> Result<(), anyhow::Error> {
    let host = cpal::default_host();
    let Some(device) = host.default_input_device() else {
        return Err(anyhow!("Failed to unwrap default input device"));
    };

    let config = device.default_input_config()?;
    let config_clone1 = config.clone();
    let config_clone2 = config.clone();
    let recorded_samples_f32 = Arc::new(Mutex::new(vec![]));
    let recorded_samples_i16 = Arc::new(Mutex::new(vec![]));
    let recorded_samples_u16 = Arc::new(Mutex::new(vec![]));
    let recorded_samples_f32_clone = Arc::clone(&recorded_samples_f32);
    let recorded_samples_i16_clone = Arc::clone(&recorded_samples_i16);
    let recorded_samples_u16_clone = Arc::clone(&recorded_samples_u16);
    let err_fn = |err| tracing::error!("An error occurred on the input audio stream: {:?}", err);
    tx.send(Event::Config(config_clone1))?;
    tracing::info!("CPAL Config: {:?}", config);

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => {
            build_input_stream::<f32>(&device, &config.into(), recorded_samples_f32, err_fn)?
        }
        cpal::SampleFormat::I16 => {
            build_input_stream::<i16>(&device, &config.into(), recorded_samples_i16, err_fn)?
        }
        cpal::SampleFormat::U16 => {
            build_input_stream::<u16>(&device, &config.into(), recorded_samples_u16, err_fn)?
        }
        sample_format => panic!("Unsupported sample format: {:?}", sample_format),
    };

    loop {
        stream.play()?;
        thread::sleep(Duration::from_millis(period_ms));
        match config_clone2.sample_format() {
            cpal::SampleFormat::F32 => {
                let Ok(mut samples) = recorded_samples_f32_clone.lock() else {
                    return Err(anyhow!("Failed to acquire recorded samples f32 lock"));
                };

                tx.send(Event::Audio(samples.to_vec()))?;
                samples.clear();
            }
            cpal::SampleFormat::I16 => {
                let Ok(mut samples) = recorded_samples_i16_clone.lock() else {
                    return Err(anyhow!("Failed to acquire recorded samples i16 lock"));
                };

                let converted_data = samples.iter().map(|&x| x as f32).collect();
                tx.send(Event::Audio(converted_data))?;
                samples.clear();
            }
            cpal::SampleFormat::U16 => {
                let Ok(mut samples) = recorded_samples_u16_clone.lock() else {
                    return Err(anyhow!("Failed to acquire recorded samples u16 lock"));
                };

                let converted_data = samples.iter().map(|&x| x as f32).collect();
                tx.send(Event::Audio(converted_data))?;
                samples.clear();
            }
            sample_format => tracing::error!("Unsupported sample format: {:?}", sample_format),
        }
    }
}

fn build_input_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    recorded_samples: Arc<Mutex<Vec<T>>>,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, anyhow::Error>
where
    T: Sync + Send + Sample + cpal::FromSample<f32> + cpal::SizedSample + 'static,
{
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| match recorded_samples.lock() {
            Ok(mut samples) => samples.extend_from_slice(data),
            Err(e) => {
                tracing::error!("Failed to acquire samples lock: {:?}", e);
                return;
            }
        },
        err_fn,
        None,
    )?;

    Ok(stream)
}
