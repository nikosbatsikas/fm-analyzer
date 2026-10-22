//! Sound-card output through cpal (WASAPI on Windows, ALSA on Linux, CoreAudio on macOS).
//! Pulls interleaved stereo from the shared [`AudioOut`] FIFO.

use fm_analyzer::audio::AudioOut;
use std::sync::Arc;

#[cfg(feature = "audio")]
mod imp {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{FromSample, Sample, SizedSample};

    pub struct Sound {
        _stream: cpal::Stream,
        pub device_name: String,
        pub rate: u32,
    }

    fn build<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        audio: Arc<AudioOut>,
    ) -> Result<cpal::Stream, String>
    where
        T: SizedSample + FromSample<f32>,
    {
        let channels = config.channels as usize;
        let mut tmp: Vec<f32> = Vec::new();
        device
            .build_output_stream(
                config,
                move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                    tmp.resize(data.len(), 0.0);
                    audio.pull(&mut tmp, channels);
                    for (d, s) in data.iter_mut().zip(tmp.iter()) {
                        *d = T::from_sample(*s);
                    }
                },
                |e| eprintln!("audio stream error: {e}"),
                None,
            )
            .map_err(|e| e.to_string())
    }

    pub fn open(audio: Arc<AudioOut>) -> Result<Sound, String> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "no audio output device".to_string())?;
        let device_name = device.name().unwrap_or_else(|_| "default".into());
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let rate = config.sample_rate.0;

        let stream = match format {
            cpal::SampleFormat::F32 => build::<f32>(&device, &config, audio.clone()),
            cpal::SampleFormat::I16 => build::<i16>(&device, &config, audio.clone()),
            cpal::SampleFormat::U16 => build::<u16>(&device, &config, audio.clone()),
            cpal::SampleFormat::I32 => build::<i32>(&device, &config, audio.clone()),
            other => Err(format!("unsupported sample format {other:?}")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        audio.set_rate(rate);
        Ok(Sound { _stream: stream, device_name, rate })
    }
}

#[cfg(not(feature = "audio"))]
mod imp {
    use super::*;

    pub struct Sound {
        pub device_name: String,
        pub rate: u32,
    }

    pub fn open(_audio: Arc<AudioOut>) -> Result<Sound, String> {
        Err("audio output not compiled in (feature \"audio\")".into())
    }
}

pub use imp::{open, Sound};
