use crate::wave::peak::Peak;
use anyhow::anyhow;
use dasp::ring_buffer;
use dasp_interpolate::sinc::Sinc;
use dasp_signal::Signal;
use fundsp::prelude::*;
use ndarray::s;
use non_empty_slice::non_empty_vec;
use spectrograms::{LinearHz, Power, Spectrogram, audio::*, nzu};
use std::collections::HashMap;

pub fn downsample(
    signal: &[f32],
    downsample_rate: f64,
    sample_rate: f64,
) -> Result<Vec<f32>, anyhow::Error> {
    let source = dasp_signal::from_iter(signal.iter().map(|&x| x as f64));
    let scale = downsample_rate / sample_rate;
    let rbuf = ring_buffer::Fixed::from(vec![0.0; 70]);
    let sinc = Sinc::new(rbuf);
    let num_samples = (scale * signal.len() as f64).round() as usize;
    let downsampled_signal = source
        .scale_hz(sinc, scale)
        .take(num_samples)
        .map(|x| x as f32)
        .collect::<Vec<_>>();
    Ok(downsampled_signal)
}

pub fn bandpass(
    signal: &mut [f32],
    downsample_rate: f64,
    low_cutoff: f64,
    high_cutoff: f64,
    q_factor: f64,
) {
    let mut filter = highpass_hz(low_cutoff, q_factor) >> lowpass_hz(high_cutoff, q_factor);
    filter.set_sample_rate(downsample_rate);
    signal
        .iter_mut()
        .for_each(|sample| *sample = filter.filter_mono(*sample));
}

pub fn spectrogram(
    signal: &[f32],
    downsample_rate: f64,
) -> Result<Spectrogram<LinearHz, Power>, anyhow::Error> {
    let mut samples = non_empty_vec![0.0; nzu!(1)];
    for sample in signal {
        samples.push(*sample as f64);
    }

    let stft = StftParams::new(nzu!(512), nzu!(256), WindowType::Hanning, true)?;
    let params = SpectrogramParams::new(stft, downsample_rate)?;
    let spec = LinearPowerSpectrogram::compute(&samples, &params, None)?;
    Ok(spec)
}

pub fn extract_peaks(spec: &Spectrogram<LinearHz, Power>) -> Result<Vec<Peak>, anyhow::Error> {
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
        let sum = maxs.iter().map(|pk| pk.amplitude()).sum::<f64>();
        let maxs_len = maxs.len() as f64;
        let avg = sum / maxs_len;
        maxs.retain(|pk| pk.amplitude() > avg);
        peaks.extend_from_slice(maxs);
    }

    peaks.sort_by(|a, b| a.time().total_cmp(&b.time()));
    Ok(peaks)
}

pub fn fingerprint(
    peaks: &[Peak],
    time_zone: f64,
    freq_zone: f64,
    min_targets: usize,
) -> Result<HashMap<u32, f64>, anyhow::Error> {
    let mut fingerprints = HashMap::new();
    for (idx, &anchor) in peaks.iter().enumerate() {
        let mut targets = vec![];
        for j in 0..peaks.len() {
            if j == idx {
                continue;
            }

            let Some(&target) = peaks.get(j) else {
                return Err(anyhow!("Failed to unwrap peak option"));
            };

            if (anchor.time() - target.time()).abs() > time_zone {
                continue;
            }

            if (anchor.frequency() - target.frequency()).abs() > freq_zone {
                continue;
            }

            targets.push((anchor.distance(target), target));
        }

        targets.sort_by(|a, b| a.0.total_cmp(&b.0));
        let k_targets = std::cmp::min(min_targets, targets.len());
        let target_slice = &targets[0..k_targets];
        for (_, target) in target_slice {
            let mut anchor_bits = (anchor.frequency() / 10.0).round() as u32;
            let mut target_bits = (target.frequency() / 10.0).round() as u32;
            let mut dt_bits = ((anchor.time() - target.time()) * 1000.0).abs().round() as u32;
            anchor_bits = anchor_bits & ((1 << 10) - 1);
            target_bits = target_bits & ((1 << 10) - 1);
            dt_bits = dt_bits & ((1 << 12) - 1);
            let hash = (anchor_bits << 22) | (target_bits << 12) | dt_bits;
            fingerprints.insert(hash, anchor.time());
        }
    }

    Ok(fingerprints)
}
