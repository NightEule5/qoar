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

use std::cmp::min;
use std::io::Write;
use std::mem;
use crate::{DEQUANT_TABLE, div, Error, FRAME_LEN, Int, MAGIC, Pcm16Source, QoaLmsState, QUANT_TABLE, Result, SLICE_LEN, StreamDescriptor, WriteKind};

struct Frame {
	slice_width: usize,
	slice_count: u16,
	slice_index: u16,
	buffer: Vec<i16>,
}

impl Frame {
	fn new(channel_count: u8) -> Self {
		Self {
			slice_width: SLICE_LEN * channel_count as usize,
			slice_count: 0,
			slice_index: 0,
			buffer: Vec::new(),
		}
	}

	/// Returns `true` if the frame header has been written.
	fn start(&mut self, samples: usize, channels: usize) -> bool {
		if self.complete() {
			let samples = min(FRAME_LEN, samples);
			self.slice_width = SLICE_LEN * channels;
			self.slice_count = ((samples + SLICE_LEN - 1) / SLICE_LEN) as u16;
			self.slice_index = 0;
			self.buffer.reserve(self.slice_width);
			true
		} else {
			false
		}
	}

	/// Returns `true` if all slices have been written.
	fn complete(&self) -> bool { self.slice_index >= self.slice_count }

	fn reset(&mut self) {
		self.slice_count = 0;
		self.slice_index = 0;
	}
}

pub struct Encoder<S: Write> {
	desc: StreamDescriptor,
	sink: Option<S>,
	has_header: bool,
	lms_states: Vec<QoaLmsState>,
	frame: Frame,
}

impl<S: Write> Encoder<S> {
	pub fn new_fixed(sample_count: u32, sample_rate: u32, channel_count: u8, sink: S) -> Result<Self> {
		Ok(Self {
			desc: StreamDescriptor::new(
				Some(sample_count),
				Some(sample_rate),
				Some(channel_count),
				true
			).map_err(Error::InvalidDescriptor)?,
			sink: Some(sink),
			has_header: false,
			lms_states: vec![QoaLmsState::default(); channel_count as usize],
			frame: Frame::new(channel_count),
		})
	}

	pub fn new_streaming(sink: S) -> Self {
		Self {
			desc: StreamDescriptor::default(),
			sink: Some(sink),
			has_header: false,
			lms_states: Vec::new(),
			frame: Frame::new(0),
		}
	}

	/// Encodes samples from a [`Vec`].
	pub fn encode_vec(&mut self, source: &mut Vec<i16>, mut desc: StreamDescriptor) -> Result {
		let Self { desc: this_desc, has_header, lms_states, frame, .. } = self;
		desc.infer_from_vec(source, this_desc);

		if this_desc.is_streaming() {
			this_desc.sample_rate   = desc.sample_count;
			this_desc.channel_count = desc.channel_count;
		} else {
			if desc.sample_rate   != this_desc.sample_rate ||
			   desc.channel_count != this_desc.channel_count {
				return Err(Error::InvalidDescriptorChange)
			}
		}

		let (samples, rate, channels, interleaved) = desc.unwrap_all();

		if samples == 0 || rate == 0 || channels == 0 {
			return Ok(())
		}


		let mut samples = samples as usize;

		{
			let sink = self.sink.as_mut().ok_or(Error::Closed)?;

			if *has_header {
				sink.enc_file_header(this_desc.sample_count.unwrap_or_default())?;
				*has_header = true;
			}

			lms_states.resize(channels as usize, QoaLmsState::default());

			if interleaved {
				while let n @ 1.. = sink.enc_frame(source, samples, channels, rate, lms_states, frame)? {
					source.truncate(source.len().saturating_sub(n * channels as usize));
					samples = samples.saturating_sub(n);
				}
			} else {
				todo!("Non-interleaved samples are not yet supported.")
			}

			sink.flush().map_err(|err| Error::Write(WriteKind::Flush, err.into()))?
		}

		self.set_sample_count(samples as u32);
		Ok(())
	}

