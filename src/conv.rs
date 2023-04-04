// Copyright 2023 Strixpyrr
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Conversion streams to and from other formats, converting raw PCM16-LE samples via the
//! Symphonia crate.

use std::io;
use std::cmp::min;
use errors::{Error as SymError, Error::ResetRequired};
use symphonia::core::audio::{AudioBuffer, AudioBufferRef, Channels, Signal};
use symphonia::core::codecs::{CodecType, Decoder, decl_codec_type, CodecParameters};
use symphonia::core::errors;
use symphonia::core::formats::{FormatReader, Track};
use symphonia::core::units::Time;
use crate::{DescriptorError, Pcm16Source, Result, StreamDescriptor};

/// Quite OK Audio
pub const CODEC_TYPE_QOA: CodecType = decl_codec_type(b"qoaf");

/// A [`Pcm16Source`] implementation reading samples from a Symphonia format stream.
pub struct FormatSource {
	track: Track,
	demuxer: Box<dyn FormatReader>,
	decoder: Box<dyn Decoder>,
	buffer: Option<AudioBuffer<i16>>,
}

impl FormatSource {
	pub fn new(track: Track, demuxer: Box<dyn FormatReader>, decoder: Box<dyn Decoder>) -> Self {
		Self {
			track,
			demuxer,
			decoder,
			buffer: None,
		}
	}

	fn read(&mut self) -> Result<bool, SymError> {
		let Self { track, demuxer, decoder, buffer } = self;

		while let Some(packet) = {
			match demuxer.next_packet() {
				Err(
					errors::Error::IoError(error)
				) =>
					if let io::ErrorKind::UnexpectedEof = error.kind() {
						None
					} else {
						return Err(error.into())
					},
				packet => Some(packet?)
			}
		} {
			if packet.track_id() == track.id {
				fn convert_or_deref(buf: AudioBufferRef) -> AudioBuffer<i16> {
					if let AudioBufferRef::S16(inner) = buf {
						inner.into_owned()
					} else {
						let mut cpy = buf.make_equivalent();
						buf.convert(&mut cpy);
						cpy
					}
				}

				let _ = buffer.insert(
					match decoder.decode(&packet).map(convert_or_deref) {
						Err(ResetRequired) => {
							decoder.reset();
							decoder.decode(&packet).map(convert_or_deref)
						}
						result => result
					}?
				);

				return Ok(true)
			}
		}

		Ok(false)
	}
}

impl Pcm16Source for FormatSource {
	type Error = SymError;

	fn read_interleaved(&mut self, mut buf: &mut [i16]) -> Result<usize, Self::Error> {
		let mut samples = 0;
		while let Some(mut sample_buf) = self.read()?.then(|| self.buffer.take()).flatten() {
			let channels = sample_buf.spec().channels.count();
			let frames = sample_buf.frames();
			let slices = min(frames, buf.len() / channels);

			for chn in 0..channels {
				let slices = sample_buf.chan(chn)[..slices]
					.iter()
					.enumerate();
				for (i, slice) in slices {
					buf[i * chn] = *slice;
				}
			}

			buf = &mut buf[slices * channels..];
			sample_buf.truncate(frames - slices);
			samples += slices;

			if frames > 0 {
				let _ = self.buffer.insert(sample_buf);
			}

			if buf.is_empty() { break }
		}

		Ok(samples)
	}

	fn exhausted(&mut self) -> Result<bool, Self::Error> {
		Ok(self.buffer.is_some() || self.read()?)
	}

	fn descriptor(&self) -> Result<StreamDescriptor, DescriptorError> {
		self.buffer
			.as_ref()
			.map_or_else(
				|| (&self.track.codec_params).try_into(),
				TryInto::try_into
			)
	}
}

const MAX_SAMPLES: usize = u32::MAX as usize;

impl TryFrom<&CodecParameters> for StreamDescriptor {
	type Error = DescriptorError;

	fn try_from(value: &CodecParameters) -> Result<Self, Self::Error> {
		let CodecParameters { sample_rate, channels, n_frames, time_base, .. } = value.clone();

		let channels = channels.map(Channels::count);

		let samples = (|| {
			let Time { seconds, frac } = time_base?.calc_time(n_frames?);
			let rate = sample_rate?;

			Some(rate as u64 * seconds + (rate as f64 * frac).ceil() as u64)
		})();

		if let Some(channels @ 256..) = channels {
			return Err(DescriptorError::TooManyChannels(channels))
		}

		if let Some(samples @ MAX_SAMPLES..) = samples.map(|s| s as usize) {
			return Err(DescriptorError::TooManySamples(samples))
		}

		Self::new(
			samples.map(|s| s as u32),
			sample_rate,
			channels.map(|c| c as u8),
			false,
		)
	}
}

impl TryFrom<&AudioBuffer<i16>> for StreamDescriptor {
	type Error = DescriptorError;

	fn try_from(value: &AudioBuffer<i16>) -> Result<Self, Self::Error> {
		let samples = value.frames();
		let channels = value.spec().channels.count();
		let rate = value.spec().rate;

		if let channels @ 256.. = channels {
			return Err(DescriptorError::TooManyChannels(channels))
		}

		if let samples @ MAX_SAMPLES.. = samples {
			return Err(DescriptorError::TooManySamples(samples))
		}

		Self::new(
			Some(samples as u32),
			Some(rate),
			Some(channels as u8),
			false,
		)
	}
}
