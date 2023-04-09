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
use std::cmp::{max, min};
use errors::{Error as SymError, Error::ResetRequired};
use symphonia::core::audio::{AudioBuffer, AudioBufferRef, Channels, Signal};
use symphonia::core::codecs::{CodecType, Decoder as SymDecoder, decl_codec_type};
use symphonia::core::errors;
use symphonia::core::formats::{FormatReader, Track};
use crate::{PcmSink, PcmSource, PcmStream, Result};
use crate::pcm_io::Error;

/// Quite OK Audio
pub const CODEC_TYPE_QOA: CodecType = decl_codec_type(b"qoaf");

/// A [`Pcm16Source`] implementation reading samples from a Symphonia format stream.
pub struct FormatSource {
	track: Track,
	demuxer: Box<dyn FormatReader>,
	decoder: Box<dyn SymDecoder>,
	buffer: Option<AudioBuffer<i16>>,
	samples: usize,
}

impl FormatSource {
	pub fn new(track: Track, demuxer: Box<dyn FormatReader>, decoder: Box<dyn SymDecoder>) -> Self {
		let samples = track.codec_params
						   .n_frames
						   .unwrap_or_default() as usize;
		Self {
			track,
			demuxer,
			decoder,
			buffer: None,
			samples,
		}
	}

	fn read(&mut self) -> Result<Option<AudioBuffer<i16>>, SymError> {
		let Self { track, demuxer, decoder, buffer, .. } = self;

		if let Some(buf) = buffer.take() {
			return Ok(Some(buf))
		}

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

				let buf =
					match decoder.decode(&packet).map(convert_or_deref) {
						Err(ResetRequired) => {
							decoder.reset();
							decoder.decode(&packet).map(convert_or_deref)
						}
						result => result
					}?;
				self.samples = self.samples.saturating_sub(buf.frames());

				return Ok(Some(buf))
			}
		}

		Ok(None)
	}
}

impl PcmStream for FormatSource {
	fn channel_count(&mut self) -> u8 {
		self.track
			.codec_params
			.channels
			.map(Channels::count)
			.unwrap_or_default() as u8
	}

	fn sample_rate(&mut self) -> u32 {
		self.track
			.codec_params
			.sample_rate
			.unwrap_or_default()
	}
}

impl PcmSource for FormatSource {
	fn read(&mut self, sink: &mut impl PcmSink, mut sample_count: usize) -> Result<usize, Error> {
		let mut samples = 0;
		while let Some(mut buf) = self.read().map_err(|err| Error::Read(err.into()))? {
			if sample_count == 0 { break }

			let channels = buf.spec().channels.count();
			sink.set_descriptor(buf.spec().rate, channels as u8)?;

			let read = (0..min(channels, 255))
				.map(|chn| {
					let data = buf.chan(chn);
					let len = min(sample_count, data.len());
					sink.write(&data[..len], chn as u8)
				})
				.reduce(|max_read, read| Ok(max(max_read?, read?))) // Ew
				.unwrap()?;
			samples      += read;
			sample_count -= read;

			buf.truncate(buf.frames() - read);

			if buf.frames() > 0 {
				let _ = self.buffer.insert(buf);
			}
		}
		Ok(samples)
	}

	fn sample_count(&mut self, _: u8) -> usize { self.samples }
}