	/// Encodes samples from a [`Pcm16Source`].
	pub fn encode(&mut self, source: &mut impl Pcm16Source) -> Result {
		let mut desc = source.descriptor().map_err(Error::InvalidDescriptor)?;
		let Self { desc: this_desc, has_header, lms_states, frame, .. } = self;
		desc.infer(this_desc);

		if !this_desc.is_streaming() {
			if desc.sample_rate   != this_desc.sample_rate ||
			   desc.channel_count != this_desc.channel_count {
				return Err(Error::InvalidDescriptorChange)
			}
		}

		let (samples, rate, channels, _) = desc.unwrap_all();

		if samples == 0 || rate == 0 || channels == 0 {
			return Ok(())
		}

		let mut samples = samples as usize;

		{
			let sink = self.sink.as_mut().ok_or(Error::Closed)?;

			if *has_header {
				sink.enc_file_header(this_desc.sample_count.unwrap_or_default())?;
				*has_header = true;
			}

			lms_states.resize(channels as usize, QoaLmsState::default());

			frame.start(samples, channels as usize);

			while !source.exhausted().map_err(|err| Error::SampleRead(err.into()))? {
				let off = frame.buffer.len();
				frame.buffer.resize(frame.slice_width, 0);
				let ref mut buf = frame.buffer[off..frame.slice_width];
				let n = source.read_interleaved(buf)
							  .map_err(|err| Error::SampleRead(err.into()))?;
				frame.buffer.truncate(n);
				let n = sink.enc_frame(&[], samples, channels, rate, lms_states, frame)?;
				samples = samples.saturating_sub(n);

				if n == 0 {
					break
				}
			}

			sink.flush().map_err(|err| Error::Write(WriteKind::Flush, err.into()))?;
		}

		self.set_sample_count(samples as u32);
		Ok(())
	}

	/// Flushes buffered samples to the inner sink.
	pub fn flush(&mut self) -> Result<()> {
		self.encode_vec(&mut Vec::new(), StreamDescriptor::default())
	}

	/// Closes the encoder, returning the inner sink if not already closed.
	pub fn close(&mut self) -> Option<Result<S>> {
		match self.flush() {
			Err(Error::Closed) => return None,
			Err(err) => return Some(Err(err)),
			_ => { }
		}

		self.sink.take().map(Ok)
	}

	/// Sets the sample count after encoding, if in fixed mode.
	fn set_sample_count(&mut self, n: u32) {
		if let Some(ref mut samples) = self.desc.sample_count {
			*samples = n;
		}
	}
}

impl<S: Write> Drop for Encoder<S> {
	/// Closes the encoder.
	fn drop(&mut self) { let _ = self.close(); }
}

// Convenience trait for converting big-endian integers of different sizes.
trait Int: Sized {
	const SIZE: usize = mem::size_of::<Self>();
	fn into_bytes(self) -> [u8; Self::SIZE];
	fn from_bytes(bytes: [u8; Self::SIZE]) -> Self;
}

impl Int for u64 {
	fn into_bytes(self) -> [u8; 8] { self.to_be_bytes() }
	fn from_bytes(bytes: [u8; 8]) -> Self { u64::from_be_bytes(bytes) }
}

impl Int for u32 {
	fn into_bytes(self) -> [u8; 4] { self.to_be_bytes() }
	fn from_bytes(bytes: [u8; 4]) -> Self { u32::from_be_bytes(bytes) }
}

impl Int for u16 {
	fn into_bytes(self) -> [u8; 2] { self.to_be_bytes() }
	fn from_bytes(bytes: [u8; 2]) -> Self { u16::from_be_bytes(bytes) }
}

trait QoaSink: Write {
	fn write_int<T: Int>(
		&mut self,
		value: T,
		kind: impl FnOnce() -> WriteKind
	) -> Result where [u8; T::SIZE]: {
		self.write_bytes(&value.into_bytes(), kind)
	}

	fn write_byte(&mut self, value: u8, kind: impl FnOnce() -> WriteKind) -> Result {
		self.write_bytes(&[value], kind)
	}

	fn write_bytes(&mut self, value: &[u8], kind: impl FnOnce() -> WriteKind) -> Result {
		self.write_all(value).map_err(|err| Error::Write(kind(), err))
	}

	fn enc_file_header(&mut self, sample_count: u32) -> Result {
		self.write_int(MAGIC,        || WriteKind::FileHeader("magic bytes"))?;
		self.write_int(sample_count, || WriteKind::FileHeader("sample count"))
	}

	fn enc_frame_header(&mut self, channel_count: u8, sample_rate: u32, sample_count: u16, slice_count: usize) -> Result {
		let size = 24 * channel_count as u16 + 8 * slice_count as u16 * channel_count as u16;
		self.write_byte(channel_count, || WriteKind::FrameHeader("channel count"))?;
		self.write_bytes(
			&sample_rate.into_bytes()[1..], // Clip to 24-bits
			|| WriteKind::FrameHeader("sample rate")
		)?;
		self.write_int(sample_count,   || WriteKind::FrameHeader("sample count"))?;
		self.write_int(size,           || WriteKind::FrameHeader("size"))
	}

	fn enc_lms_state(&mut self, value: &QoaLmsState) -> Result {
		let QoaLmsState { history, weights } = value;

		fn pack(acc: u64, cur: &i32) -> u64 {
			(acc << 16) | (*cur as u16 & 0xFFFF) as u64
		}

		let history = history.iter().fold(0, pack);
		let weights = weights.iter().fold(0, pack);
		self.write_int(history, || WriteKind::LmsState("history"))?;
		self.write_int(weights, || WriteKind::LmsState("weights"))
	}

	fn enc_slice(&mut self, samples: &[i16], channel_count: usize, lms: &mut [QoaLmsState]) -> Result {
		for chn in 0..channel_count {
			let len = SLICE_LEN.clamp(0, samples.len());
			let rng = 0..len * channel_count + chn;

			let mut best_error = -1;
			let mut best_slice = 0;
			let mut best_lms = QoaLmsState::default();

			for sf in 0..16 {
				let mut cur_lms = lms[chn];
				let mut slice = sf as u64;
				let mut cur_err = 0;

				for si in rng.clone().step_by(channel_count) {
					let sample = samples[si];
					let predicted = cur_lms.predict();
					let residual = sample as i32 - predicted;
					let scaled = div(residual, sf);
					let clamped = scaled.clamp(-8, 8);
					let quantized = QUANT_TABLE[(clamped + 8) as usize];
					let dequantized = DEQUANT_TABLE[sf][quantized as usize];
					let reconst = (predicted + dequantized).clamp(i16::MIN as i32, 32767) as i16;

					let error = sample as i64 - reconst as i64;
					cur_err += error * error;

					if cur_err > best_error {
						break;
					}

					cur_lms.update(reconst, dequantized);
					slice = slice << 3 | quantized as u64;
				}

				if cur_err < best_error {
					best_error = cur_err;
					best_slice = slice;
					best_lms = cur_lms;
				}
			}

			lms[chn] = best_lms;
			best_slice <<= (SLICE_LEN - len) * 3;
			self.write_int(best_slice, || WriteKind::SliceData(chn as u8))?;
		}

		Ok(())
	}

	fn enc_frame(
		&mut self,
		sample_buf: &[i16],
		sample_cnt: usize,
		channels: u8,
		rate: u32,
		lms: &mut [QoaLmsState],
		frame: &mut Frame,
	) -> Result<usize> {
		if sample_buf.is_empty() && frame.buffer.is_empty() {
			return Ok(0)
		}

		let mut consumed = 0;
		let mut off = 0;
		let mut len = sample_buf.len();

		if frame.start(sample_cnt, channels as usize) {
			self.enc_frame_header(channels, rate, sample_cnt as u16, frame.slice_count as usize)?;

			for lms in lms.iter() { self.enc_lms_state(lms)? }
		}

		{
			let Frame { slice_width, slice_count, slice_index, buffer, .. } = frame;

			len = min(len, *slice_width * *slice_count as usize);

			if !buffer.is_empty() || len <= *slice_width {
				off = min((*slice_width).saturating_sub(buffer.len()), len);
				consumed = SLICE_LEN;
				buffer.extend_from_slice(&sample_buf[..off]);

				if buffer.len() <= *slice_width {
					self.enc_slice(&buffer, channels as usize, lms)?;
					*slice_index += 1;
					buffer.clear();
				}
			}
		}

		let slices = sample_buf[off..len].chunks_exact(frame.slice_width);
		let excess = slices.remainder();
		for slice in slices {
			if frame.complete() { break }

			self.enc_slice(slice, channels as usize, lms)?;
			frame.slice_index += 1;
			consumed += SLICE_LEN;
		}

		frame.buffer.extend_from_slice(excess);

		if frame.complete() {
			frame.reset();
		}

		Ok(consumed)
	}
}

impl<W: Write> QoaSink for W { }
